//! Maintenance mode: who it keeps out at connection, its switch, the players who still get in,
//! and its settings.

use jiff::Timestamp;
use observer_domain::allowlist::{MAX_NOTE_CHARS, parse_license};
use observer_domain::audit::AuditAction;
use observer_domain::bans::{MAX_MESSAGE_CHARS, clean_line};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::maintenance::{
    self as rules, AddPass, MAX_PASSES, MAX_SERVER_NAME_CHARS, MaintenancePass, MaintenancePasses,
    MaintenancePreview, MaintenanceSettings, SetMaintenance,
};
use observer_fxserver::bridge::{BridgeAction, BridgeEvent, MaintenanceEvent, Refusal, Refused};
use observer_store::maintenance::{self as stored, NewPass};
use serde_json::json;
use tracing::warn;

use super::Admission;
use crate::audit::Record;
use crate::bans::Seen;
use crate::{Caller, Error, Servers};

/// What maintenance makes of a connecting player.
pub(super) enum Door {
    /// Maintenance is off: the allowlist has its say.
    Open,
    /// The player is on the list of those who still get in.
    Passed,
    Closed(Refusal),
}

impl Admission {
    /// What maintenance makes of the player. A database that cannot tell leaves the door open, so
    /// that a failing disk never locks everyone out.
    pub(super) async fn door(&self, seen: &Seen) -> Door {
        let license = seen.license.clone();
        let found = self
            .inner
            .db
            .read(move |conn| -> Result<_, Error> {
                if !stored::is_on(conn)? {
                    return Ok(None);
                }
                let passes = match &license {
                    Some(license) => stored::has_pass(conn, license)?,
                    None => false,
                };
                Ok(Some(if passes {
                    None
                } else {
                    Some(stored::settings(conn)?)
                }))
            })
            .await;
        match found {
            Ok(None) => Door::Open,
            Ok(Some(None)) => Door::Passed,
            Ok(Some(Some(settings))) => Door::Closed(Refusal {
                message: rules::refusal(&settings.message, seen.name.as_deref().unwrap_or_default()),
                event: Refused::Maintenance(MaintenanceEvent {
                    name: seen.name.clone(),
                    license: seen.license.clone(),
                }),
            }),
            Err(error) => {
                warn!(%error, "a player could not be checked against maintenance mode");
                Door::Open
            }
        }
    }

    /// Tells the resources of a start that maintenance is on, as they heard nothing of it yet,
    /// and gives the server its maintenance name again: the cfg has just given it its own.
    pub(super) async fn started(&self) {
        let read = self
            .inner
            .db
            .read(|conn| -> Result<_, Error> { Ok((stored::status(conn)?, stored::settings(conn)?)) })
            .await;
        match read {
            Ok((status, settings)) => {
                if let (true, Some(since), Some(by)) = (status.on, status.since, status.by) {
                    self.announce(true, since, by).await;
                    if settings.server_name.is_some() {
                        self.rename(settings.server_name).await;
                    }
                }
            }
            Err(error) => warn!(%error, "cannot tell a start whether maintenance mode is on"),
        }
    }

    /// Raises `observer:maintenance`. Best effort, as every event: a server that is not running
    /// hears of a maintenance still on at its next start.
    async fn announce(&self, on: bool, at: Timestamp, by: String) {
        let event = BridgeEvent::Maintenance {
            on,
            at: at.as_millisecond(),
            by,
        };
        let _ = self.inner.supervisor.announce(event).await;
    }

    /// Has the bridge give the server `name` in FiveM's server list, or its own back without one.
    /// Best effort too: a server that starts later gets its own name from its cfg, and the
    /// maintenance one once it is online.
    async fn rename(&self, name: Option<String>) {
        let _ = self.inner.supervisor.act(BridgeAction::ServerName { name }).await;
    }

    /// What whoever turns maintenance on is shown first: the message players will read, and the
    /// name the server will take.
    pub async fn maintenance_preview(&self) -> Result<MaintenancePreview, Error> {
        let settings = self.maintenance_settings().await?;
        Ok(MaintenancePreview {
            message: settings.message,
            server_name: settings.server_name,
        })
    }

    /// Closes the server to every player but those on the list, or opens it again, from the next
    /// connection. Asking for what already is changes nothing.
    pub async fn set_maintenance(&self, caller: &Caller, request: SetMaintenance) -> Result<(), Error> {
        let action = if request.on {
            AuditAction::MaintenanceStarted
        } else {
            AuditAction::MaintenanceEnded
        };
        let record = Record::by(caller, action).target(Servers::DEFAULT);
        let by = caller.username.clone();
        let now = self.inner.clock.now();
        let changed = self
            .inner
            .db
            .write({
                let by = by.clone();
                move |tx| -> Result<_, Error> {
                    let before = stored::status(tx)?;
                    if before.on == request.on {
                        return Ok(None);
                    }
                    if request.on {
                        stored::turn_on(tx, now, &by)?;
                        record
                            .details(json!({ "passes": before.passes }))
                            .write(tx, now)?;
                    } else {
                        stored::turn_off(tx)?;
                        record.details(json!({ "since": before.since })).write(tx, now)?;
                    }
                    Ok(Some(stored::settings(tx)?.server_name))
                }
            })
            .await?;
        if let Some(server_name) = changed {
            self.refreshed().await;
            self.announce(request.on, now, by).await;
            if request.on {
                if server_name.is_some() {
                    self.rename(server_name).await;
                }
            } else {
                self.rename(None).await;
            }
        }
        Ok(())
    }

    pub async fn maintenance_settings(&self) -> Result<MaintenanceSettings, Error> {
        Ok(self.inner.db.read(stored::settings).await?)
    }

    /// Changes what a player refused during maintenance reads, from the next connection, and the
    /// name the server takes in FiveM's server list: at once while maintenance is on.
    pub async fn change_maintenance_settings(
        &self,
        caller: &Caller,
        settings: MaintenanceSettings,
    ) -> Result<(), Error> {
        let settings = settings.checked().ok_or_else(|| {
            Error::from(
                ApiError::new(ErrorCode::InvalidMaintenanceSettings)
                    .with("maxMessage", MAX_MESSAGE_CHARS)
                    .with("maxName", MAX_SERVER_NAME_CHARS),
            )
        })?;
        let before = self.maintenance_settings().await?;
        if before == settings {
            return Ok(());
        }
        let record = Record::by(caller, AuditAction::MaintenanceSettingsChanged)
            .details(json!({ "from": before, "to": settings }));
        let renamed = (before.server_name != settings.server_name).then(|| settings.server_name.clone());
        let now = self.inner.clock.now();
        let on = self
            .inner
            .db
            .write(move |tx| -> Result<bool, Error> {
                stored::save_settings(tx, &settings)?;
                record.write(tx, now)?;
                Ok(stored::is_on(tx)?)
            })
            .await?;
        if let (true, Some(server_name)) = (on, renamed) {
            self.rename(server_name).await;
        }
        Ok(())
    }

    /// The players who still get in during maintenance, latest added first.
    pub async fn passes(&self) -> Result<MaintenancePasses, Error> {
        Ok(MaintenancePasses {
            passes: self.inner.db.read(stored::passes).await?,
            zone: self.zone(),
        })
    }

    /// Puts a licence on the list of those who still get in, with a note for whoever reads it.
    pub async fn add_pass(&self, caller: &Caller, request: AddPass) -> Result<MaintenancePass, Error> {
        let invalid = |field: &str| {
            Error::from(
                ApiError::new(ErrorCode::InvalidPass)
                    .with("field", field)
                    .with("maxNote", MAX_NOTE_CHARS),
            )
        };
        let license = parse_license(&request.license).ok_or_else(|| invalid("license"))?;
        let note = match request.note.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(text) => Some(clean_line(text, MAX_NOTE_CHARS).ok_or_else(|| invalid("note"))?),
        };
        let record = Record::by(caller, AuditAction::MaintenancePassAdded);
        let (by_user, by_name) = (caller.user_id, caller.username.clone());
        let now = self.inner.clock.now();
        let pass = self
            .inner
            .db
            .write(move |tx| -> Result<_, Error> {
                if stored::status(tx)?.passes >= MAX_PASSES {
                    return Err(ApiError::new(ErrorCode::TooManyPasses)
                        .with("max", MAX_PASSES)
                        .into());
                }
                let id = stored::add_pass(
                    tx,
                    &NewPass {
                        license: &license,
                        note: note.as_deref(),
                        by_user: Some(by_user),
                        by_name: &by_name,
                        at: now,
                    },
                )?
                .ok_or(ErrorCode::PassExists)?;
                let pass = stored::pass_by_id(tx, id)?.ok_or(ErrorCode::PassNotFound)?;
                record.details(pass_details(&pass)).write(tx, now)?;
                Ok(pass)
            })
            .await?;
        self.refreshed().await;
        Ok(pass)
    }

    /// Takes a licence off the list: maintenance refuses its player from their next connection.
    /// One who plays stays until they leave.
    pub async fn remove_pass(&self, caller: &Caller, id: i64) -> Result<(), Error> {
        let record = Record::by(caller, AuditAction::MaintenancePassRemoved);
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                let pass = stored::remove_pass(tx, id)?.ok_or(ErrorCode::PassNotFound)?;
                Ok(record.details(pass_details(&pass)).write(tx, now)?)
            })
            .await?;
        self.refreshed().await;
        Ok(())
    }
}

fn pass_details(pass: &MaintenancePass) -> serde_json::Value {
    json!({ "license": pass.license, "playerName": pass.name, "note": pass.note })
}
