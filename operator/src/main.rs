use clap::{Parser, Subcommand};

mod downloads;
mod executors;
mod util;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    ManageDownloads,
    ManageExecutors,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::ManageDownloads) => {
            downloads::main().await
        }
        Some(Command::ManageExecutors) => {
            executors::main().await
        }
        None => {
            println!("Please choose a subcommand.");
        }
    }
}
