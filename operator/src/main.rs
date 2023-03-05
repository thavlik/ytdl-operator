mod executor;

#[tokio::main]
async fn main() {
    executor::main().await
}
