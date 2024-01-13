use byte_unit::Byte;
use chrono::{DateTime, Utc};
use color_eyre::{
    eyre::{bail, ContextCompat, WrapErr},
    Report, Result,
};
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};
use tracing::debug;

use askama::Template;

#[derive(Deserialize, Debug)]
pub struct FetchQuery {
    #[serde(rename = "ord")]
    #[serde(default)]
    sort_direction: SortDirection,
    #[serde(rename = "sort")]
    #[serde(default)]
    sort_key: SortKey,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SortDirection {
    #[serde(rename = "asc")]
    Ascending,
    #[serde(rename = "desc")]
    Descending,
}

impl Default for SortDirection {
    fn default() -> Self {
        Self::Ascending
    }
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SortKey {
    Name,
    Date,
    Size,
    ChildrenCount,
}

impl Default for SortKey {
    fn default() -> Self {
        Self::Name
    }
}

#[derive(Clone)]
pub enum CacheEntry {
    File(FileEntry),
    Dir(DirEntry),
}

impl CacheEntry {
    pub fn size(&self) -> u64 {
        use CacheEntry::*;
        match self {
            File(f) => f.size,
            Dir(d) => d.children.iter().map(|e| e.size()).sum(),
        }
    }

    pub fn size_str(&self) -> String {
        format!("{byte:#}", byte = Byte::from_u64(self.size()))
    }

    pub fn created(&self) -> &str {
        use CacheEntry::*;
        match self {
            File(f) => &f.created,
            Dir(d) => &d.created,
        }
    }

    pub fn name(&self) -> &str {
        use CacheEntry::*;
        match self {
            File(f) => &f.name,
            Dir(d) => &d.name,
        }
    }

    pub fn is_dir(&self) -> bool {
        use CacheEntry::*;
        match self {
            File(_) => false,
            Dir(_) => true,
        }
    }

    pub fn as_dir(&self) -> &DirEntry {
        let CacheEntry::Dir(entry) = self else {
            unreachable!()
        };
        entry
    }

    pub fn as_file(&self) -> &FileEntry {
        let CacheEntry::File(entry) = self else {
            unreachable!()
        };
        entry
    }
}

/// Struct that represents a file/directory inside a directory, that can
/// access all its fields without erroring, because it errors upon construction
#[derive(Clone)]
pub struct DirEntry {
    /// Name of the file
    pub name: String,
    /// UTC time this file was modified, in format `%Y-%m-%d [%H:%M:%S]`
    pub created: String,
    /// Children
    pub children: Vec<CacheEntry>,
}

impl DirEntry {
    pub fn children_count(&self) -> usize {
        self.children.len()
    }
}

/// Struct that represents a file/directory inside a directory, that can
/// access all its fields without erroring, because it errors upon construction
#[derive(Clone)]
pub struct FileEntry {
    /// Name of the file
    pub name: String,
    /// Localtime this file was modified, in format
    pub created: String,
    /// Size of this file, if this is a file, already formatted
    /// Size of all children, if this is a directory
    pub size: u64,
}

impl TryFrom<std::fs::DirEntry> for CacheEntry {
    type Error = Report;

    fn try_from(value: std::fs::DirEntry) -> Result<Self> {
        let name = value
            .file_name()
            .to_str()
            .with_context(|| format!("File name for {:?} was invalid unicode", value.file_name()))?
            .to_owned();

        let is_dir = value
            .file_type()
            .wrap_err_with(|| format!("Could not get filetype for {name}"))?
            .is_dir();

        let meta = value
            .metadata()
            .wrap_err_with(|| format!("Failed to get metadata for {name}"))?;

        let created: DateTime<Utc> = meta
            .created()
            .wrap_err_with(|| format!("Failed to get creation time for {name}"))?
            .into();
        let created = created.format("%Y-%m-%d [%H:%M:%S]").to_string();

        if is_dir {
            let children: Vec<CacheEntry> = {
                let entries = value
                    .path()
                    .read_dir()
                    .wrap_err_with(|| format!("Failed to read children for directory {name}"))?;
                let mut children = vec![];
                for e in entries {
                    let e = e?;
                    children.push(
                        e.try_into()
                            .wrap_err_with(|| format!("Failed to get child for {name}"))?,
                    );
                }
                children
            };
            Ok(CacheEntry::Dir(DirEntry {
                name,
                created,
                children,
            }))
        } else {
            let size = meta.len();
            Ok(CacheEntry::File(FileEntry {
                name,
                created,
                size,
            }))
        }
    }
}

#[derive(Template)]
#[template(path = "dir_view.html")]
pub struct DirectoryViewTemplate {
    parent: Option<String>,
    dirname: String,
    full_dirname: String,
    entries: Vec<CacheEntry>,
    sort_direction: SortDirection,
    sort_key: SortKey,
}

/// Make the path be in the format we want
pub fn make_good(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        bail!("Path cannot be absolute, they no worky, got path {path:?}")
    }

    if path == Path::new(".") {
        return Ok(PathBuf::new());
    }

    let components = path.components();
    if components.clone().any(|c| c == Component::ParentDir) {
        bail!("Path cannot have `..`, nice try...");
    }

    let components_vec = components.collect::<Vec<_>>();
    if components_vec[0] == Component::CurDir {
        return Ok(components_vec[1..].iter().collect());
    }

    Ok(path.to_path_buf())
}

pub fn get_path_from_cache(path: &Path, v: &[CacheEntry]) -> Result<Option<Vec<CacheEntry>>> {
    debug!("Getting path from cache: {path:?}");
    if path == Path::new("") {
        return Ok(Some(v.to_vec()));
    }

    let mut components = path.components();
    let Some(component) = components.next() else {
        bail!("No component in path despite path not being empty");
    };

    let Component::Normal(s) = component else {
        bail!("Found component of type not normal in path {path:?}");
    };

    let Some(c) = v.iter().find(|c| c.is_dir() && *c.as_dir().name == *s) else {
        return Ok(None);
    };

    debug!("Recursing into {}", c.as_dir().name);
    return get_path_from_cache(components.as_path(), &c.as_dir().children);
}

impl DirectoryViewTemplate {
    pub fn new(data_dir: &Path, mut entries: Vec<CacheEntry>, query: FetchQuery) -> Result<Self> {
        // FIXME: Encode file names
        let parent = if data_dir == Path::new(".") {
            None
        } else {
            data_dir
                .parent()
                .map(|p| p.to_str().wrap_err("Parent dir was not UTF-8"))
                .transpose()?
                .map(|p| p.to_owned())
        };

        let full_dirname = data_dir.as_os_str().to_string_lossy().as_ref().to_owned();
        // TODO: add anchors to each directory here
        let dirname = match full_dirname.as_str() {
            "." => String::new(),
            s => s.split('/').intersperse(" / ").collect(),
        };

        entries.sort_by(|e1, e2| {
            let ord = match query.sort_key {
                SortKey::Name => e1.name().cmp(e2.name()),
                SortKey::Date => match e1.created().cmp(e2.created()) {
                    std::cmp::Ordering::Equal => e1.name().cmp(e2.name()),
                    o => o,
                },
                SortKey::Size => match e1.size().cmp(&e2.size()) {
                    std::cmp::Ordering::Equal => e1.name().cmp(e2.name()),
                    o => o,
                },
                SortKey::ChildrenCount => {
                    let o = if e1.is_dir() && e2.is_dir() {
                        e1.as_dir()
                            .children_count()
                            .cmp(&e2.as_dir().children_count())
                    } else if e1.is_dir() && !e2.is_dir() {
                        std::cmp::Ordering::Greater
                    } else if !e1.is_dir() && e2.is_dir() {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Equal
                    };

                    match o {
                        std::cmp::Ordering::Equal => e1.name().cmp(e2.name()),
                        o => o,
                    }
                }
            };
            if query.sort_direction == SortDirection::Descending {
                ord.reverse()
            } else {
                ord
            }
        });
        Ok(Self {
            parent,
            dirname,
            // FIXME: Display the directory properly in the title
            full_dirname,
            entries,
            sort_direction: query.sort_direction,
            sort_key: query.sort_key,
        })
    }
}
