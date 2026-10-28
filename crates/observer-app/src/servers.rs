use observer_domain::audit::AuditAction;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::launch::{ExtraArgument, LaunchSettings, Platform, StoredLaunch};
use observer_store::{Database, launch};
use serde_json::{Map, Value, json};

use crate::audit::Record;
use crate::{Caller, Clock, Error};

/// The game servers Observer runs. For now there is one, and what it keeps is how FXServer is
/// launched.
#[derive(Clone)]
pub struct Servers {
    db: Database,
    clock: Clock,
}

impl Servers {
    /// The server of a single-server Observer. Routes already name it, so a second one changes
    /// no path.
    pub const DEFAULT: &str = "default";

    pub fn new(db: Database, clock: Clock) -> Self {
        Self { db, clock }
    }

    /// `None` until the owner first saves them.
    pub async fn launch_settings(&self, server: &str) -> Result<Option<StoredLaunch>, Error> {
        let server = server.to_owned();
        Ok(self.db.read(move |conn| launch::get(conn, &server)).await?)
    }

    /// Stores launch settings that pass every check, audited with what changed. FXServer picks
    /// them up at its next start. `version` must be the one the change was made from, `None`
    /// before the first save. Saving the same settings again changes and audits nothing.
    pub async fn change_launch_settings(
        &self,
        caller: &Caller,
        server: &str,
        settings: LaunchSettings,
        version: Option<i64>,
    ) -> Result<StoredLaunch, Error> {
        let issues = settings.issues(Platform::host());
        if !issues.is_empty() {
            return Err(ApiError::new(ErrorCode::InvalidLaunchSettings)
                .with("issues", serde_json::to_value(issues).unwrap_or_default())
                .into());
        }

        let record = Record::by(caller, AuditAction::LaunchSettingsChanged).target(server);
        let server = server.to_owned();
        let now = self.clock.now();
        self.db
            .write(move |tx| -> Result<_, Error> {
                let before = launch::get(tx, &server)?;
                if before.as_ref().map(|before| before.version) != version {
                    return Err(ErrorCode::Conflict.into());
                }
                if let Some(same) = before.as_ref().filter(|before| before.settings == settings) {
                    return Ok(same.clone());
                }
                let stored = StoredLaunch {
                    settings,
                    version: version.map_or(1, |version| version + 1),
                };
                launch::save(tx, &server, &stored, now)?;
                let old = before.as_ref().map(|before| &before.settings);
                record.details(changes(old, &stored.settings)).write(tx, now)?;
                Ok(stored)
            })
            .await
    }
}

/// The settings whose values anyone reading the log may see. Only the owner reads the others,
/// and an argument may well hold a token: the log only says they changed.
const SHOWN: [&str; 3] = ["onesync", "autoStart", "txadminEvents"];

/// Each setting that differs: its value before and after for the shown ones, `true` for a hidden
/// one, and the names of the arguments set or removed. Before the first save, every setting was
/// null.
fn changes(before: Option<&LaunchSettings>, after: &LaunchSettings) -> Value {
    let fields = |settings: Option<&LaunchSettings>| match settings.map(serde_json::to_value) {
        Some(Ok(Value::Object(fields))) => fields,
        _ => Map::new(),
    };
    let old_fields = fields(before);
    let changed = fields(Some(after))
        .into_iter()
        .filter_map(|(name, value)| {
            let old = old_fields.get(&name).cloned().unwrap_or(Value::Null);
            if old == value {
                return None;
            }
            let change = match name.as_str() {
                "extraArguments" => arguments(
                    before.map_or(&[], |before| &before.extra_arguments),
                    &after.extra_arguments,
                )?,
                shown if SHOWN.contains(&shown) => json!({ "before": old, "after": value }),
                _ => Value::Bool(true),
            };
            Some((name, change))
        })
        .collect();
    Value::Object(changed)
}

/// The names of the arguments added or given another value, and of those gone. `None` when the
/// list is the same.
fn arguments(before: &[ExtraArgument], after: &[ExtraArgument]) -> Option<Value> {
    if before == after {
        return None;
    }
    let set: Vec<&str> = after
        .iter()
        .filter(|argument| !before.contains(argument))
        .map(|argument| argument.name.as_str())
        .collect();
    let removed: Vec<&str> = before
        .iter()
        .filter(|old| !after.iter().any(|new| new.name == old.name))
        .map(|old| old.name.as_str())
        .collect();
    Some(json!({ "set": set, "removed": removed }))
}
