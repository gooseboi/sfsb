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

use crate::{dir_cache::CacheEntry, AppState};

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
    parent: Option<String>,
    display_dirname: String,
    dirname: String,
    entries: Vec<CacheEntry>,
    sort_direction: SortDirection,
    sort_key: SortKey,
}

pub fn validate_path_and_make_relative(path: &Utf8Path) -> Result<Utf8PathBuf> {
    ensure!(
        path.is_relative(),
        "Path fetched must be relative, got absolute path {path:?}"
    );

    if path == Utf8Path::new(".") {
        return Ok(Utf8PathBuf::new());
    }

    let components = path.components();
    ensure!(
        components.clone().all(|c| c != Utf8Component::ParentDir),
        "Path cannot have `..`, nice try... (Got path {path:?})"
    );

    let components_vec = components.collect::<Vec<_>>();
    if components_vec[0] == Utf8Component::CurDir {
        return Ok(components_vec[1..].iter().collect());
    }

    Ok(path.to_path_buf())
}

pub fn get_path_from_cache(path: &Utf8Path, v: &[CacheEntry]) -> Result<Option<Vec<CacheEntry>>> {
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

    return get_path_from_cache(components.as_path(), &c.as_dir().children);
}

impl DirectoryViewTemplate {
    pub fn new(
        data_dir: &Utf8Path,
        mut entries: Vec<CacheEntry>,
        query: FetchQuery,
    ) -> Self {
        // FIXME: Encode file names
        let parent = if data_dir == Utf8Path::new(".") {
            None
        } else {
            data_dir.parent().map(std::string::ToString::to_string)
        };

        let dirname = {
            let mut dirname = data_dir.as_os_str().to_string_lossy().as_ref().to_owned();
            if !dirname.is_empty() && !dirname.ends_with('/') {
                dirname.push('/');
            }
            dirname
        };
        // TODO: add anchors to each directory here
        let display_dirname = match dirname.as_str() {
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
        Self {
            parent,
            display_dirname,
            // FIXME: Display the directory properly in the title
            dirname,
            entries,
            sort_direction: query.sort_direction,
            sort_key: query.sort_key,
        }
    }
}

pub fn generate_aria2(base_url: &Url, _fetch_dir: &Utf8Path, entries: &[CacheEntry]) -> String {
    let mut list = String::new();
    for entry in entries {
        // TODO: Directories
        if entry.is_file() {
            let mut entry_url = base_url.clone();
            entry_url
                .path_segments_mut()
                .expect("Base url provided is a base")
                .push("dl")
                .push(entry.name());
            let mut entry_str = String::new();
            entry_str.push_str(entry_url.as_str());
            entry_str.push('\n');
            entry_str.push('\t');
            entry_str.push_str("dir=.");
            entry_str.push('\n');
            entry_str.push('\t');
            entry_str.push_str(&format!("out={name}", name = entry.name()));
            entry_str.push('\n');
            entry_str.push('\n');
            list.push_str(&entry_str);
        }
    }
    list
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

    let validated_path_for_view = validate_path_and_make_relative(path_for_view)
        .wrap_err_with(|| format!("Failed making path {path_for_view:?} goody"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let lock = cache.read();
    let path_entries = get_path_from_cache(&validated_path_for_view, &lock)
        .wrap_err_with(|| format!("Failed fetching contents of path {validated_path_for_view:?}"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    drop(lock);

    // If we have no dir entries, user tried to browse a file
    let Some(dir_entries) = path_entries else {
        return Ok(Redirect::permanent(&format!("/dl/{validated_path_for_view}")).into_response());
    };

    if query.aria2() {
        // FIXME: Should this go in /dl instead of /browse?
        let base_url = &state.base_url;
        Response::builder()
            .header("Content-Type", "text/plain")
            .body(Body::new(generate_aria2(
                base_url,
                &validated_path_for_view,
                &dir_entries,
            )))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    } else {
        Ok(DirectoryViewTemplate::new(&validated_path_for_view, dir_entries, query).into_response())
    }
}
