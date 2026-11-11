use std::net::{IpAddr, SocketAddr};

use observer_domain::bans::{BanEdit, BanEndChange, BanReasonChange, BanState};
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

fn change(reason: Option<&str>, ends: Option<BanEnd>) -> BanEditRequest {
    BanEditRequest {
        reason: reason.map(str::to_owned),
        ends,
    }
}

fn template(reason: &str, minutes: Option<u32>) -> BanTemplate {
    BanTemplate {
        reason: reason.to_owned(),
        minutes,
    }
}

fn time(text: &str) -> Timestamp {
    text.parse().unwrap()
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
async fn a_ban_is_told_as_txadmin_tells_one_with_what_it_holds_and_who_it_drops() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let kaito = lab.kaito().await;
    let two_days = Some(2 * 24 * 60);
    let ban = bans
        .ban(
            &staff(Permissions::PLAYERS_BAN),
            request(Some(kaito), &[], two_days),
        )
        .await
        .unwrap();
    let held = [
        ("discord".to_owned(), DISCORD.to_owned()),
        (
            "license".to_owned(),
            KAITO.trim_start_matches("license:").to_owned(),
        ),
        ("token".to_owned(), token(1)),
        ("token".to_owned(), token(2)),
    ];
    let second_account = showing(&[&format!("license:{}", "c".repeat(40))], &[token(1)]);
    let dropped = [(4, second_account.clone()), (9, showing(&[KAITO], &[]))];
    let sharing = [(7, second_account.clone()), (4, second_account)];
    let by_hand = Ban {
        name: None,
        license: None,
        ends_at: None,
        ..ban.clone()
    };
    let settings = BansSettings::default();

    let told = bans.txadmin_ban(&ban, two_days, &held, &dropped, &settings);
    let nobody = bans.txadmin_ban(&by_hand, None, &held[..1], &[], &settings);
    let shared = bans.txadmin_ban(&by_hand, None, &held, &sharing, &settings);

    assert_eq!(
        (told.author.as_str(), told.reason.as_str()),
        ("maxime", "Aimbot on stream")
    );
    assert_eq!(told.action_id, ban.code);
    assert_eq!(told.expiration, ban.ends_at.map(Timestamp::as_second));
    assert_eq!(
        (told.duration_input.as_str(), told.duration_translated.as_deref()),
        ("2 days", Some("2 days"))
    );
    assert_eq!(
        told.target_net_id,
        Some(9),
        "the player banned, not the account that shares a token"
    );
    assert_eq!(told.target_ids, [KAITO.to_owned(), format!("discord:{DISCORD}")]);
    assert_eq!(told.target_hwids, [token(1), token(2)]);
    assert_eq!(told.target_name, "kaito");
    assert_eq!(
        told.kick_message,
        format!(
            "You are banned from this server.\nReason: Aimbot on stream\nEnds: 2026-11-11 21:00 (Europe/Paris)\nBan ID: {}",
            ban.code
        )
    );
    assert_eq!(nobody.expiration, None);
    assert_eq!(
        (
            nobody.duration_input.as_str(),
            nobody.duration_translated,
            nobody.target_net_id
        ),
        ("permanent", None, None)
    );
    assert_eq!(
        shared.target_net_id,
        Some(4),
        "the lowest id among the players dropped, whatever their order"
    );
    assert_eq!(nobody.target_ids, [format!("discord:{DISCORD}")]);
    assert_eq!(nobody.target_hwids, Vec::<String>::new());
    assert_eq!(nobody.target_name, "identifiers");
    task.close().await;
}

#[tokio::test]
async fn a_lifted_ban_is_told_as_txadmin_tells_an_action_revoked_with_what_the_ban_holds() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let kaito = lab.kaito().await;
    let ban = bans
        .ban(&staff(Permissions::PLAYERS_BAN), request(Some(kaito), &[], None))
        .await
        .unwrap();
    let code = ban.code.clone();
    let held = lab.db.read(move |conn| stored::held(conn, &code)).await.unwrap();
    let by_hand = Ban {
        name: None,
        ..ban.clone()
    };

    let told = txadmin_revocation(&ban, &held, "sarah");
    let unnamed = serde_json::to_value(txadmin_revocation(&by_hand, &held, "sarah")).unwrap();

    assert_eq!(told.action_id, ban.code);
    assert_eq!(told.action_type, TxAdminAction::Ban);
    assert_eq!(
        (
            told.action_reason.as_str(),
            told.action_author.as_str(),
            told.revoked_by.as_str()
        ),
        ("Aimbot on stream", "maxime", "sarah")
    );
    assert_eq!(told.player_name.as_deref(), Some("kaito"));
    assert_eq!(told.player_ids, [KAITO.to_owned(), format!("discord:{DISCORD}")]);
    assert_eq!(told.player_hwids, [token(1), token(2)]);
    assert_eq!(unnamed["playerName"], json!(false));
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
async fn an_edit_gives_the_ban_a_new_reason_and_end_and_keeps_what_they_replace() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let kaito = lab.kaito().await;
    let caller = staff(Permissions::PLAYERS_BAN | Permissions::BANS_EDIT);
    let ban = bans
        .ban(&caller, request(Some(kaito), &[], Some(60)))
        .await
        .unwrap();
    lab.clock.advance(SignedDuration::from_mins(10));

    let edited = bans
        .edit(
            &caller,
            &ban.code.to_lowercase(),
            change(Some("  Aimbot, two clips "), Some(BanEnd::Never)),
        )
        .await
        .unwrap();

    assert_eq!(edited.reason, "Aimbot, two clips");
    assert_eq!((edited.ends_at, edited.state), (None, BanState::Active));
    assert_eq!(
        (edited.by.as_str(), edited.created_at),
        ("maxime", ban.created_at)
    );
    assert_eq!(
        edited.edits,
        [BanEdit {
            at: time("2026-11-09T20:10:00Z"),
            by: "maxime".to_owned(),
            reason: Some(BanReasonChange {
                before: "Aimbot on stream".to_owned(),
                after: "Aimbot, two clips".to_owned(),
            }),
            ends: Some(BanEndChange {
                before: Some(time("2026-11-09T21:00:00Z")),
                after: None,
            }),
        }]
    );
    assert_eq!(bans.of_player(kaito).await.unwrap(), [edited]);
    task.close().await;
}

#[tokio::test]
async fn a_connecting_player_is_checked_against_the_ban_as_it_was_edited() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let caller = staff(Permissions::PLAYERS_BAN | Permissions::BANS_EDIT);
    let ban = bans.ban(&caller, request(None, &[KAITO], None)).await.unwrap();
    bans.edit(
        &caller,
        &ban.code,
        change(Some("Ban evasion"), Some(BanEnd::In { minutes: 30 })),
    )
    .await
    .unwrap();

    let (message, event) = bans.refusal(&showing(&[KAITO], &[]), None).await.unwrap();
    lab.clock.advance(SignedDuration::from_mins(31));
    let once_ended = bans.refusal(&showing(&[KAITO], &[]), None).await;

    assert_eq!(
        message,
        format!(
            "You are banned from this server.\nReason: Ban evasion\nEnds: 2026-11-09 21:30 (Europe/Paris)\nBan ID: {}",
            ban.code
        )
    );
    assert_eq!(event.reason, "Ban evasion");
    assert_eq!(event.ends_at, Some(time("2026-11-09T20:30:00Z").as_millisecond()));
    assert!(once_ended.is_none());
    task.close().await;
}

#[tokio::test]
async fn an_edit_pushes_the_end_of_a_ban_back_or_brings_it_closer() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let caller = staff(Permissions::BANS_EDIT);
    let ban = bans
        .ban(
            &staff(Permissions::PLAYERS_BAN),
            request(None, &[KAITO], Some(24 * 60)),
        )
        .await
        .unwrap();
    let ends = |end: BanEnd| change(None, Some(end));

    let later = bans
        .edit(&caller, &ban.code, ends(BanEnd::In { minutes: 48 * 60 }))
        .await
        .unwrap();
    let for_good = bans.edit(&caller, &ban.code, ends(BanEnd::Never)).await.unwrap();
    let sooner = bans
        .edit(&caller, &ban.code, ends(BanEnd::In { minutes: 60 }))
        .await
        .unwrap();

    assert_eq!(later.ends_at, Some(time("2026-11-11T20:00:00Z")));
    assert_eq!(for_good.ends_at, None);
    assert_eq!(sooner.ends_at, Some(time("2026-11-09T21:00:00Z")));
    assert_eq!(sooner.reason, ban.reason);
    assert_eq!(
        sooner.edits.iter().map(|edit| edit.ends).collect::<Vec<_>>(),
        [
            Some(BanEndChange {
                before: None,
                after: sooner.ends_at,
            }),
            Some(BanEndChange {
                before: later.ends_at,
                after: None,
            }),
            Some(BanEndChange {
                before: ban.ends_at,
                after: later.ends_at,
            }),
        ]
    );
    task.close().await;
}

#[tokio::test]
async fn only_an_active_ban_is_edited_and_an_edit_that_changes_nothing_is_not_kept() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let caller = staff(Permissions::PLAYERS_BAN | Permissions::BANS_EDIT | Permissions::BANS_LIFT);
    let ended = bans
        .ban(&caller, request(None, &[KAITO], Some(30)))
        .await
        .unwrap();
    let lifted = bans
        .ban(&caller, request(None, &[&token(3)], None))
        .await
        .unwrap();
    let active = bans
        .ban(&caller, request(None, &[&token(4)], None))
        .await
        .unwrap();
    bans.lift(&caller, &lifted.code, LiftRequest { reason: None })
        .await
        .unwrap();
    lab.clock.advance(SignedDuration::from_mins(31));
    let hour = || change(None, Some(BanEnd::In { minutes: 60 }));
    let code = |error: Error| refused(error).code;
    let field = |error: Error| refused(error).params["field"].clone();

    for (ban, expected) in [
        (ended.code.as_str(), ErrorCode::BanNotActive),
        (lifted.code.as_str(), ErrorCode::BanNotActive),
        ("ZZZZ-ZZZZ", ErrorCode::BanNotFound),
        ("not a code", ErrorCode::BanNotFound),
    ] {
        let refusal = bans.edit(&caller, ban, hour()).await.unwrap_err();
        assert_eq!(code(refusal), expected, "{ban}");
    }
    let blank = bans.edit(&caller, &active.code, change(Some(" \u{200B} "), None));
    assert_eq!(field(blank.await.unwrap_err()), "reason");
    let no_time = bans.edit(
        &caller,
        &active.code,
        change(None, Some(BanEnd::In { minutes: 0 })),
    );
    assert_eq!(field(no_time.await.unwrap_err()), "minutes");
    let same = bans
        .edit(
            &caller,
            &active.code,
            change(Some(" Aimbot on stream "), Some(BanEnd::Never)),
        )
        .await
        .unwrap();
    let nothing = bans
        .edit(&caller, &active.code, change(None, None))
        .await
        .unwrap();
    assert_eq!(same.edits, []);
    assert_eq!(nothing, same);
    task.close().await;
}

#[tokio::test]
async fn the_settings_keep_the_templates_in_order_and_refuse_one_no_ban_could_be_made_from() {
    let lab = Lab::new().await;
    let (bans, task) = lab.bans();
    let caller = staff(Permissions::SETTINGS_MANAGE);
    let with = |templates: Vec<BanTemplate>| BansSettings {
        templates,
        ..BansSettings::default()
    };

    let unusable = bans
        .change_settings(&caller, with(vec![template("Cheating", Some(0))]))
        .await
        .unwrap_err();
    bans.change_settings(
        &caller,
        with(vec![
            template(" Cheating ", None),
            template("Insults", Some(3 * 24 * 60)),
            template("Cheating", None),
        ]),
    )
    .await
    .unwrap();

    let refusal = refused(unusable);
    assert_eq!(refusal.code, ErrorCode::InvalidBansSettings);
    assert_eq!(refusal.params["maxTemplates"], MAX_BAN_TEMPLATES);
    let kept = [template("Cheating", None), template("Insults", Some(3 * 24 * 60))];
    assert_eq!(bans.templates().await.unwrap(), kept);
    assert_eq!(bans.settings().await.unwrap().templates, kept);
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
                ..BansSettings::default()
            },
        )
        .await
        .unwrap_err();
    bans.change_settings(
        &caller,
        BansSettings {
            message: "Banned: {reason}\n".to_owned(),
            token_matches: 3,
            templates: Vec::new(),
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
            templates: Vec::new(),
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
