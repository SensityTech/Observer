mod artifacts;
mod config;
mod db;
#[cfg(windows)]
mod desktop;
mod fxserver;
mod lock;
mod logging;
mod run;
mod setup;
mod shutdown;
#[cfg(target_os = "linux")]
mod systemd;
mod tools;
mod update;
mod user;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use observer_artifacts::{Branch, Platform};
use observer_domain::about::Build;

use crate::config::Config;

/// `cargo xtask dist` names the commit; a plain `cargo build` does not.
pub const BUILD: Build = Build {
    version: env!("CARGO_PKG_VERSION"),
    commit: option_env!("OBSERVER_COMMIT"),
    commit_date: option_env!("OBSERVER_COMMIT_DATE"),
};

static VERSION: LazyLock<String> = LazyLock::new(|| version(BUILD));

/// `0.1.0 (3f2a9c1b7d4e, 2026-10-28)`, or `0.1.0 (development build)`.
fn version(build: Build) -> String {
    match (build.commit, build.commit_date) {
        (Some(commit), Some(date)) => format!("{} ({commit}, {date})", build.version),
        (Some(commit), None) => format!("{} ({commit})", build.version),
        _ => format!("{} (development build)", build.version),
    }
}

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
    Run(run::RunArgs),
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
    /// Drive FXServer by hand, without the panel.
    #[command(subcommand)]
    Fxserver(FxserverCommand),
    /// Run the server tools by hand: a report at any time, a fix or an undo while Observer is
    /// stopped. Exits with 2 when duplicates are left.
    #[command(subcommand)]
    Tools(tools::ToolsCommand),
    /// Run Observer as a systemd service.
    #[cfg(target_os = "linux")]
    #[command(subcommand)]
    Service(ServiceCommand),
}

#[cfg(target_os = "linux")]
#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Print a systemd unit that runs this binary with this configuration, as the user running
    /// the command. It writes nothing: it says where the unit goes.
    PrintUnit,
}

#[derive(Debug, Subcommand)]
enum FxserverCommand {
    /// Run FXServer under the supervisor in the foreground, printing its console and state
    /// changes. Lines typed here go to its console. Ctrl+C stops it gracefully.
    Run(fxserver::RunArgs),
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
    let matches = Cli::command().version(VERSION.as_str()).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());

    let result = match Config::load(&cli.config) {
        Ok(config) => {
            let command = cli.command.unwrap_or(Command::Run(run::RunArgs::default()));
            perform(config, command, &cli.config).await
        }
        Err(error) => Err(error),
    };

    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Runs a command: every one exits with success unless it fails, but the tools', which tell
/// scripts whether duplicates are left.
async fn perform(
    config: Config,
    command: Command,
    config_path: &std::path::Path,
) -> anyhow::Result<ExitCode> {
    #[cfg(not(target_os = "linux"))]
    let _ = config_path;
    let done = match command {
        Command::Tools(command) => return tools::command(&config, command).await,
        Command::Run(args) => {
            return match run::run(config, args).await? {
                run::Exit::Stopped => Ok(ExitCode::SUCCESS),
                run::Exit::Relaunch(relaunch) => Ok(update::relaunch(&relaunch)),
            };
        }
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
        Command::Fxserver(FxserverCommand::Run(args)) => fxserver::run(config, args).await,
        #[cfg(target_os = "linux")]
        Command::Service(ServiceCommand::PrintUnit) => systemd::print_unit(config_path),
    };
    done.map(|()| ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_names_the_commit_when_the_build_knows_it() {
        let build = |commit, commit_date| Build {
            version: "0.1.0",
            commit,
            commit_date,
        };

        assert_eq!(
            version(build(Some("3f2a9c1b7d4e"), Some("2026-10-28"))),
            "0.1.0 (3f2a9c1b7d4e, 2026-10-28)"
        );
        assert_eq!(version(build(None, None)), "0.1.0 (development build)");
    }
}
