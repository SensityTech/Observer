use anyhow::{Context, anyhow};
use observer_app::{Accounts, Clock, PasswordCost};
use observer_domain::accounts::User;
use observer_domain::error::ErrorCode;
use observer_domain::permissions::Permissions;
use observer_store::Database;

use crate::config::Config;

/// Opens the accounts the way the command line may: alongside a running Observer, without
/// migrating, since SQLite lets the two processes take turns.
pub async fn open(config: &Config) -> anyhow::Result<(Database, Accounts)> {
    let paths = config.paths();
    let db = Database::open_existing(&paths.database, &paths.backups)
        .await
        .with_context(|| format!("cannot open {}", paths.database.display()))?;
    let accounts = Accounts::new(db.clone(), Clock::system(), PasswordCost::STANDARD).await?;
    Ok((db, accounts))
}

/// `observer user list`.
pub async fn list(config: &Config) -> anyhow::Result<()> {
    let (db, accounts) = open(config).await?;
    let users = accounts.users().await?;
    db.close().await?;

    if users.is_empty() {
        println!("no account yet; `observer setup-link` creates the owner");
        return Ok(());
    }
    let rows: Vec<[String; 4]> = users.iter().map(row).collect();
    let header = ["USERNAME", "ROLE", "LAST LOGIN", "PERMISSIONS"].map(str::to_owned);
    let widths: Vec<usize> = (0..3)
        .map(|column| {
            rows.iter()
                .chain([&header])
                .map(|row| row[column].len())
                .max()
                .unwrap_or_default()
        })
        .collect();
    for [name, role, login, permissions] in [&header].into_iter().chain(&rows) {
        println!(
            "{name:<0$}  {role:<1$}  {login:<2$}  {permissions}",
            widths[0], widths[1], widths[2]
        );
    }
    Ok(())
}

fn row(user: &User) -> [String; 4] {
    let role = match (user.is_owner, user.must_change_password) {
        (true, false) => "owner",
        (true, true) => "owner, temporary password",
        (false, false) => "user",
        (false, true) => "user, temporary password",
    };
    let permissions = if user.is_owner {
        "all".to_owned()
    } else if user.permissions == Permissions::NONE {
        "none".to_owned()
    } else {
        user.permissions
            .iter()
            .map(|info| info.constant)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let login = user.last_login_at.map_or_else(
        || "never".to_owned(),
        |at| at.strftime("%Y-%m-%d %H:%M UTC").to_string(),
    );
    [user.username.clone(), role.to_owned(), login, permissions]
}

/// `observer user reset-password`: a temporary password, shown here once.
pub async fn reset_password(config: &Config, name: &str) -> anyhow::Result<()> {
    let (db, accounts) = open(config).await?;
    let result = accounts.reset_password_from_cli(name).await;
    db.close().await?;

    let password = result.map_err(|error| unknown_user(error, name))?;
    println!("Temporary password for {name}: {password}");
    println!("It works for 72 hours; {name} chooses a new password right after signing in with it.");
    Ok(())
}

/// `observer user unlock`: lifts the login locks on a name, from every network.
pub async fn unlock(config: &Config, name: &str) -> anyhow::Result<()> {
    let (db, accounts) = open(config).await?;
    let result = accounts.unlock(name).await;
    db.close().await?;

    match result.map_err(|error| unknown_user(error, name))? {
        0 => println!("{name} was not locked out; any failed attempts were cleared"),
        1 => println!("{name} is unlocked on the one network that was locked out"),
        networks => println!("{name} is unlocked on the {networks} networks that were locked out"),
    }
    Ok(())
}

fn unknown_user(error: observer_app::Error, name: &str) -> anyhow::Error {
    if error.code() == Some(ErrorCode::NotFound) {
        anyhow!("no account is named {name}")
    } else {
        error.into()
    }
}
