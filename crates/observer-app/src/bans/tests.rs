use std::net::{IpAddr, SocketAddr};

use observer_domain::bans::BanState;
use observer_domain::players::IdentifierKind;
use observer_domain::schedule::ScheduleSettings;
use observer_store::StoreError;
use observer_store::players::{self as players, Seen as PlayerSeen};
use tempfile::TempDir;

use super::*;
use crate::{Accounts, Client, PasswordCost, Running, Servers, Setup};

const KAITO: &str = "license:00000000000000000000000000000000000000aa";
const DISCORD: &str = "123456789012345678";

fn token(n: u8) -> String {
    format!("4:{}", format!("{n:02x}").repeat(32))
}

/// The state database in a temporary folder, with the supervision the bans stand on.
struct Lab {
    _dir: TempDir,
    db: Database,
    clock: Clock,
    supervision: Supervision,
    _running: Running,
}

impl Lab {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2026-11-09T20:00:00Z".parse().unwrap());
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
        Self {
            _dir: dir,
            db,
            clock,
            supervision,
            _running: running,
        }
    }

    fn bans(&self) -> (Bans, BansTask) {
        let status = ScheduleStatus {
            settings: ScheduleSettings::default(),
            version: None,
            zone: "Europe/Paris".to_owned(),
            next: None,
            upcoming: Vec::new(),
            one_off: None,
        };
        Bans::start(BansSetup {
            db: self.db.clone(),
            clock: self.clock.clone(),
            supervision: self.supervision.clone(),
            schedule: watch::channel(status).1,
        })
    }

    /// kaito, as a confirmed join wrote him: a Discord account and two tokens.
    async fn kaito(&self) -> i64 {
        let at = self.clock.now();
        self.db
            .write(move |tx| -> Result<i64, StoreError> {
                let others = [
                    (IdentifierKind::Discord, DISCORD.to_owned()),
                    (IdentifierKind::Token, token(1)),
                    (IdentifierKind::Token, token(2)),
                ];
                let (player, _) = players::record_seen(
                    tx,
                    &PlayerSeen {
                        license: KAITO,
                        name: Some("kaito"),
                        others: &others,
                        at,
                        pseudonym: [7; 16],
                    },
                )?;
                Ok(player)
            })
            .await
            .unwrap()
    }
}

fn staff(permissions: Permissions) -> Caller {
    Caller {
        session_id: 1,
        user_id: 2,
        username: "maxime".to_owned(),
        is_owner: false,
        permissions,
        must_change_password: false,
        reauthenticated_at: None,
        client: Client {
            ip: IpAddr::from([127, 0, 0, 1]),
            user_agent: None,
        },
    }
}

fn request(player: Option<i64>, identifiers: &[&str], minutes: Option<u32>) -> BanRequest {
    BanRequest {
        player,
        identifiers: identifiers.iter().map(|text| (*text).to_owned()).collect(),
        reason: "  Aimbot on stream ".to_owned(),
        minutes,
    }
}

fn refused(error: Error) -> ApiError {
    match error {
        Error::Refused(error) => error,
        other => panic!("not a refusal: {other:?}"),
    }
}

/// What a player shows when they connect, as the bridge reports it.
fn showing(identifiers: &[&str], tokens: &[String]) -> Seen {
    Seen::of(&Shown {
        identifiers: identifiers.iter().map(|text| (*text).to_owned()).collect(),
        tokens: tokens.to_vec(),
        name: Some("kaito\u{202E}".to_owned()),
        ..Shown::default()
    })
}

#[tokio::test]
async fn a_ban_of_a_known_player_holds_their_licence_identifiers_and_tokens() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let kaito = lab.kaito().await;

    let ban = bans
        .ban(
            &staff(Permissions::PLAYERS_BAN),
            request(Some(kaito), &[], Some(60)),
        )
        .await
        .unwrap();

    assert_eq!(ban.player, Some(kaito));
    assert_eq!(ban.name.as_deref(), Some("kaito"));
    assert_eq!(ban.license.as_deref(), Some(KAITO));
    assert_eq!((ban.identifiers, ban.tokens), (1, 2));
    assert_eq!(ban.reason, "Aimbot on stream");
    assert_eq!(ban.by, "maxime");
    assert_eq!(ban.state, BanState::Active);
    assert_eq!(
        ban.ends_at,
        Some("2026-11-09T21:00:00Z".parse::<Timestamp>().unwrap())
    );
    assert_eq!(rules::parse_ban_code(&ban.code), Some(ban.code.clone()));
    assert_eq!(bans.of_player(kaito).await.unwrap(), [ban]);
    task.close().await;
}

#[tokio::test]
async fn a_ban_needs_a_reason_a_length_and_something_to_hold() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let caller = staff(Permissions::PLAYERS_BAN);
    let field = |error: Error| refused(error).params["field"].clone();

    let blank = BanRequest {
        reason: " \u{200B} ".to_owned(),
        ..request(None, &[KAITO], None)
    };
    assert_eq!(field(bans.ban(&caller, blank).await.unwrap_err()), "reason");
    assert_eq!(
        field(
            bans.ban(&caller, request(None, &[KAITO], Some(0)))
                .await
                .unwrap_err()
        ),
        "minutes"
    );
    assert_eq!(
        field(
            bans.ban(&caller, request(None, &["steam:nonsense"], None))
                .await
                .unwrap_err()
        ),
        "identifiers"
    );
    assert_eq!(
        refused(
            bans.ban(&caller, request(Some(404), &[], None))
                .await
                .unwrap_err()
        )
        .code,
        ErrorCode::PlayerNotFound
    );
    let by_hand = bans
        .ban(&caller, request(None, &[KAITO, &token(9)], None))
        .await
        .unwrap();
    assert_eq!((by_hand.player, by_hand.ends_at), (None, None));
    assert_eq!((by_hand.identifiers, by_hand.tokens), (0, 1));
    task.close().await;
}

#[tokio::test]
async fn a_banned_player_is_refused_with_the_owners_message_and_counted() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let kaito = lab.kaito().await;
    let ban = bans
        .ban(
            &staff(Permissions::PLAYERS_BAN),
            request(Some(kaito), &[], Some(60)),
        )
        .await
        .unwrap();

    let (message, event) = bans.refusal(&showing(&[KAITO], &[]), None).await.unwrap();
    let stranger = bans
        .refusal(
            &showing(&[&format!("license:{}", "b".repeat(40))], &[token(5)]),
            None,
        )
        .await;

    assert_eq!(
        message,
        format!(
            "You are banned from this server.\nReason: Aimbot on stream\nEnds: 2026-11-09 22:00 (Europe/Paris)\nBan ID: {}",
            ban.code
        )
    );
    assert_eq!(event.ban, ban.code);
    assert_eq!(event.name.as_deref(), Some("kaito"));
    assert_eq!(event.license.as_deref(), Some(KAITO));
    assert_eq!(event.ends_at, Some(ban.ends_at.unwrap().as_millisecond()));
    assert_eq!(event.player, None);
    assert!(stranger.is_none());
    assert_eq!(bans.of_player(kaito).await.unwrap()[0].refused, 1);
    task.close().await;
}

#[tokio::test]
async fn tokens_alone_refuse_only_as_many_as_the_settings_ask() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let kaito = lab.kaito().await;
    let caller = staff(Permissions::PLAYERS_BAN | Permissions::SETTINGS_MANAGE);
    bans.ban(&caller, request(Some(kaito), &[], None)).await.unwrap();
    let other_account = showing(&[&format!("license:{}", "c".repeat(40))], &[token(1)]);

    assert!(bans.refusal(&other_account, None).await.is_some());
    bans.change_settings(
        &caller,
        BansSettings {
            token_matches: 2,
            ..BansSettings::default()
        },
    )
    .await
    .unwrap();
    assert!(bans.refusal(&other_account, None).await.is_none());
    assert!(
        bans.refusal(&showing(&[], &[token(1), token(2)]), Some(4))
            .await
            .is_some_and(|(_, event)| event.player == Some(4))
    );
    task.close().await;
}

#[tokio::test]
async fn a_lifted_ban_lets_the_player_in_and_lifts_once() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let caller = staff(Permissions::PLAYERS_BAN | Permissions::BANS_LIFT);
    let ban = bans.ban(&caller, request(None, &[KAITO], None)).await.unwrap();

    bans.lift(
        &caller,
        &ban.code.to_lowercase(),
        LiftRequest {
            reason: Some(" appeal accepted ".to_owned()),
        },
    )
    .await
    .unwrap();

    assert!(bans.refusal(&showing(&[KAITO], &[]), None).await.is_none());
    let again = bans.lift(&caller, &ban.code, LiftRequest { reason: None }).await;
    assert_eq!(refused(again.unwrap_err()).code, ErrorCode::BanNotActive);
    let unknown = bans
        .lift(&caller, "ZZZZ-ZZZZ", LiftRequest { reason: None })
        .await;
    assert_eq!(refused(unknown.unwrap_err()).code, ErrorCode::BanNotFound);
    let page = bans
        .page(BanFilter::Lifted, None, None, Permissions::BANS_VIEW)
        .await
        .unwrap();
    let lift = page.bans[0].lifted.clone().unwrap();
    assert_eq!(
        (lift.by.as_str(), lift.reason.as_deref()),
        ("maxime", Some("appeal accepted"))
    );
    assert_eq!((page.counts.active, page.counts.lifted), (0, 1));
    task.close().await;
}

#[tokio::test]
async fn an_ended_ban_refuses_no_one() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let caller = staff(Permissions::PLAYERS_BAN);
    bans.ban(&caller, request(None, &[KAITO], Some(30)))
        .await
        .unwrap();

    lab.clock.advance(SignedDuration::from_mins(31));

    assert!(bans.refusal(&showing(&[KAITO], &[]), None).await.is_none());
    let ended = bans
        .page(BanFilter::Ended, None, None, Permissions::BANS_VIEW)
        .await
        .unwrap();
    assert_eq!(ended.bans[0].state, BanState::Ended);
    task.close().await;
}

#[tokio::test]
async fn searching_by_another_identifier_takes_the_identifiers_permission() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let kaito = lab.kaito().await;
    let ban = bans
        .ban(&staff(Permissions::PLAYERS_BAN), request(Some(kaito), &[], None))
        .await
        .unwrap();
    let discord = format!("discord:{DISCORD}");

    let refusal = bans
        .page(BanFilter::All, Some(&discord), None, Permissions::BANS_VIEW)
        .await
        .unwrap_err();
    let found = bans
        .page(
            BanFilter::All,
            Some(&discord),
            None,
            Permissions::BANS_VIEW | Permissions::PLAYER_IDENTIFIERS_VIEW,
        )
        .await
        .unwrap();
    let by_licence = bans
        .page(BanFilter::All, Some(KAITO), None, Permissions::BANS_VIEW)
        .await
        .unwrap();

    assert_eq!(refused(refusal).params["permission"], "playerIdentifiersView");
    assert_eq!(found.bans[0].code, ban.code);
    assert_eq!(by_licence.bans[0].code, ban.code);
    assert_eq!(found.zone, "Europe/Paris");
    task.close().await;
}

#[tokio::test]
async fn the_settings_refuse_an_empty_message() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let caller = staff(Permissions::SETTINGS_MANAGE);

    let empty = bans
        .change_settings(
            &caller,
            BansSettings {
                message: "\n \n".to_owned(),
                token_matches: 1,
            },
        )
        .await
        .unwrap_err();
    bans.change_settings(
        &caller,
        BansSettings {
            message: "Banned: {reason}\n".to_owned(),
            token_matches: 3,
        },
    )
    .await
    .unwrap();

    assert_eq!(refused(empty).code, ErrorCode::InvalidBansSettings);
    assert_eq!(
        bans.settings().await.unwrap(),
        BansSettings {
            message: "Banned: {reason}".to_owned(),
            token_matches: 3,
        }
    );
    task.close().await;
}

#[test]
fn the_overlap_counts_tokens_apart() {
    let held = [
        ("license".to_owned(), "a".repeat(40)),
        ("token".to_owned(), token(1)),
        ("token".to_owned(), token(2)),
    ];
    let shown = [
        ("license".to_owned(), "a".repeat(40)),
        ("token".to_owned(), token(2)),
        ("token".to_owned(), token(3)),
    ];

    assert_eq!(overlap(&held, &shown), (1, 1));
}
