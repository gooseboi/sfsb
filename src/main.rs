#![deny(
    clippy::enum_glob_use,
    clippy::pedantic,
    clippy::nursery,
    clippy::unwrap_used
)]

use camino::Utf8PathBuf;
use clap::Parser;
use color_eyre::Result;
use std::net::{IpAddr, Ipv4Addr};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use url::Url;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct RawConfig {
    #[arg(env = "SFSB_BASE_URL")]
    base_url: Url,

    #[arg(env = "SFSB_DATA_DIR")]
    data_dir: Utf8PathBuf,

    #[arg(env = "SFSB_LISTEN_ADDRESS", default_value_t = IpAddr::V4(Ipv4Addr::new(0,0,0,0)))]
    listen_address: IpAddr,

    #[arg(env = "SFSB_PORT", default_value_t = 3779)]
    port: u16,

    #[arg(env = "SFSB_PORT", default_value_t = 0)]
    threads: usize,
}

impl RawConfig {
    fn convert(self, listener: tokio::net::TcpListener) -> sfsb::AppConfig {
        sfsb::AppConfig {
            listener,
            data_dir: self.data_dir,
            base_url: self.base_url,
            shutdown: None,
        }
    }
}

async fn startup(config: RawConfig) -> Result<()> {
    let listener = tokio::net::TcpListener::bind((config.listen_address, config.port)).await?;

    sfsb::run_app(config.convert(listener)).await?;

    Ok(())
}

fn main() -> Result<()> {
    let config = RawConfig::parse();

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
    rt.block_on(startup(config))
}
