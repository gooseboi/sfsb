use askama::filters::urlencode;
use chrono::{DateTime, Utc};
use color_eyre::{
    eyre::{ContextCompat, WrapErr},
    Report, Result,
};

#[derive(Debug, Clone)]
pub enum CacheEntry {
    File(FileEntry),
    Dir(DirEntry),
}

impl CacheEntry {
    pub fn size(&self) -> u64 {
        match self {
            Self::File(f) => f.size,
            Self::Dir(d) => d.children.iter().map(Self::size).sum(),
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn size_str(&self) -> String {
        const BYTE_SIZE: u64 = 1024;

        let size = self.size();
        if size < BYTE_SIZE {
            format!("{size} B")
        } else if size < BYTE_SIZE.pow(2) {
            let size = (size as f64) / (BYTE_SIZE as f64).powi(1);
            format!("{size:.1} KiB")
        } else if size < BYTE_SIZE.pow(3) {
            let size = (size as f64) / (BYTE_SIZE as f64).powi(2);
            format!("{size:.1} MiB")
        } else if size < BYTE_SIZE.pow(4) {
            let size = (size as f64) / (BYTE_SIZE as f64).powi(3);
            format!("{size:.1} GiB")
        } else {
            "You really shouldn't be serving files that big with this tool...".to_owned()
        }
    }

    pub fn created(&self) -> &str {
        match self {
            Self::File(f) => &f.created,
            Self::Dir(d) => &d.created,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::File(f) => &f.name,
            Self::Dir(d) => &d.name,
        }
    }

    pub fn name_url_encoded(&self) -> String {
        urlencode(self.name()).expect("TODO: Handle invalid chars in name")
    }

    pub const fn is_dir(&self) -> bool {
        match self {
            Self::File(_) => false,
            Self::Dir(_) => true,
        }
    }

    pub const fn is_file(&self) -> bool {
        match self {
            Self::File(_) => true,
            Self::Dir(_) => false,
        }
    }

    pub fn as_dir(&self) -> &DirEntry {
        let Self::Dir(entry) = self else {
            unreachable!()
        };
        entry
    }

    pub fn as_file(&self) -> &FileEntry {
        let Self::File(entry) = self else {
            unreachable!()
        };
        entry
    }
}

/// Struct that represents a file/directory inside a directory, that can
/// access all its fields without erroring, because it errors upon construction
#[derive(Debug, Clone)]
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

    pub fn max_depth(&self) -> usize {
        self.children
            .iter()
            .filter_map(|c| {
                if let CacheEntry::Dir(d) = c {
                    Some(d)
                } else {
                    None
                }
            })
            .map(Self::max_depth)
            .max()
            .map_or(0, |d| d + 1)
    }
}

/// Struct that represents a file/directory inside a directory, that can
/// access all its fields without erroring, because it errors upon construction
#[derive(Debug, Clone)]
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
            let children: Vec<Self> = {
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
            Ok(Self::Dir(DirEntry {
                name,
                created,
                children,
            }))
        } else {
            let size = meta.len();
            Ok(Self::File(FileEntry {
                name,
                created,
                size,
            }))
        }
    }
}
