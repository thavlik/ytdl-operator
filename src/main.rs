pub mod crd;

mod video;
mod reconcile_video;

use reconcile_video::reconcile_video_main;

#[tokio::main]
async fn main() {
    reconcile_video_main().await
}