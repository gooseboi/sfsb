use camino::Utf8Path;
use reqwest::StatusCode;
use scraper::Html;
use std::net::{IpAddr, Ipv4Addr};
use tempfile::TempDir;
use tokio::sync::oneshot;
use url::Url;

struct SpawnInfo {
    url: Url,
    dir: TempDir,
    shutdown: oneshot::Sender<()>,
}

impl Drop for SpawnInfo {
    fn drop(&mut self) {
        let (tx, _) = oneshot::channel();
        let old = std::mem::replace(&mut self.shutdown, tx);
        old.send(()).unwrap();
    }
}

fn start_tracing() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sfsb=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

async fn spawn_app_empty() -> SpawnInfo {
    if std::env::var("SFSB_TEST_TRACING").is_ok() {
        start_tracing();
    }

    let dir = tempfile::tempdir().expect("could not create tempdir for data");
    let data_dir = Utf8Path::from_path(dir.path())
        .expect("temp path was not UTF-8")
        .to_path_buf();
    let listener = tokio::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0))
        .await
        .expect("failed binding to port");
    let addr = listener.local_addr().expect("had local addr");
    let (tx, rx) = oneshot::channel();

    let config = sfsb::AppConfig {
        base_url: Url::parse("http://localhost").expect("valid url"),
        data_dir,
        listener,
        shutdown: Some(rx),
    };

    tokio::spawn(sfsb::run_app(config));
    let port = addr.port();

    SpawnInfo {
        url: Url::parse(&format!("http://localhost:{port}")).expect("valid url"),
        dir,
        shutdown: tx,
    }
}

// Every test of the app needs to be ran using the multi threaded runtime, because otherwise this
// task has to yield to the scheduler for the scheduler to poll the shutdown task, on the event of
// a shutdown, which would involve manually adding a sleep, which I think is jankier and more
// cumbersome. Though the test can run on a single thread, so we just have a single worker thread.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn empty_view_produces_valid_html() {
    let SpawnInfo {
        ref url,
        dir: ref _tempdir,
        shutdown: _,
    } = spawn_app_empty().await;

    let res = reqwest::get(url.clone())
        .await
        .expect("no error with reqwest");
    assert_eq!(res.status(), StatusCode::OK);

    let bytes = res.bytes().await.expect("no error receiving html");
    let content: &[u8] = &bytes.slice(..);
    let content = std::str::from_utf8(content).expect("response html was not UTF-8");

    let parser = Html::parse_document(content);
    // Force fully parsing the file
    let _ = parser.html();
    assert_eq!(parser.errors, Vec::<&str>::new());
}
