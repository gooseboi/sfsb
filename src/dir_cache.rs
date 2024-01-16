use byte_unit::Byte;
use chrono::{DateTime, Utc};
use color_eyre::{
    eyre::{ContextCompat, WrapErr},
    Report, Result,
};

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

    pub fn is_file(&self) -> bool {
        use CacheEntry::*;
        match self {
            File(_) => true,
            Dir(_) => false,
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
