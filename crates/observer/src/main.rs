mod config;
mod db;
mod lock;
mod logging;
mod run;
mod setup;
mod shutdown;
mod user;

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
    /// Manage accounts from the host, even while Observer runs.
    #[command(subcommand)]
    User(UserCommand),
    /// Print a new link that creates the owner account, while none exists.
    SetupLink,
}

#[derive(Debug, Subcommand)]
enum DbCommand {
    /// Check the database's integrity and schema version.
    Check,
    /// Write a backup of the database now.
    Backup,
}

#[derive(Debug, Subcommand)]
enum UserCommand {
    /// List every account with its role and permissions.
    List,
    /// Give an account a temporary password and end its sessions. Works for the owner too.
    ResetPassword {
        /// The account's username.
        name: String,
    },
    /// Lift the login locks on an account, from every network.
    Unlock {
        /// The account's username.
        name: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match Config::load(&cli.config) {
        Ok(config) => match cli.command.unwrap_or(Command::Run) {
            Command::Run => run::run(config).await,
            Command::Db(DbCommand::Check) => db::check(&config).await,
            Command::Db(DbCommand::Backup) => db::backup(&config).await,
            Command::User(UserCommand::List) => user::list(&config).await,
            Command::User(UserCommand::ResetPassword { name }) => user::reset_password(&config, &name).await,
            Command::User(UserCommand::Unlock { name }) => user::unlock(&config, &name).await,
            Command::SetupLink => setup::link(&config).await,
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
