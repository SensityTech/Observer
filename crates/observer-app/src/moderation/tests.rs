use std::net::{IpAddr, SocketAddr};

use observer_domain::audit::{AuditEntry, AuditFilter};
use observer_domain::permissions::Permissions;
use observer_domain::schedule::{ScheduleSettings, ScheduleStatus};
use tempfile::TempDir;
use tokio::sync::watch;

use super::*;
use crate::{Accounts, AuditLog, BansSetup, BansTask, Client, PasswordCost, Running, Servers, Setup};

/// The state database in a temporary folder, with the services the moderation stands on. No
/// server runs: what a kick does to one is tested against the fake FXServer.
struct Lab {
    _dir: TempDir,
    audit: AuditLog,
    moderation: Moderation,
    banning: BansTask,
    _running: Running,
}

impl Lab {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2026-11-11T20:00:00Z".parse().unwrap());
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");
        let (supervision, running) = Supervision::start(
            Setup {
                servers: Servers::new(db.clone(), clock.clone()),
                db: db.clone(),
                clock: clock.clone(),
                accounts,
                data_dir: data_dir.clone(),
                panel: SocketAddr::from(([127, 0, 0, 1], 41975)),
                artifacts: data_dir.join("artifacts"),
                console_logs: data_dir.join("logs/console"),
                log: None,
            },
            |_| {},
        )
        .await
        .unwrap();
        let status = ScheduleStatus {
            settings: ScheduleSettings::default(),
            version: None,
            zone: "Europe/Paris".to_owned(),
            next: None,
            upcoming: Vec::new(),
            one_off: None,
        };
        let (bans, banning) = Bans::start(BansSetup {
            db: db.clone(),
            clock: clock.clone(),
            supervision: supervision.clone(),
            schedule: watch::channel(status).1,
        });
        let moderation = Moderation::new(ModerationSetup {
            db: db.clone(),
            clock: clock.clone(),
            supervision,
            bans,
        });
        Self {
            _dir: dir,
            audit: AuditLog::new(db, clock),
            moderation,
            banning,
            _running: running,
        }
    }

    async fn audited(&self) -> Vec<AuditEntry> {
        self.audit.page(AuditFilter::default()).await.unwrap().entries
    }
}

fn staff() -> Caller {
    Caller {
        session_id: 1,
        user_id: 2,
        username: "maxime".to_owned(),
        is_owner: false,
        permissions: Permissions::PLAYERS_KICK | Permissions::SETTINGS_MANAGE,
        must_change_password: false,
        reauthenticated_at: None,
        client: Client {
            ip: IpAddr::from([127, 0, 0, 1]),
            user_agent: None,
        },
    }
}

fn refused(error: Error) -> ApiError {
    match error {
        Error::Refused(error) => error,
        other => panic!("not a refusal: {other:?}"),
    }
}

fn kick(reason: &str) -> KickRequest {
    KickRequest {
        player: 1,
        license: None,
        reason: reason.to_owned(),
    }
}

#[tokio::test]
async fn a_kick_needs_a_reason_within_its_length() {
    let lab = Lab::new().await;

    for reason in [" \u{200B} ".to_owned(), "r".repeat(MAX_KICK_REASON_CHARS + 1)] {
        let one = refused(lab.moderation.kick(&staff(), kick(&reason)).await.unwrap_err());
        let everyone = refused(
            lab.moderation
                .kick_all(&staff(), KickAllRequest { reason })
                .await
                .unwrap_err(),
        );

        assert_eq!(one.code, ErrorCode::InvalidKick);
        assert_eq!(one.params["max"], MAX_KICK_REASON_CHARS);
        assert_eq!(everyone.code, ErrorCode::InvalidKick);
    }
    lab.banning.close().await;
}

#[tokio::test]
async fn an_announcement_and_a_message_need_a_text_within_their_length() {
    let lab = Lab::new().await;

    for message in ["\n \n".to_owned(), "m".repeat(MAX_TEXT_CHARS + 1)] {
        let announcement = lab
            .moderation
            .announce(
                &staff(),
                AnnouncementRequest {
                    message: message.clone(),
                },
            )
            .await;
        let direct = lab
            .moderation
            .message(
                &staff(),
                DirectMessageRequest {
                    player: 1,
                    license: None,
                    message,
                },
            )
            .await;

        let announcement = refused(announcement.unwrap_err());
        assert_eq!(announcement.code, ErrorCode::InvalidMessage);
        assert_eq!(announcement.params["max"], MAX_TEXT_CHARS);
        assert_eq!(refused(direct.unwrap_err()).code, ErrorCode::InvalidMessage);
    }
    lab.banning.close().await;
}

#[tokio::test]
async fn a_server_that_does_not_run_takes_nothing_and_nothing_is_audited() {
    let lab = Lab::new().await;
    let caller = staff();
    let message = || "Restart in five minutes".to_owned();

    let refusals = [
        lab.moderation.kick(&caller, kick("RDM")).await.unwrap_err(),
        lab.moderation
            .kick_all(
                &caller,
                KickAllRequest {
                    reason: "Maintenance".to_owned(),
                },
            )
            .await
            .unwrap_err(),
        lab.moderation
            .announce(&caller, AnnouncementRequest { message: message() })
            .await
            .unwrap_err(),
        lab.moderation
            .message(
                &caller,
                DirectMessageRequest {
                    player: 1,
                    license: None,
                    message: message(),
                },
            )
            .await
            .unwrap_err(),
    ];

    for refusal in refusals {
        assert_eq!(refused(refusal).code, ErrorCode::ServerNotRunning);
    }
    assert_eq!(lab.audited().await, []);
    lab.banning.close().await;
}

#[tokio::test]
async fn the_kick_message_is_saved_cleaned_and_audited_once() {
    let lab = Lab::new().await;
    let caller = staff();
    let saved = KickSettings {
        message: "Kicked by {by}\u{202E}: {reason}\n".to_owned(),
    };

    let empty = lab
        .moderation
        .change_settings(
            &caller,
            KickSettings {
                message: "\n \n".to_owned(),
            },
        )
        .await
        .unwrap_err();
    lab.moderation
        .change_settings(&caller, saved.clone())
        .await
        .unwrap();
    lab.moderation.change_settings(&caller, saved).await.unwrap();

    let empty = refused(empty);
    assert_eq!(empty.code, ErrorCode::InvalidKickSettings);
    assert_eq!(empty.params["maxMessage"], MAX_KICK_MESSAGE_CHARS);
    assert_eq!(
        lab.moderation.settings().await.unwrap().message,
        "Kicked by {by}: {reason}"
    );
    let audited = lab.audited().await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].action, AuditAction::KickSettingsChanged);
    assert_eq!(audited[0].actor_name.as_deref(), Some("maxime"));
    assert_eq!(
        audited[0].details["from"]["message"],
        KickSettings::default().message
    );
    assert_eq!(audited[0].details["to"]["message"], "Kicked by {by}: {reason}");
    lab.banning.close().await;
}

#[tokio::test]
async fn a_staff_kick_carries_the_owners_message_with_the_reason_and_who_kicked() {
    let lab = Lab::new().await;

    let kick = lab
        .moderation
        .staff_kick(KickCause::Kick, "RDM".to_owned(), &staff())
        .await
        .unwrap();

    assert_eq!(
        kick,
        Kick {
            cause: KickCause::Kick,
            reason: "RDM".to_owned(),
            by: "maxime".to_owned(),
            message: "You were kicked from this server.\nReason: RDM".to_owned(),
        }
    );
    lab.banning.close().await;
}
