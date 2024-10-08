#![feature(iter_intersperse)]

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{eyre::Context as _, Result};
use notify::{RecursiveMode, Watcher as _};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};
use url::Url;

mod dir_cache;
mod dir_view;
mod download;
mod utils;
use axum::{response::Redirect, routing::get, Router};
use dir_cache::CacheEntry;
use dir_view::{root_directory_view, serve_path_view};
use download::{dl_archive, dl_path};
use tokio::sync::oneshot;

pub struct AppConfig {
    pub base_url: Url,
    pub data_dir: Utf8PathBuf,
    pub listener: tokio::net::TcpListener,
    pub shutdown: Option<oneshot::Receiver<()>>,
}

#[derive(Clone)]
struct AppState {
    base_url: Arc<Url>,
    data_dir: Arc<Utf8Path>,
    cache: Arc<RwLock<Vec<CacheEntry>>>,
}

impl AppState {
    fn from_config(config: &AppConfig) -> Self {
        Self {
            base_url: config.base_url.clone().into(),
            data_dir: config.data_dir.clone().into(),
            cache: Arc::default(),
        }
    }
}

fn refresh_cache(cache: &RwLock<Vec<CacheEntry>>, data_dir: &Utf8Path) -> Result<()> {
    let entries = data_dir
        .read_dir()
        .wrap_err_with(|| format!("Failed to read contents of data dir {data_dir}"))?;
    let entries: Result<Vec<CacheEntry>> = entries.map(|e| e?.try_into()).collect();
    let entries =
        entries.wrap_err_with(|| format!("Failed to parse contents of data dir {data_dir}"))?;
    let empty = {
        let mut lock = cache.write();
        let empty = lock.is_empty();
        *lock = entries;
        empty
    };
    if empty {
        info!("Generated directory cache");
    } else {
        info!("Updated directory cache after fs event");
    }

    Ok(())
}

enum DataUpdateEvent {
    FsNotify(notify_debouncer_full::DebounceEventResult),
    Shutdown,
}

pub async fn run_app(config: AppConfig) -> Result<()> {
    let state = AppState::from_config(&config);

    let data_dir = Arc::clone(&state.data_dir);
    let cache = Arc::clone(&state.cache);

    let (data_update_tx, mut data_update_rx) = tokio::sync::mpsc::channel(2);

    refresh_cache(&cache, &data_dir).expect("Failed refreshing cache");
    let task_tx = data_update_tx.clone();
    tokio::task::spawn_blocking(move || {
        let data_dir = Arc::clone(&data_dir);

        let mut watcher =
            notify_debouncer_full::new_debouncer(Duration::from_secs(1), None, move |ev| {
                match task_tx.blocking_send(DataUpdateEvent::FsNotify(ev)) {
                    Ok(()) => {}
                    Err(e) => error!("Failed sending DataUpdateEvent after notify event: {e}"),
                }
            })
            .expect("Failed creating watcher for data dir");

        watcher
            .watcher()
            .watch(data_dir.as_std_path(), RecursiveMode::Recursive)
            .expect("Failed watching data dir");

        loop {
            match data_update_rx.blocking_recv() {
                // FIXME: Should this crash the program if the update fails?
                Some(DataUpdateEvent::FsNotify(_)) => {
                    info!("Refreshing data directory cache after event");
                    match refresh_cache(&cache, &data_dir) {
                        Ok(_) => {}
                        Err(e) => error!("Failed refreshing cache: {}", e),
                    }
                }
                Some(DataUpdateEvent::Shutdown) => {
                    warn!("Aborting data refresh task");
                    break;
                }
                None => {
                    error!("All data update senders have been dropped, quitting anyway");
                    break;
                }
            }
        }
    });

    let app = Router::new()
        .route("/", get(|| async { Redirect::permanent("/browse/") }))
        .route("/browse", get(root_directory_view))
        .route("/browse/", get(root_directory_view))
        .route("/browse/*path", get(serve_path_view))
        .route("/dl/*path", get(dl_path))
        .route("/arc/*path", get(dl_archive))
        .with_state(state);

    // Tokio doesn't follow this for some reason
    #[allow(clippy::redundant_pub_crate)]
    let quit_sig = async move {
        #[cfg(target_family = "unix")]
        let wait_for_stop = async move {
            use tokio::signal::unix;

            let mut term = unix::signal(unix::SignalKind::terminate())
                .expect("listening for signal shouldn't fail");
            let mut int = unix::signal(unix::SignalKind::interrupt())
                .expect("listening for signal shouldn't fail");

            tokio::select! {
                _ = int.recv() => { warn!("Received SIGINT, stopping") },
                _ = term.recv() => { warn!("Received SIGTERM, stopping") },
            };
        };

        #[cfg(target_family = "windows")]
        let wait_for_stop = async move {
            _ = tokio::signal::ctrl_c()
                .await
                .expect("listening for stop shouldn't fail");
        };

        if let Some(shutdown) = config.shutdown {
            tokio::select! {
                _ = wait_for_stop => { warn!("Received stop signal, shutting down") },
                _ = shutdown => { warn!("Received manual shutdown signal, shutting down") },
            }
        } else {
            wait_for_stop.await;
        }

        // If the above task finishes, then that means we received a termination/interrupt signal,
        // and should quit
        warn!("Initiating graceful shutdown...");

        data_update_tx
            .send(DataUpdateEvent::Shutdown)
            .await
            .expect("Failed sending data watch cancellation message");
        warn!("Starting abort for data refresh task");
    };

    info!("Server listening on {}", config.listener.local_addr()?);
    axum::serve(config.listener, app)
        .with_graceful_shutdown(quit_sig)
        .await?;

    Ok(())
}
