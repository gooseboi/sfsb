#![feature(iter_intersperse)]

use askama_axum::IntoResponse;
use axum::{
    extract::{self, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::get,
    Router,
};
use color_eyre::Result;
use parking_lot::RwLock;
use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod dir;

use dir::{CacheEntry, FetchQuery};

#[derive(Clone)]
struct AppState {
    admin_username: Arc<str>,
    admin_password: Arc<str>,
    data_dir: Arc<Path>,
    cache: Arc<RwLock<Vec<CacheEntry>>>,
}

impl AppState {
    fn new() -> Self {
        let admin_username = env::var("SFSB_ADMIN_USERNAME").unwrap().into();
        // FIXME: Hash this?
        let admin_password = env::var("SFSB_ADMIN_PASSWORD").unwrap().into();
        let data_dir = env::var("SFSB_DATA_DIR").unwrap_or("./data".into());
        let data_dir = PathBuf::from(&data_dir).into();
        let cache = Arc::new(RwLock::new(vec![]));

        AppState {
            admin_username,
            admin_password,
            data_dir,
            cache,
        }
    }
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

    let state = AppState::new();
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
    fetch_path(Path::new("."), Arc::clone(&state.cache), query).await
}

async fn fetch_path(
    fetch_dir: &Path,
    cache: Arc<RwLock<Vec<CacheEntry>>>,
    query: FetchQuery,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    info!("Fetching [{path}]", path = fetch_dir.to_string_lossy());
    dir::DirectoryViewTemplate::new(fetch_dir, cache, query)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn serve_path(
    extract::Path(path): extract::Path<PathBuf>,
    State(state): State<AppState>,
    Query(query): Query<FetchQuery>,
) -> impl IntoResponse {
    // FIXME: nicer errors?
    fetch_path(&path, Arc::clone(&state.cache), query).await
}
