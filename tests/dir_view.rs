use camino::Utf8Path;
use reqwest::StatusCode;
use scraper::Html;
use std::net::{IpAddr, Ipv4Addr};
use tempfile::TempDir;
use url::Url;

async fn spawn_app_empty() -> (Url, TempDir) {
    let dir = tempfile::tempdir().expect("could not create tempdir for data");
    let data_dir = Utf8Path::from_path(dir.path())
        .expect("temp path was not UTF-8")
        .to_path_buf();
    let listener = tokio::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0))
        .await
        .expect("failed binding to port");
    let addr = listener.local_addr().expect("had local addr");
    let config = sfsb::AppConfig {
        base_url: Url::parse("http://localhost").expect("valid url"),
        data_dir,
        listener,
    };
    tokio::spawn(sfsb::run_app(config));
    let port = addr.port();
    (
        Url::parse(&format!("http://localhost:{port}")).expect("valid url"),
        dir,
    )
}

#[tokio::test]
async fn empty_view_produces_valid_html() {
    let (addr, _tempdir) = spawn_app_empty().await;

    let res = reqwest::get(addr).await.expect("no error with reqwest");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.bytes().await.expect("no error receiving html");
    let content: &[u8] = &bytes.slice(..);
    let content = std::str::from_utf8(content).expect("response html was not UTF-8");

    let parser = Html::parse_document(content);
    // Force fully parsing the file
    let _ = parser.html();
    assert_eq!(parser.errors, Vec::<&str>::new());
}
