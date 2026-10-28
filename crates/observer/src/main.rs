mod config;
mod db;
mod lock;
mod logging;
mod run;
mod shutdown;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::config::Config;

#[derive(Debug, Parser)]
#[command(name = "observer", version, about = "Server manager for FiveM and RedM")]
struct Cli {
    /// Path to the configuration file. Relative paths inside it are resolved against its directory.
    #[arg(long, global = true, default_value = "observer.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run Observer in the foreground. This is the default.
    Run,
    /// Maintain the state database while Observer is stopped.
    #[command(subcommand)]
    Db(DbCommand),
}

#[derive(Debug, Subcommand)]
enum DbCommand {
    /// Check the database's integrity and schema version.
    Check,
    /// Write a backup of the database now.
    Backup,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match Config::load(&cli.config) {
        Ok(config) => match cli.command.unwrap_or(Command::Run) {
            Command::Run => run::run(config).await,
            Command::Db(DbCommand::Check) => db::check(&config).await,
            Command::Db(DbCommand::Backup) => db::backup(&config).await,
        },
        Err(error) => Err(error),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
