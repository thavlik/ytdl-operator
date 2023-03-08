use clap::{Parser, Subcommand};
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
    Query {
        #[arg(long)]
        input: String,

        #[arg(long, short = 'i', default_value_t = false)]
        ignore_errors: bool,
    },

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
    // Get the youtube-dl command to use from the spec.
    let command = get_command();
    // Parse command line options.
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Query { input, ignore_errors }) => {
            let result = query::query(&command, &input, ignore_errors)
                .await
                .unwrap();
        }
        Some(Command::Download {
            download_video,
            download_thumbnail,
        }) => {
            download::download(&command, download_video, download_thumbnail)
                .await;
        }
        None => {
            println!("No command specified");
        }
    }
}
