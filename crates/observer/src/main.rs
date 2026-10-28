mod artifacts;
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
use observer_artifacts::{Branch, Platform};

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
    /// List and install the FXServer builds Cfx.re publishes, even while Observer runs.
    #[command(subcommand)]
    Artifacts(ArtifactsCommand),
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

#[derive(Debug, Subcommand)]
enum ArtifactsCommand {
    /// List the builds of a platform with their channel, dates and support.
    List {
        /// `windows` or `linux`; the host's platform by default.
        #[arg(long)]
        platform: Option<Platform>,
        /// Also list the builds of a test branch, such as feature/network-improvements-clean.
        #[arg(long = "branch", value_name = "NAME")]
        branches: Vec<Branch>,
        /// Also list the old builds that only the support policy still names.
        #[arg(long)]
        all: bool,
    },
    /// Download a build, verify it and install it under the data directory, then print its folder.
    Install {
        /// The build number, such as 37872.
        build: u32,
        /// The test branch the build comes from, when it is not a master build.
        #[arg(long = "branch", value_name = "NAME")]
        branches: Vec<Branch>,
        /// Install a build even past its end of life, which players can no longer join.
        #[arg(long)]
        allow_end_of_life: bool,
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
            Command::Artifacts(ArtifactsCommand::List {
                platform,
                branches,
                all,
            }) => artifacts::list(platform, &branches, all).await,
            Command::Artifacts(ArtifactsCommand::Install {
                build,
                branches,
                allow_end_of_life,
            }) => artifacts::install(&config, build, &branches, allow_end_of_life).await,
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
