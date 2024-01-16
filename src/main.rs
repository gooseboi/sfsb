#![feature(iter_intersperse)]

use askama_axum::IntoResponse;
use axum::{
    body::Body,
    extract::{self, Query, State},
    http::{HeaderMap, Response, StatusCode},
    response::Redirect,
    routing::get,
    Router,
};
use color_eyre::{
    eyre::{bail, ensure, ContextCompat, WrapErr},
    Result,
};
use parking_lot::RwLock;
use std::{
    env,
    io::SeekFrom,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::io::{AsyncSeekExt, BufReader};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use url::Url;

mod dir;

use dir::{CacheEntry, FetchQuery};

#[derive(Clone)]
struct AppState {
    // TODO: Use these
    _admin_username: Arc<str>,
    _admin_password: Arc<str>,
    base_url: Arc<Url>,
    data_dir: Arc<Path>,
    cache: Arc<RwLock<Vec<CacheEntry>>>,
}

impl AppState {
    fn new() -> Result<Self> {
        let admin_username = env::var("SFSB_ADMIN_USERNAME").wrap_err("Could not get environment variable SFSB_ADMIN_USERNAME")?.into();
        // FIXME: Hash this?
        let admin_password = env::var("SFSB_ADMIN_PASSWORD").wrap_err("Could not get environment variable SFSB_ADMIN_PASSWORD")?.into();
        let base_url = env::var("SFSB_BASE_URL").wrap_err("Could not get environment variable SFSB_BASE_URL")?;
        let base_url = Arc::new(Url::parse(&base_url).wrap_err("Could not parse environment variable SFSB_BASE_URL into a url")?);
        let data_dir = env::var("SFSB_DATA_DIR").unwrap_or("./data".into());
        let data_dir = PathBuf::from(&data_dir).into();
        let cache = Arc::new(RwLock::new(vec![]));

        Ok(AppState {
            _admin_username: admin_username,
            _admin_password: admin_password,
            base_url,
            data_dir,
            cache,
        })
    }
}

fn generate_aria2(base_url: &Url, _fetch_dir: &Path, entries: &[CacheEntry]) -> String {
    let mut list = String::new();
    for entry in entries {
        // TODO: Directories
        if entry.is_file() {
            let mut entry_url = base_url.clone();
            entry_url.path_segments_mut().unwrap().push("dl").push(entry.name());
            let mut entry_str = String::new();
            entry_str.push_str(entry_url.as_str());
            entry_str.push('\n');
            entry_str.push('\t');
            entry_str.push_str("dir=.");
            entry_str.push('\n');
            entry_str.push('\t');
            entry_str.push_str(&format!("out={name}", name=entry.name()));
            entry_str.push('\n');
            entry_str.push('\n');
            list.push_str(&entry_str);
        }
    }
    list
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sfsb=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    color_eyre::install()?;

    let state = AppState::new().wrap_err("Could not get app config").unwrap();
    let data_dir = Arc::clone(&state.data_dir);
    let cache = Arc::clone(&state.cache);

    tokio::spawn(async move {
        loop {
            let entries = match data_dir.read_dir() {
                Ok(entries) => entries,
                Err(e) => {
                    info!("Failed to read contents of data dir {data_dir:?}: {e}");
                    continue;
                }
            };
            let entries: Result<Vec<CacheEntry>> = entries.map(|e| e?.try_into()).collect();
            let entries = match entries {
                Ok(entries) => entries,
                Err(e) => {
                    info!("Failed to parse contents of data dir {data_dir:?}: {e}");
                    continue;
                }
            };
            let mut lock = cache.write();
            lock.clear();
            lock.extend(entries);
            drop(lock);
            break;
        }
    });

    let app = Router::new()
        .route("/", get(|| async { Redirect::permanent("/browse/") }))
        .route("/browse", get(fetch_root))
        .route("/browse/", get(fetch_root))
        .route("/browse/*path", get(serve_path))
        .route("/dl/*path", get(dl_path))
        .with_state(state);
    let addr: SocketAddr = "0.0.0.0:3779".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn fetch_root(
    State(state): State<AppState>,
    Query(query): Query<FetchQuery>,
) -> impl IntoResponse {
    fetch_path(Path::new("."), state, query).await
}

async fn fetch_path(
    fetch_dir: &Path,
    state: AppState,
    query: FetchQuery,
) -> Result<Response<Body>, (StatusCode, String)> {
    let cache = Arc::clone(&state.cache);
    info!(
        "Displaying directory view for [{path}]",
        path = fetch_dir.to_string_lossy()
    );
    let fetch_dir = dir::make_good(fetch_dir)
        .wrap_err_with(|| format!("Failed making path {fetch_dir:?} goody"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let lock = cache.read();
    let entries = dir::get_path_from_cache(&fetch_dir, &lock)
        .wrap_err_with(|| format!("Failed fetching contents of path {fetch_dir:?}"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    drop(lock);
    let Some(entries) = entries else {
        return Ok(
            Redirect::permanent(&format!("/dl/{p}", p = fetch_dir.to_string_lossy()))
                .into_response(),
        );
    };
    if query.aria2() {
        // FIXME: Should this go in /dl instead of /browse?
        let base_url = &state.base_url;
        Response::builder()
            .header("Content-Type", "text/plain")
            .body(Body::new(generate_aria2(base_url, &fetch_dir, &entries)))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    } else {
        dir::DirectoryViewTemplate::new(&fetch_dir, entries, query)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
            .map(|template| template.into_response())
    }
}

async fn serve_path(
    extract::Path(path): extract::Path<PathBuf>,
    State(state): State<AppState>,
    Query(query): Query<FetchQuery>,
) -> impl IntoResponse {
    // FIXME: nicer errors?
    fetch_path(&path, state, query).await
}

fn content_type_from_extension(ext: Option<&str>) -> &str {
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Basics_of_HTTP/MIME_types/Common_types
    let Some(ext) = ext else {
        // FIXME: text/plain or application/octet-stream?
        return "text/plain";
    };
    match ext {
        ".aac" => "audio/aac",
        ".abw" => "application/x-abiword",
        ".apng" => "image/apng",
        ".arc" => "application/x-freearc",
        ".avif" => "image/avif",
        ".avi" => "video/x-msvideo",
        ".azw" => "application/vnd.amazon.ebook",
        ".bin" => "application/octet-stream",
        ".bmp" => "image/bmp",
        ".bz" => "application/x-bzip",
        ".bz2" => "application/x-bzip2",
        ".cda" => "application/x-cdf",
        ".csh" => "application/x-csh",
        ".css" => "text/css",
        ".csv" => "text/csv",
        ".doc" => "application/msword",
        ".docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".eot" => "application/vnd.ms-fontobject",
        ".epub" => "application/epub+zip",
        ".gz" => "application/gzip",
        ".gif" => "image/gif",
        ".htm" | ".html" => "text/html",
        ".ico" => "image/vnd.microsoft.icon",
        ".ics" => "text/calendar",
        ".jar" => "application/java-archive",
        ".jpeg" | ".jpg" => "image/jpeg",
        ".js" => "text/javascript",
        ".json" => "application/json",
        ".jsonld" => "application/ld+json",
        ".mid," => "audio/midi",
        ".mjs" => "text/javascript",
        ".mp3" => "audio/mpeg",
        ".mp4" => "video/mp4",
        ".mpeg" => "video/mpeg",
        ".mpkg" => "application/vnd.apple.installer+xml",
        ".odp" => "application/vnd.oasis.opendocument.presentation",
        ".ods" => "application/vnd.oasis.opendocument.spreadsheet",
        ".odt" => "application/vnd.oasis.opendocument.text",
        ".oga" => "audio/ogg",
        ".ogv" => "video/ogg",
        ".ogx" => "application/ogg",
        ".opus" => "audio/opus",
        ".otf" => "font/otf",
        ".png" => "image/png",
        ".pdf" => "application/pdf",
        ".php" => "application/x-httpd-php",
        ".ppt" => "application/vnd.ms-powerpoint",
        ".pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ".rar" => "application/vnd.rar",
        ".rtf" => "application/rtf",
        ".sh" => "application/x-sh",
        ".svg" => "image/svg+xml",
        ".tar" => "application/x-tar",
        ".tif" | ".tiff" => "image/tiff",
        ".ts" => "video/mp2t",
        ".ttf" => "font/ttf",
        ".txt" => "text/plain",
        ".vsd" => "application/vnd.visio",
        ".wav" => "audio/wav",
        ".weba" => "audio/webm",
        ".webm" => "video/webm",
        ".webp" => "image/webp",
        ".woff" => "font/woff",
        ".woff2" => "font/woff2",
        ".xhtml" => "application/xhtml+xml",
        ".xls" => "application/vnd.ms-excel",
        ".xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ".xml" => "application/xml",
        ".xul" => "application/vnd.mozilla.xul+xml",
        ".zip" => "application/zip",
        ".3gp" => "video/3gpp",
        ".3g2" => "video/3gpp2",
        ".7z" => "application/x-7z-compressed",
        // FIXME: Same as above
        _ => "text/plain",
    }
}

fn parse_ranges(range: &str) -> Result<Vec<(Option<u64>, Option<u64>)>> {
    let mut vals = range.split('=');
    ensure!(
        vals.next().context("Missing range unit")? == "bytes",
        "Range unit was not bytes"
    );
    let range_str = vals.next().context("Missing range values")?;
    let mut res = vec![];
    for range in range_str.split(',') {
        let mut it = range.split('-');
        let v1 = it.next().wrap_err("Missing any value for range")?;
        let v1 = if v1.is_empty() {
            None
        } else {
            Some(
                v1.parse::<u64>()
                    .wrap_err_with(|| format!("Invalid value for range: {v1}"))?,
            )
        };

        let v2 = it.next().wrap_err("Missing `-` in range")?;
        let v2 = if v2.is_empty() {
            None
        } else {
            Some(
                v2.parse::<u64>()
                    .wrap_err_with(|| format!("Invalid value for range: {v2}"))?,
            )
        };
        if it.next().is_some() {
            bail!("Excess range in {range}");
        }
        res.push((v1, v2));
    }
    Ok(res)
}

async fn dl_path(
    extract::Path(path): extract::Path<PathBuf>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    info!("Downloading [{p}]", p = path.to_string_lossy());
    let data_dir = &state.data_dir;
    let file_path = {
        let mut p = data_dir.to_path_buf();
        p.push(&path);
        p
    };

    let metadata = tokio::fs::metadata(&file_path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if metadata.is_dir() {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("TODO: Cannot download folders yet: requested {path:?}"),
        ));
    }
    let file_len = metadata.len();
    let ext = path.extension().map(|s| s.to_str().unwrap());
    let content_type = content_type_from_extension(ext);
    if headers.contains_key("Range") {
        let ranges = headers
            .get("Range")
            .unwrap()
            .to_str()
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let ranges = parse_ranges(ranges).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        info!("Requested ranges: {ranges:?}");
        if ranges.len() != 1 {
            return Err((
                StatusCode::RANGE_NOT_SATISFIABLE,
                "Do not support multiple ranges in Range request".to_string(),
            ));
        }
        if ranges.is_empty() {
            return Err((StatusCode::RANGE_NOT_SATISFIABLE, "You shouldn't send a range request without an actual range. That's bad for the environment".to_string()));
        }
        let start = ranges[0]
            .0
            .context("Range without starting range not supported")
            .map_err(|e| (StatusCode::RANGE_NOT_SATISFIABLE, e.to_string()))?;

        let mut file = tokio::fs::File::open(file_path)
            .await
            .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
        if start > file_len {
            return Err((
                StatusCode::RANGE_NOT_SATISFIABLE,
                "The range start was past the end of the file".to_string(),
            ));
        }
        file.seek(SeekFrom::Current(start.try_into().unwrap()))
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to seek file: {e}"),
                )
            })?;
        let buffered_file = BufReader::new(file);
        let stream = tokio_util::io::ReaderStream::new(buffered_file);
        let stream = axum::body::Body::from_stream(stream);
        let sent_len = file_len - start;
        let end = file_len - 1;
        Response::builder()
            .status(206)
            .header("Content-Range", format!("bytes {start}-{end}/{file_len}"))
            .header("Content-Length", sent_len)
            .header("Content-Type", content_type)
            .header(
                "Content-Disposition",
                format!("attachment; filename=\"{p}\"", p = path.to_string_lossy()),
            )
            .body(stream)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    } else {
        let file = tokio::fs::File::open(file_path)
            .await
            .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
        let stream = tokio_util::io::ReaderStream::new(file);
        let stream = axum::body::Body::from_stream(stream);

        Response::builder()
            .header("Accept-Ranges", "bytes")
            .header("Content-Length", file_len)
            .header("Content-Type", content_type)
            .header(
                "Content-Disposition",
                format!("attachment; filename=\"{p}\"", p = path.to_string_lossy()),
            )
            .body(stream)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    }
}
