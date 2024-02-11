#![feature(iter_intersperse)]
#![deny(
    clippy::enum_glob_use,
    clippy::pedantic,
    clippy::nursery,
    clippy::unwrap_used
)]

use axum::{response::Redirect, routing::get, Router};
use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{ensure, WrapErr},
    Result,
};
use parking_lot::RwLock;
use std::{env, net::SocketAddr, sync::Arc};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use url::Url;

mod utils;

mod dir_view;
use dir_view::{root_directory_view, serve_path_view};

mod dir_cache;
use dir_cache::CacheEntry;

mod download;
use download::dl_path;

#[derive(Clone)]
struct AppState {
    // TODO: Use these
    _admin_username: Arc<str>,
    _admin_password: Arc<str>,
    base_url: Arc<Url>,
    data_dir: Arc<Utf8Path>,
    // TODO: Use ArcSwap
    // TODO: Update this upon directory event
    cache: Arc<RwLock<Vec<CacheEntry>>>,
}

impl AppState {
    fn new() -> Result<Self> {
        const ADMIN_USERNAME_VAR: &str = "SFSB_ADMIN_USERNAME";
        const ADMIN_PASSWORD_VAR: &str = "SFSB_ADMIN_PASSWORD";
        const BASE_URL_VAR: &str = "SFSB_BASE_URL";
        const DATA_DIR_VAR: &str = "SFSB_DATA_DIR";

        let admin_username = env::var(ADMIN_USERNAME_VAR)
            .wrap_err_with(|| format!("Could not get environment variable {ADMIN_USERNAME_VAR}"))?
            .into();

        // FIXME: Hash this?
        let admin_password = env::var(ADMIN_PASSWORD_VAR)
            .wrap_err_with(|| format!("Could not get environment variable {ADMIN_PASSWORD_VAR}"))?
            .into();

        let base_url = env::var(BASE_URL_VAR)
            .wrap_err_with(|| format!("Could not get environment variable {BASE_URL_VAR}"))?;
        let base_url = Arc::new(Url::parse(&base_url).wrap_err_with(|| {
            format!("Could not parse environment variable {BASE_URL_VAR} into a url")
        })?);
        ensure!(
            !base_url.cannot_be_a_base(),
            "The server's base url must be a base"
        );

        let data_dir = env::var(DATA_DIR_VAR).unwrap_or_else(|_| "./data".into());
        let data_dir = Utf8PathBuf::from(&data_dir).into();

        Ok(Self {
            _admin_username: admin_username,
            _admin_password: admin_password,
            base_url,
            data_dir,
            cache: Arc::default(),
        })
    }
}

async fn inner_main(state: AppState) -> Result<()> {
    let data_dir = Arc::clone(&state.data_dir);
    let cache = Arc::clone(&state.cache);

    tokio::spawn(async move {
        loop {
            let entries = match data_dir.read_dir() {
                Ok(entries) => entries,
                Err(e) => {
                    error!("Failed to read contents of data dir {data_dir:?}: {e}");
                    continue;
                }
            };
            let entries: Result<Vec<CacheEntry>> = entries.map(|e| e?.try_into()).collect();
            let entries = match entries {
                Ok(entries) => entries,
                Err(e) => {
                    let errors = e
                        .chain()
                        .map(std::string::ToString::to_string)
                        .collect::<Vec<_>>();
                    error!("Failed to parse contents of data dir {data_dir:?}: {errors:?}");
                    continue;
                }
            };
            let mut lock = cache.write();
            *lock = entries;
            drop(lock);
            break;
        }
    });

    let app = Router::new()
        .route("/", get(|| async { Redirect::permanent("/browse/") }))
        .route("/browse", get(root_directory_view))
        .route("/browse/", get(root_directory_view))
        .route("/browse/*path", get(serve_path_view))
        .route("/dl/*path", get(dl_path))
        .with_state(state);
    let addr: SocketAddr = "0.0.0.0:3779".parse().expect("This is a valid address");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Server listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

fn main() -> Result<()> {
    const NUM_THREADS_VAR: &str = "SFSB_NUM_THREADS";

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sfsb=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    color_eyre::install()?;

    let state = AppState::new().expect("Could not get app config");

    let num_threads = env::var(NUM_THREADS_VAR)
        .wrap_err_with(|| format!("Could not get environment variable {NUM_THREADS_VAR}"))?
        .parse()
        .unwrap_or_else(|_| {
            panic!("Expected environment variable {NUM_THREADS_VAR} to be a number")
        });

    let rt = match num_threads {
        0 => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all();
            builder
        }
        1 => {
            let mut builder = tokio::runtime::Builder::new_current_thread();
            builder.enable_all();
            builder
        }
        n => {
            let mut builder = tokio::runtime::Builder::new_current_thread();
            builder.enable_all().worker_threads(n);
            builder
        }
    }
    .build()
    .expect("Failed building tokio runtime");

    info!(threads = num_threads, "Starting runtime");
    rt.block_on(inner_main(state))
}
