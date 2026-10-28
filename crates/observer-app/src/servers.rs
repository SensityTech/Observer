use observer_domain::audit::AuditAction;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::launch::{LaunchSettings, Platform};
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
    pub async fn launch_settings(&self, server: &str) -> Result<Option<LaunchSettings>, Error> {
        let server = server.to_owned();
        Ok(self.db.read(move |conn| launch::get(conn, &server)).await?)
    }

    /// Stores launch settings that pass every check, audited with what changed. FXServer picks
    /// them up at its next start. Saving the same settings again changes and audits nothing.
    pub async fn change_launch_settings(
        &self,
        caller: &Caller,
        server: &str,
        settings: LaunchSettings,
    ) -> Result<LaunchSettings, Error> {
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
                if before.as_ref() == Some(&settings) {
                    return Ok(settings);
                }
                launch::save(tx, &server, &settings, now)?;
                record
                    .details(changes(before.as_ref(), &settings))
                    .write(tx, now)?;
                Ok(settings)
            })
            .await
    }
}

/// Each setting that differs, with its value before and after. Before the first save, every
/// setting was null.
fn changes(before: Option<&LaunchSettings>, after: &LaunchSettings) -> Value {
    let fields = |settings: Option<&LaunchSettings>| match settings.map(serde_json::to_value) {
        Some(Ok(Value::Object(fields))) => fields,
        _ => Map::new(),
    };
    let before = fields(before);
    let changed = fields(Some(after))
        .into_iter()
        .filter_map(|(name, value)| {
            let old = before.get(&name).cloned().unwrap_or(Value::Null);
            (old != value).then(|| (name, json!({ "before": old, "after": value })))
        })
        .collect();
    Value::Object(changed)
}
