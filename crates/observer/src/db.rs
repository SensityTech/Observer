use anyhow::Context;
use observer_store::{BackupKind, Database};

use crate::config::Config;
use crate::lock;

/// `observer db check`: integrity and schema of the database, without changing it.
pub async fn check(config: &Config) -> anyhow::Result<()> {
    let paths = config.paths();
    let _lock = lock::acquire(&paths.root).context("stop Observer before checking its database")?;

    let inspection = Database::inspect(&paths.database).await?;
    println!("integrity: ok");
    println!(
        "schema: {} (this build: {})",
        inspection.schema, inspection.latest
    );
    if inspection.schema < inspection.latest {
        println!("the schema will be migrated on the next start, after a backup");
    }
    Ok(())
}

/// `observer db backup`: a snapshot of the database next to the automatic ones.
pub async fn backup(config: &Config) -> anyhow::Result<()> {
    let paths = config.paths();
    let _lock = lock::acquire(&paths.root).context("stop Observer before backing up its database")?;

    let backup = Database::backup_file(&paths.database, &paths.backups, BackupKind::Manual).await?;
    println!("{}", backup.display());
    Ok(())
}
