use askama::filters::urlencode;
use axum::response::IntoResponse;
use axum::{
    body::Body,
    extract::{self, Query, State},
    http::{Response, StatusCode},
    response::Redirect,
};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, ensure, WrapErr},
    Result,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info};
use url::Url;

use askama::Template;

use crate::{dir_cache::CacheEntry, utils::cmp_ignore_case_utf8, AppState};

#[derive(Deserialize, Debug)]
pub struct FetchQuery {
    #[serde(rename = "ord")]
    #[serde(default)]
    sort_direction: SortDirection,
    #[serde(rename = "sort")]
    #[serde(default)]
    sort_key: SortKey,
    aria2: Option<String>,
}

impl FetchQuery {
    pub const fn aria2(&self) -> bool {
        self.aria2.is_some()
    }
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

#[derive(Template)]
#[template(path = "dir_view.html")]
pub struct DirectoryViewTemplate {
    /// String pointing to parent directory of current directory, used to traverse up
    parent_directory: Option<String>,
    /// List of dirnames with anchor tags used to browse up in the view
    /// For directory "Some dir/dir1"
    /// "<a href="/browse/Some%20dir">Some dir</a> / <a href="/browse/Some%20dir/dir1">dir1</a>"
    list_of_anchors: String,
    /// Name of the current directory being browsed
    display_dirname: String,
    /// Directory name urlencoded
    encoded_dirname: String,
    /// List of every entry in the current directory
    entries: Vec<CacheEntry>,
    /// Direction to sort by
    sort_direction: SortDirection,
    /// What value to sort by
    sort_key: SortKey,
}

pub fn normalise_path(path: &Utf8Path) -> Result<Utf8PathBuf> {
    ensure!(
        path.is_relative(),
        "Path fetched must be relative, got absolute path {path:?}"
    );

    if path == Utf8Path::new(".") {
        return Ok(Utf8PathBuf::new());
    }

    ensure!(
        path.components().all(|c| c != Utf8Component::ParentDir),
        "Path cannot have `..`, nice try... (Got path {path:?})"
    );

    let mut components = path.components();
    if matches!(components.next(), Some(Utf8Component::CurDir)) {
        Ok(components.collect())
    } else {
        Ok(path.to_path_buf())
    }
}

pub fn path_contents_from_cache(
    path: &Utf8Path,
    v: &[CacheEntry],
) -> Result<Option<Vec<CacheEntry>>> {
    if path == Utf8Path::new("") {
        return Ok(Some(v.to_vec()));
    }

    let mut components = path.components();
    let Some(component) = components.next() else {
        bail!("No component in path despite path not being empty");
    };

    let Utf8Component::Normal(s) = component else {
        bail!("Found component of type not normal in path {path:?}");
    };

    let Some(c) = v.iter().find(|c| c.is_dir() && *c.as_dir().name == *s) else {
        return Ok(None);
    };

    return path_contents_from_cache(components.as_path(), &c.as_dir().children);
}

impl DirectoryViewTemplate {
    pub fn new(data_dir: &Utf8Path, mut entries: Vec<CacheEntry>, query: FetchQuery) -> Self {
        let parent_directory = if data_dir == Utf8Path::new(".") {
            None
        } else {
            data_dir.parent().map(std::string::ToString::to_string)
        };

        let mut dirname = data_dir.as_os_str().to_string_lossy().as_ref().to_owned();
        if !dirname.is_empty() && !dirname.ends_with('/') {
            dirname.push('/');
        }
        let dirname = dirname;

        let encoded_dirname = urlencode(&dirname).expect("TODO: Handle dirnames not urlencodable");

        let list_of_anchors = match dirname.as_str() {
            "." => String::new(),
            s => {
                let mut accumulated = String::new();
                let mut anchor_tags = vec![];
                for dirname in s.split('/').filter(|s| !s.is_empty()) {
                    let encoded_dirname =
                        urlencode(dirname).expect("TODO: Handle invalid url charaters in filename");
                    let current_relative_dirname = format!("{accumulated}/{encoded_dirname}");
                    // The / is added by the above line
                    let anchor_tag = format!(
                        "<a href=\"/browse{current_relative_dirname}\"><strong>{dirname}</strong></a>"
                    );
                    accumulated = current_relative_dirname.clone();
                    anchor_tags.push(anchor_tag);
                }
                anchor_tags.into_iter().intersperse(" / ".into()).collect()
            }
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

        Self {
            parent_directory,
            list_of_anchors,
            // FIXME: Display the directory properly in the title
            display_dirname: dirname,
            encoded_dirname,
            entries,
            sort_direction: query.sort_direction,
            sort_key: query.sort_key,
        }
    }
}

pub fn generate_aria2(base_url: &Url, entries: &[CacheEntry]) -> String {
    fn generate_aria2_helper(
        base_url: &Url,
        fetch_dir: &Utf8Path,
        entries: &[CacheEntry],
    ) -> String {
        let mut file_list = String::new();
        let mut subdir_list = String::new();
        let aria2_dir = if fetch_dir == Utf8Path::new("") {
            ".".to_string()
        } else {
            fetch_dir.as_str().trim_end_matches('/').to_string()
        };
        let mut entries = entries.to_vec();
        entries.sort_by(|e1, e2| cmp_ignore_case_utf8(e1.name(), e2.name()));
        for entry in entries {
            if entry.is_file() {
                let mut entry_url = base_url.clone();
                {
                    let mut path_segments = entry_url
                        .path_segments_mut()
                        .expect("Base url provided is a base");
                    path_segments.push("dl");

                    fetch_dir.components().for_each(|c| {
                        path_segments.push(c.as_ref());
                    });

                    path_segments.push(entry.name());
                }
                let mut entry_str = String::new();
                entry_str.push_str(entry_url.as_str());
                entry_str.push('\n');
                entry_str.push_str(&' '.to_string().repeat(2));
                entry_str.push_str(&format!("dir={aria2_dir}"));
                entry_str.push('\n');
                entry_str.push_str(&' '.to_string().repeat(2));
                entry_str.push_str(&format!("out={name}", name = entry.name()));
                entry_str.push('\n');
                entry_str.push('\n');
                file_list.push_str(&entry_str);
            } else if entry.is_dir() {
                let entry_path = {
                    let mut fetch_dir = fetch_dir.to_path_buf();
                    fetch_dir.push(entry.name());
                    fetch_dir
                };
                subdir_list.push_str(&generate_aria2_helper(
                    base_url,
                    &entry_path,
                    &entry.as_dir().children,
                ));
            }
        }
        file_list.push_str(&subdir_list);
        file_list
    }
    generate_aria2_helper(base_url, "".into(), entries)
}

pub async fn root_directory_view(
    State(state): State<AppState>,
    Query(query): Query<FetchQuery>,
) -> impl IntoResponse {
    view_for_path(Utf8Path::new("."), &state, query)
}

pub async fn serve_path_view(
    extract::Path(path): extract::Path<PathBuf>,
    State(state): State<AppState>,
    Query(query): Query<FetchQuery>,
) -> Result<Response<Body>, (StatusCode, String)> {
    // FIXME: nicer errors?
    let path = Utf8PathBuf::from_path_buf(path)
        .map_err(|p| (StatusCode::BAD_REQUEST, format!("Path {p:?} was not UTF-8")))?;
    view_for_path(&path, &state, query)
}

pub fn view_for_path(
    path_for_view: &Utf8Path,
    state: &AppState,
    query: FetchQuery,
) -> Result<Response<Body>, (StatusCode, String)> {
    let cache = Arc::clone(&state.cache);

    info!(
        path = ?path_for_view,
        "Displaying directory view"
    );

    debug!(fetch_query = ?query);

    let normalised_path = normalise_path(path_for_view)
        .wrap_err_with(|| format!("Failed making path {path_for_view:?} goody"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let lock = cache.read();
    let max_depth = lock
        .iter()
        .filter(|c| c.is_dir())
        .map(|d| d.as_dir().max_depth())
        .max()
        .unwrap_or(0);
    // Allow displaying the dir view for an empty directory, as empty
    if path_for_view.components().count() > max_depth + 1 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Path had more components than maximum depth of data".to_string(),
        ));
    }
    drop(lock);

    let lock = cache.read();
    let path_entries = path_contents_from_cache(&normalised_path, &lock)
        .wrap_err_with(|| format!("Failed fetching contents of path {normalised_path:?}"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    drop(lock);

    // If we have no dir entries, user tried to browse a file
    let Some(dir_entries) = path_entries else {
        return Ok(Redirect::permanent(&format!("/dl/{normalised_path}")).into_response());
    };

    if query.aria2() {
        // FIXME: Should this go in /dl instead of /browse?
        let base_url = &state.base_url;
        Response::builder()
            .header("Content-Type", "text/plain")
            .body(Body::new(generate_aria2(base_url, &dir_entries)))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    } else {
        Ok(DirectoryViewTemplate::new(&normalised_path, dir_entries, query).into_response())
    }
}
