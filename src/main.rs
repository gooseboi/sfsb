use askama_axum::IntoResponse;
use axum::{
    extract::{self, State},
    http::StatusCode,
    response::Redirect,
    routing::get,
    Router,
};
use color_eyre::Result;
use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod dir;

#[derive(Clone)]
struct AppState {
    admin_username: Arc<str>,
    admin_password: Arc<str>,
    data_dir: Arc<Path>,
}

impl AppState {
    fn new() -> Self {
        let admin_username = env::var("SFSB_ADMIN_USERNAME").unwrap().into();
        // FIXME: Hash this?
        let admin_password = env::var("SFSB_ADMIN_PASSWORD").unwrap().into();
        let data_dir = env::var("SFSB_DATA_DIR").unwrap_or("./data".into());
        let data_dir = PathBuf::from(&data_dir).into();

        AppState {
            admin_username,
            admin_password,
            data_dir,
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

    let app = Router::new()
        .route("/", get(|| async { Redirect::permanent("/browse/") }))
        .route("/browse", get(fetch_root))
        .route("/browse/", get(fetch_root))
        .route("/browse/*path", get(serve_path))
        .with_state(AppState::new());
    let addr: SocketAddr = "0.0.0.0:3779".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn fetch_root(State(state): State<AppState>) -> impl IntoResponse {
    fetch_path(&state.data_dir, Path::new(".")).await
}

async fn fetch_path(
    data_dir: &Path,
    fetch_dir: &Path,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    dir::DirectoryViewTemplate::new(data_dir, fetch_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn serve_path(
    extract::Path(path): extract::Path<PathBuf>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // FIXME: nicer errors?
    info!("Fetching [{path}]", path = path.to_string_lossy());
    if path.is_absolute() {
        Err((StatusCode::FORBIDDEN, "Absolute paths no worky".into()))
    } else {
        fetch_path(&state.data_dir, &path).await
    }
}
