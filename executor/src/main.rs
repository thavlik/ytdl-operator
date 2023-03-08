use clap::{Parser, Subcommand};
use kube::client::Client;
use std::env;
use ytdl_common::Error;

mod download;
mod query;
pub mod ready;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Query,

    Download {
        #[arg(long, default_value_t = false)]
        download_video: bool,

        #[arg(long, default_value_t = false)]
        download_thumbnail: bool,
    },
}

/// Returns the precise youtube-dl command to use,
/// which may be overriden to use e.g. yt-dlp, a
/// popular fork of youtube-dl that is often patched
/// faster than the main project.
fn get_command() -> String {
    env::var("YOUTUBE_DL_COMMAND").unwrap_or_else(|_| "yt-dlp".to_owned())
}

#[tokio::main]
async fn main() {
    let client: Client = Client::try_default()
        .await
        .expect("Expected a valid KUBECONFIG environment variable.");
    // Get the youtube-dl command to use from the spec.
    let command = get_command();
    // Parse command line options.
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Query) => {
            query::query(client, &command).await.unwrap();
        }
        Some(Command::Download {
            download_video,
            download_thumbnail,
        }) => {
            download::download(client, &command, download_video, download_thumbnail).await;
        }
        None => {
            println!("No command specified");
        }
    }
}
