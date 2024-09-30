#![deny(
    clippy::enum_glob_use,
    clippy::pedantic,
    clippy::nursery,
    clippy::unwrap_used
)]

use camino::Utf8PathBuf;
use clap::Parser;
use color_eyre::Result;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use url::Url;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Config {
    #[arg(env = "SFSB_BASE_URL")]
    base_url: Url,

    #[arg(env = "SFSB_DATA_DIR")]
    data_dir: Utf8PathBuf,

    #[arg(env = "SFSB_PORT", default_value_t = 3779)]
    port: u16,

    #[arg(env = "SFSB_PORT", default_value_t = 0)]
    threads: usize,
}

impl From<Config> for sfsb::AppState {
    fn from(config: Config) -> Self {
        Self {
            base_url: config.base_url.into(),
            data_dir: config.data_dir.into(),
            port: config.port,
            cache: Arc::default(),
        }
    }
}

fn main() -> Result<()> {
    let config = Config::parse();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sfsb=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    color_eyre::install()?;

    let rt = match config.threads {
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

    info!(threads = config.threads, "Starting tokio runtime");
    rt.block_on(sfsb::run_app(config.into()))
}
