mod executor;

#[tokio::main]
async fn main() {
    println!("Initializing controller...");
    executor::main().await
}
