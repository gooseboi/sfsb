use byte_unit::Byte;
use chrono::{DateTime, Utc};
use color_eyre::{
    eyre::{bail, ContextCompat, WrapErr},
    Report, Result,
};
use parking_lot::RwLock;
use std::{
    path::{Path, PathBuf, Component},
    sync::Arc,
};
use tracing::info;

use askama::Template;

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
        self.children.iter().count()
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
    entries: Vec<CacheEntry>,
}

/// Make the path be in the format we want
fn make_good(path: &Path) -> Result<PathBuf> {
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

fn get_path_from_cache(path: &Path, v: &[CacheEntry]) -> Result<Vec<CacheEntry>> {
    info!("Getting path from cache: {path:?}");
    if path == Path::new("") {
        return Ok(v.to_vec());
    }

    let mut components = path.components();
    let Some(component) = components.next() else {
        return Ok(vec![]);
    };

    let Component::Normal(s) = component else {
        bail!("Found component of type not normal in path {path:?}");
    };

    let Some(c) = v.iter().find(|c| c.is_dir() && *c.as_dir().name == *s) else {
        return Ok(vec![]);
    };

    info!("{}", c.as_dir().name);
    return get_path_from_cache(components.as_path(), &c.as_dir().children);
}

impl DirectoryViewTemplate {
    pub fn new(data_dir: &Path, cache: Arc<RwLock<Vec<CacheEntry>>>) -> Result<Self> {
        let data_dir = make_good(data_dir)
            .wrap_err_with(|| format!("Failed making path {data_dir:?} goody"))?;

        let parent = if data_dir == Path::new(".") {
            None
        } else {
            data_dir
                .parent()
                .map(|p| p.to_str().wrap_err("Parent dir was not UTF-8"))
                .transpose()?
                .map(|p| p.to_owned())
        };

        let dirname = match data_dir.as_os_str().to_string_lossy().as_ref() {
            "." => String::new(),
            s => s.to_owned(),
        };

        let lock = cache.read();
        let entries = get_path_from_cache(&data_dir, &lock).wrap_err_with(|| format!("Failed getting path for {dirname}"))?;
        drop(lock);
        Ok(Self {
            parent,
            dirname,
            entries,
        })
    }
}
