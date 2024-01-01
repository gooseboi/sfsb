#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(index));

}
