//! What the staff does to the players connected now: a kick, with the message the owner writes
//! for it, and the announcements and messages the bridge raises for the server's resources to
//! show. Each one is audited before it reaches FXServer.

use std::sync::Arc;

use observer_domain::audit::AuditAction;
use observer_domain::bans::clean_line;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::moderation::{
    AnnouncementRequest, DirectMessageRequest, KickAllRequest, KickRequest, KickSettings, KickedAll,
    MAX_KICK_MESSAGE_CHARS, MAX_KICK_REASON_CHARS, MAX_TEXT_CHARS, clean_text, kick_message,
};
use observer_fxserver::bridge::{BridgeAction, BridgeEvent, KickCause, KickEvent};
use observer_fxserver::supervisor::Supervisor;
use observer_store::Database;
use observer_store::moderation as stored;
use serde_json::json;

use crate::audit::Record;
use crate::bans::ConnectedPlayer;
use crate::supervision::control_error;
use crate::{Bans, Caller, Clock, Error, Supervision};

pub struct ModerationSetup {
    pub db: Database,
    pub clock: Clock,
    pub supervision: Supervision,
    /// Who is connected, as the bans follow them.
    pub bans: Bans,
}

/// The moderation service. Clones share it.
#[derive(Clone)]
pub struct Moderation {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    supervisor: Supervisor,
    bans: Bans,
}

/// Whom a kick drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KickTargets {
    /// Everyone connected when the server runs the kick.
    Everyone,
    /// The players of these server ids who are still connected.
    Players(Vec<u32>),
}

/// A kick as the bridge carries it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Kick {
    pub cause: KickCause,
    /// The reason `observer:playerKicked` gives.
    pub reason: String,
    pub by: String,
    /// What the kicked players read.
    pub message: String,
}

impl Moderation {
    pub fn new(setup: ModerationSetup) -> Self {
        Self {
            inner: Arc::new(Inner {
                db: setup.db,
                clock: setup.clock,
                supervisor: setup.supervision.supervisor().clone(),
                bans: setup.bans,
            }),
        }
    }

    /// Kicks one connected player, who reads the owner's kick message.
    pub async fn kick(&self, caller: &Caller, request: KickRequest) -> Result<(), Error> {
        let reason = kick_reason(&request.reason)?;
        let player = self.connected(request.player, request.license.as_deref())?;
        let kick = self.staff_kick(KickCause::Kick, reason, caller).await?;
        let record = Record::by(caller, AuditAction::PlayerKicked).details(json!({
            "serverId": player.net_id,
            "license": player.license,
            "reason": kick.reason,
        }));
        self.audit(of_player(record, &player)).await?;
        self.kick_players(KickTargets::Players(vec![player.net_id]), kick)
            .await
    }

    /// Kicks everyone connected, with the same message.
    pub async fn kick_all(&self, caller: &Caller, request: KickAllRequest) -> Result<KickedAll, Error> {
        let reason = kick_reason(&request.reason)?;
        let connected = self.running()?;
        let players = u32::try_from(connected.len()).unwrap_or(u32::MAX);
        let kick = self.staff_kick(KickCause::KickAll, reason, caller).await?;
        let record = Record::by(caller, AuditAction::EveryoneKicked)
            .details(json!({ "reason": kick.reason, "players": players }));
        self.audit(record).await?;
        self.kick_players(KickTargets::Everyone, kick).await?;
        Ok(KickedAll { players })
    }

    /// Has the bridge raise `observer:playerKicked` for each player of the kick, then drop them
    /// with its message. The entry of every kick: it checks and audits nothing, its caller does.
    pub(crate) async fn kick_players(&self, targets: KickTargets, kick: Kick) -> Result<(), Error> {
        let event = KickEvent {
            reason: kick.reason,
            by: kick.by,
            cause: kick.cause,
        };
        let actions = match targets {
            KickTargets::Everyone => vec![BridgeAction::kick_all(kick.message, event)],
            KickTargets::Players(players) => BridgeAction::kicks(&players, kick.message, event),
        };
        for action in actions {
            self.inner.supervisor.act(action).await.map_err(control_error)?;
        }
        Ok(())
    }

    /// Raises `observer:announcement` for the server's resources to show.
    pub async fn announce(&self, caller: &Caller, request: AnnouncementRequest) -> Result<(), Error> {
        let message = text(&request.message)?;
        self.running()?;
        let record = Record::by(caller, AuditAction::AnnouncementSent).details(json!({ "message": message }));
        self.audit(record).await?;
        self.raise(BridgeEvent::Announcement {
            message,
            by: caller.username.clone(),
        })
        .await
    }

    /// Raises `observer:directMessage` for one connected player.
    pub async fn message(&self, caller: &Caller, request: DirectMessageRequest) -> Result<(), Error> {
        let message = text(&request.message)?;
        let player = self.connected(request.player, request.license.as_deref())?;
        let record = Record::by(caller, AuditAction::DirectMessageSent).details(json!({
            "serverId": player.net_id,
            "license": player.license,
            "message": message,
        }));
        self.audit(of_player(record, &player)).await?;
        self.raise(BridgeEvent::DirectMessage {
            player: player.net_id,
            message,
            by: caller.username.clone(),
        })
        .await
    }

    pub async fn settings(&self) -> Result<KickSettings, Error> {
        Ok(self.inner.db.read(stored::kick_settings).await?)
    }

    /// Changes the kick message, from the next kick.
    pub async fn change_settings(&self, caller: &Caller, settings: KickSettings) -> Result<(), Error> {
        let settings = settings.checked().ok_or_else(|| {
            Error::from(
                ApiError::new(ErrorCode::InvalidKickSettings).with("maxMessage", MAX_KICK_MESSAGE_CHARS),
            )
        })?;
        let before = self.settings().await?;
        if before == settings {
            return Ok(());
        }
        let record = Record::by(caller, AuditAction::KickSettingsChanged)
            .details(json!({ "from": before, "to": settings }));
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                stored::save_kick_settings(tx, &settings)?;
                Ok(record.write(tx, now)?)
            })
            .await
    }

    /// A kick by the staff: what its players read is the owner's message, with the reason.
    async fn staff_kick(&self, cause: KickCause, reason: String, caller: &Caller) -> Result<Kick, Error> {
        let settings = self.settings().await?;
        Ok(Kick {
            cause,
            message: kick_message(&settings.message, &reason, &caller.username),
            reason,
            by: caller.username.clone(),
        })
    }

    /// The players of the run under way, or the refusal of a server that takes no action.
    fn running(&self) -> Result<Vec<ConnectedPlayer>, Error> {
        self.inner
            .bans
            .connected_players()
            .ok_or_else(|| ErrorCode::ServerNotRunning.into())
    }

    /// The player who holds `net_id` now. With `license`, the one the panel showed under that id:
    /// another player may hold it since, after a restart of the server for one.
    fn connected(&self, net_id: u32, license: Option<&str>) -> Result<ConnectedPlayer, Error> {
        self.running()?
            .into_iter()
            .find(|player| player.net_id == net_id)
            .filter(|player| license.is_none_or(|license| player.license.as_deref() == Some(license)))
            .ok_or_else(|| ErrorCode::PlayerNotConnected.into())
    }

    async fn raise(&self, event: BridgeEvent) -> Result<(), Error> {
        self.inner.supervisor.announce(event).await.map_err(control_error)
    }

    async fn audit(&self, record: Record) -> Result<(), Error> {
        let now = self.inner.clock.now();
        Ok(self.inner.db.write(move |tx| record.write(tx, now)).await?)
    }
}

/// The entry names the player as the server does, when it gave a name.
fn of_player(record: Record, player: &ConnectedPlayer) -> Record {
    match &player.name {
        Some(name) => record.target(name),
        None => record,
    }
}

fn kick_reason(reason: &str) -> Result<String, Error> {
    clean_line(reason, MAX_KICK_REASON_CHARS).ok_or_else(|| {
        ApiError::new(ErrorCode::InvalidKick)
            .with("max", MAX_KICK_REASON_CHARS)
            .into()
    })
}

fn text(message: &str) -> Result<String, Error> {
    clean_text(message).ok_or_else(|| {
        ApiError::new(ErrorCode::InvalidMessage)
            .with("max", MAX_TEXT_CHARS)
            .into()
    })
}

#[cfg(test)]
mod tests;
