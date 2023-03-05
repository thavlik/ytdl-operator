mod reconcile_video;
mod video;

use reconcile_video::reconcile_video_main;

#[tokio::main]
async fn main() {
    reconcile_video_main().await
}
