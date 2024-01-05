use byte_unit::Byte;
use chrono::{DateTime, Utc};
use color_eyre::{
    eyre::{ContextCompat, WrapErr},
    Report, Result,
};
use std::{fs::DirEntry, path::Path};

use askama::Template;

/// Struct that represents a file/directory inside a directory, that can
/// access all its fields without erroring, because it errors upon construction
struct Entry {
    /// Name of the file
    name: String,
    /// Localtime this file was modified, in format
    created: String,
    /// Size of this file, if this is a file, already formatted
    /// Size of all children, if this is a directory
    size: String,
    /// Is a directory
    is_dir: bool,
    /// Children count, if was a directory
    children_count: Option<usize>,
}

impl TryFrom<DirEntry> for Entry {
    type Error = Report;

    fn try_from(value: DirEntry) -> Result<Self> {
        let name = value
            .file_name()
            .to_str()
            .with_context(|| format!("File name for {:?} was invalid unicode", value.file_name()))?
            .to_owned();
        let is_dir = value
            .file_type()
            .wrap_err_with(|| format!("Could not get filetype for {name}"))?
            .is_dir();

        let children: Option<Vec<Entry>> = if is_dir {
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
            Some(children)
        } else {
            None
        };
        let children_count = children.as_ref().map(|c| c.len());

        let meta = value
            .metadata()
            .wrap_err_with(|| format!("Failed to get metadata for {name}"))?;

        let created: DateTime<Utc> = meta
            .created()
            .wrap_err_with(|| format!("Failed to get creation time for {name}"))?
            .into();
        let created = created.format("%Y-%m-%d [%H:%M:%S]").to_string();

        let size = if is_dir {
                children
                    .unwrap()
                    .into_iter()
                    .map(|e| Byte::parse_str(e.size, true).unwrap().as_u64())
                    .sum()
        } else {
            meta.len()
        };
        let size = format!("{byte:#}", byte=Byte::from_u64(size));

        Ok(Entry {
            name,
            created,
            size,
            is_dir,
            children_count,
        })
    }
}

#[derive(Template)]
#[template(path = "dir_view.html")]
pub struct DirectoryViewTemplate {
    parent: String,
    dirname: String,
    entries: Vec<Entry>,
}

impl DirectoryViewTemplate {
    pub fn new(root_dir: &Path, data_dir: &Path) -> Result<Self> {
        let parent = data_dir
            .parent()
            .unwrap()
            .to_str()
            .wrap_err("Parent dir was not UTF-8")?
            .to_owned();
        let dirname = match data_dir.as_os_str().to_string_lossy().as_ref() {
            "." => String::new(),
            s => s.to_owned(),
        };

        let data_dir_name = data_dir.to_string_lossy().into_owned();
        let mut root_dir = root_dir.to_path_buf();
        root_dir.push(data_dir);
        let contents_iter = root_dir
            .read_dir()
            .wrap_err_with(|| format!("Failed to read data dir {data_dir_name}",))?;
        let mut entries = vec![];
        for entry in contents_iter {
            let entry = entry
                .wrap_err_with(|| format!("Failed to read file from directory {data_dir_name}"))?;
            entries.push(
                entry
                    .try_into()
                    .wrap_err("Failed to get information for file")?,
            );
        }
        Ok(Self {
            parent,
            dirname,
            entries,
        })
    }
}
