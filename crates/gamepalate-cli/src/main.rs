mod corrections;
mod data_dir;
mod evaluate;
mod format;
mod names;
mod profile;
mod recommend;

use std::{path::PathBuf, process::ExitCode};

use anyhow::Context;
use clap::{Parser, Subcommand};
use gamepalate_store::data_files::load_data_files;

/// Recommends Steam games from your library, playtime and votes
#[derive(Parser)]
#[command(name = "gamepalate", bin_name = "gamepalate", version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Folder with config.toml, catalog.sqlite and the other data files
    #[arg(long, env = "GAMEPALATE_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Show the tags you like most and your families
    Profile(profile::ProfileArgs),
    /// Recommend games from your signals and families
    Recommend(recommend::RecommendArgs),
    /// Measure how well your tastes are recognized
    Evaluate(evaluate::EvaluateArgs),
    /// List the active corrections
    Corrections,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let data_dir = data_dir::resolve(cli.data_dir).context(
        "could not find the user data folder: use --data-dir or set GAMEPALATE_DATA_DIR",
    )?;

    match cli.command {
        Command::Profile(args) => {
            let files = load_data_files(&data_dir)?;
            profile::run(&args, &files, &data_dir)?;
        }
        Command::Recommend(args) => {
            let files = load_data_files(&data_dir)?;
            recommend::run(&args, &files, &data_dir)?;
        }
        Command::Evaluate(args) => {
            let files = load_data_files(&data_dir)?;
            evaluate::run(&args, &files, &data_dir)?;
        }
        Command::Corrections => {
            let files = load_data_files(&data_dir)?;
            corrections::run(&files);
        }
    }

    Ok(())
}
