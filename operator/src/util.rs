/// Friendly name for the controller.
pub const MANAGER_NAME: &str = "ytdl-operator";

pub fn get_concurrency() -> usize {
    match std::env::var("CONCURRENCY") {
        Ok(concurrency) => concurrency.parse().expect("failed to parse concurrency"),
        _ => 1,
    }
}