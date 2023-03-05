use clap::Parser;
use ytdl_operator_types::ExecutorSpec;

mod ready;
mod error;

use error::Error;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
   #[arg(long, default_value_t = false)]
   download_video: bool,

   #[arg(long, default_value_t = false)]
   download_thumbnail: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let namespace: String = get_namespace()
        .expect("failed to get namespace from environment");
    let spec: ExecutorSpec = get_spec()
        .expect("failed to get executor spec from environment");
    println!("waiting for VPN to connect");
    ready::wait_for_vpn().await.expect("vpn failed to connect");
    println!("VPN is connected");
    match (args.download_video, args.download_thumbnail) {
        (true, true) => {
            println!("downloading video and thumbnail");
            let result = tokio::join!(
                download_video(&namespace, &spec),
                download_thumbnail(&namespace, &spec),
            );
            result.0.expect("failed to download video");
            result.1.expect("failed to download thumbnail");
        }
        (true, false) => {
            println!("downloading video");
            download_video(&namespace, &spec).await.expect("failed to download video");
        }
        (false, true) => {
            println!("downloading thumbnail");
            download_thumbnail(&namespace, &spec).await.expect("failed to download thumbnail");
        }
        (false, false) => {
            panic!("no download options specified");
        }
    }
}

fn get_spec() -> Result<ExecutorSpec, Error> {
    Ok(serde_json::from_str(&std::env::var("SPEC")?)?)
}

fn get_namespace() -> Result<String, Error> {
    Ok(std::env::var("SPEC")?)
}

async fn download_video(namespace: &str, spec: &ExecutorSpec) -> Result<(), Error> {
    Ok(())
}

async fn download_thumbnail(namespace: &str, spec: &ExecutorSpec) -> Result<(), Error> {
    Ok(())
}