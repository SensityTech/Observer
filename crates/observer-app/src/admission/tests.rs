use std::net::{IpAddr, SocketAddr};

use jiff::SignedDuration;
use observer_domain::allowlist::{
    self as rules, AllowlistSettings, ApproveRequest, DenyRequests, RequestPage,
};
use observer_domain::audit::{AuditAction, AuditEntry, AuditFilter};
use observer_domain::bans::BanRequest;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::maintenance::{AddPass, MaintenanceSettings, MaintenanceStatus, SetMaintenance};
use observer_domain::permissions::Permissions;
use observer_domain::schedule::ScheduleSettings;
use observer_fxserver::bridge::{AllowlistEvent, MaintenanceEvent};
use observer_fxserver::players::Shown;
use tempfile::TempDir;

use super::*;
use crate::{Accounts, AuditLog, BansSetup, BansTask, Caller, Client, PasswordCost, Running, Servers, Setup};

const KAITO: &str = "license:00000000000000000000000000000000000000aa";
const MIRA: &str = "license:00000000000000000000000000000000000000bb";

/// The state database in a temporary folder, with the bans and the admission over it.
struct Lab {
    _dir: TempDir,
    db: Database,
    clock: Clock,
    supervision: Supervision,
    audit: AuditLog,
    bans: Bans,
    admission: Admission,
    tasks: (BansTask, AdmissionTask),
    _running: Running,
}

fn schedule() -> watch::Receiver<ScheduleStatus> {
    watch::channel(ScheduleStatus {
        settings: ScheduleSettings::default(),
        version: None,
        zone: "Europe/Paris".to_owned(),
        next: None,
        upcoming: Vec::new(),
        one_off: None,
    })
    .1
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
                panel: SocketAddr::from(([127, 0, 0, 1], 41976)),
                artifacts: data_dir.join("artifacts"),
                console_logs: data_dir.join("logs/console"),
                log: None,
            },
            |_| {},
        )
        .await
        .unwrap();
        let (bans, banning) = Bans::start(BansSetup {
            db: db.clone(),
            clock: clock.clone(),
            supervision: supervision.clone(),
            schedule: schedule(),
        });
        let (admission, admitting) = Admission::start(AdmissionSetup {
            db: db.clone(),
            clock: clock.clone(),
            supervision: supervision.clone(),
            bans: bans.clone(),
            schedule: schedule(),
        })
        .await
        .unwrap();
        Self {
            _dir: dir,
            audit: AuditLog::new(db.clone(), clock.clone()),
            db,
            clock,
            supervision,
            bans,
            admission,
            tasks: (banning, admitting),
            _running: running,
        }
    }

    /// Turns the allowlist on, with a message that shows what its placeholders gave.
    async fn on(&self) {
        self.admission
            .change_allowlist_settings(
                &staff(),
                AllowlistSettings {
                    enabled: true,
                    message: "Hello {name}, ask the staff: {id}".to_owned(),
                },
            )
            .await
            .unwrap();
    }

    /// Closes the server for maintenance, or opens it again.
    async fn maintenance(&self, on: bool) {
        self.admission
            .set_maintenance(&staff(), SetMaintenance { on })
            .await
            .unwrap();
    }

    /// Puts a licence on the maintenance list, and gives its pass's id.
    async fn pass(&self, license: &str, note: Option<&str>) -> Result<i64, Error> {
        let request = AddPass {
            license: license.to_owned(),
            note: note.map(str::to_owned),
        };
        Ok(self.admission.add_pass(&staff(), request).await?.id)
    }

    async fn audited(&self, action: AuditAction) -> Vec<AuditEntry> {
        let filter = AuditFilter {
            action: Some(action),
            ..AuditFilter::default()
        };
        self.audit.page(filter).await.unwrap().entries
    }

    /// The request's code a refused player was given, read from what the allowlist refused.
    async fn refused(&self, license: &str, name: &str) -> String {
        let refusal = self
            .admission
            .refusal(&showing(&[license], name))
            .await
            .expect("the player got in");
        let Refused::Allowlist(event) = refusal.event else {
            panic!("not the allowlist's refusal: {refusal:?}");
        };
        event.request.expect("the refusal carries no request")
    }

    async fn close(self) {
        self.tasks.1.close().await;
        self.tasks.0.close().await;
    }
}

fn staff() -> Caller {
    Caller {
        session_id: 1,
        user_id: 2,
        username: "maxime".to_owned(),
        is_owner: false,
        permissions: Permissions::ALLOWLIST_MANAGE | Permissions::SETTINGS_MANAGE | Permissions::PLAYERS_BAN,
        must_change_password: false,
        reauthenticated_at: None,
        client: Client {
            ip: IpAddr::from([127, 0, 0, 1]),
            user_agent: None,
        },
    }
}

/// What a player shows when they connect, as the bridge reports it.
fn showing(identifiers: &[&str], name: &str) -> Seen {
    Seen::of(&Shown {
        identifiers: identifiers.iter().map(|text| (*text).to_owned()).collect(),
        name: Some(name.to_owned()),
        ..Shown::default()
    })
}

fn code(error: Error) -> ErrorCode {
    refused(error).code
}

fn refused(error: Error) -> ApiError {
    match error {
        Error::Refused(error) => error,
        other => panic!("not a refusal: {other:?}"),
    }
}

#[tokio::test]
async fn while_the_allowlist_is_off_everyone_gets_in_and_asks_nothing() {
    let lab = Lab::new().await;

    let verdict = lab.admission.refusal(&showing(&[KAITO], "kaito")).await;

    assert_eq!(verdict, None);
    assert_eq!(lab.admission.requests(None, None).await.unwrap().requests, []);
    assert!(!lab.admission.status().borrow().allowlist.enabled);
    lab.close().await;
}

#[tokio::test]
async fn a_player_who_is_not_approved_is_refused_with_the_owners_message_and_their_request() {
    let lab = Lab::new().await;
    lab.on().await;

    let refusal = lab
        .admission
        .refusal(&showing(&[KAITO], "kaito\u{202E}"))
        .await
        .unwrap();
    lab.clock.advance(SignedDuration::from_mins(5));
    let again = lab.refused(KAITO, "kaito").await;
    let page = lab.admission.requests(None, None).await.unwrap();

    let request = &page.requests[0];
    assert_eq!(page.requests.len(), 1);
    assert_eq!(request.code, again);
    assert_eq!(
        refusal.message,
        format!("Hello kaito, ask the staff: {}", request.code)
    );
    assert_eq!(
        refusal.event,
        Refused::Allowlist(AllowlistEvent {
            request: Some(request.code.clone()),
            name: Some("kaito".to_owned()),
            license: Some(KAITO.to_owned()),
        })
    );
    assert_eq!(
        rules::parse_request_code(&request.code),
        Some(request.code.clone())
    );
    assert_eq!((request.license.as_str(), request.tries), (KAITO, 2));
    assert_eq!(
        request.last_at.duration_since(request.first_at),
        SignedDuration::from_mins(5)
    );
    assert_eq!(page.zone, "Europe/Paris");
    assert_eq!(
        *lab.admission.status().borrow(),
        AdmissionStatus {
            allowlist: AllowlistStatus {
                enabled: true,
                waiting: 1,
                approved: 0
            },
            maintenance: MaintenanceStatus::default(),
        }
    );
    lab.close().await;
}

#[tokio::test]
async fn a_player_without_a_licence_is_refused_and_asks_nothing() {
    let lab = Lab::new().await;
    lab.on().await;

    let refusal = lab
        .admission
        .refusal(&showing(&["discord:123456789012345678"], "lan"))
        .await
        .unwrap();

    assert_eq!(refusal.message, "Hello lan, ask the staff: none");
    assert_eq!(
        refusal.event,
        Refused::Allowlist(AllowlistEvent {
            request: None,
            name: Some("lan".to_owned()),
            license: None,
        })
    );
    assert_eq!(lab.admission.requests(None, None).await.unwrap().requests, []);
    lab.close().await;
}

#[tokio::test]
async fn approving_a_request_lets_its_player_in_and_is_audited() {
    let lab = Lab::new().await;
    lab.on().await;
    let request = lab.refused(KAITO, "kaito").await;
    lab.refused(MIRA, "Mira").await;

    let approval = lab
        .admission
        .approve_request(&staff(), &request.to_lowercase())
        .await
        .unwrap();

    assert_eq!(lab.admission.refusal(&showing(&[KAITO], "kaito")).await, None);
    assert!(lab.admission.refusal(&showing(&[MIRA], "Mira")).await.is_some());
    assert_eq!(approval.license, KAITO);
    assert_eq!(approval.name.as_deref(), Some("kaito"));
    assert_eq!((approval.by.as_str(), approval.note.as_deref()), ("maxime", None));
    assert_eq!(approval.request.as_deref(), Some(request.as_str()));
    let approved = lab.admission.approved(None, None).await.unwrap();
    assert_eq!(approved.approved, [approval]);
    let again = lab.admission.approve_request(&staff(), &request).await;
    assert_eq!(code(again.unwrap_err()), ErrorCode::AllowlistRequestNotFound);
    let status = lab.admission.status().borrow().clone();
    assert_eq!((status.allowlist.waiting, status.allowlist.approved), (1, 1));
    let audited = lab
        .audit
        .page(AuditFilter {
            action: Some(AuditAction::AllowlistRequestApproved),
            ..AuditFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(audited.entries.len(), 1);
    assert_eq!(audited.entries[0].target.as_deref(), Some(request.as_str()));
    assert_eq!(audited.entries[0].details["license"], KAITO);
    lab.close().await;
}

#[tokio::test]
async fn a_denied_player_asks_again_under_a_new_code() {
    let lab = Lab::new().await;
    lab.on().await;
    let first = lab.refused(KAITO, "kaito").await;

    lab.admission.deny_request(&staff(), &first).await.unwrap();
    let second = lab.refused(KAITO, "kaito").await;

    assert_ne!(first, second);
    let twice = lab.admission.deny_request(&staff(), &first).await;
    assert_eq!(code(twice.unwrap_err()), ErrorCode::AllowlistRequestNotFound);
    let unreadable = lab.admission.deny_request(&staff(), "not a code").await;
    assert_eq!(code(unreadable.unwrap_err()), ErrorCode::AllowlistRequestNotFound);
    assert_eq!(
        lab.admission.requests(None, None).await.unwrap().requests.len(),
        1
    );
    lab.close().await;
}

#[tokio::test]
async fn denying_all_leaves_the_requests_made_since_the_staff_looked() {
    let lab = Lab::new().await;
    lab.on().await;
    lab.refused(KAITO, "kaito").await;
    let seen = lab.clock.now();
    lab.clock.advance(SignedDuration::from_secs(30));
    let late = lab.refused(MIRA, "Mira").await;

    let denied = lab
        .admission
        .deny_requests(&staff(), DenyRequests { until: seen })
        .await
        .unwrap();
    let nothing = lab
        .admission
        .deny_requests(&staff(), DenyRequests { until: seen })
        .await
        .unwrap();

    assert_eq!((denied.denied, nothing.denied), (1, 0));
    let left = lab.admission.requests(None, None).await.unwrap().requests;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].code, late);
    let audited = lab
        .audit
        .page(AuditFilter {
            action: Some(AuditAction::AllowlistRequestsDenied),
            ..AuditFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(audited.entries.len(), 1);
    assert_eq!(audited.entries[0].details["denied"], 1);
    lab.close().await;
}

#[tokio::test]
async fn a_licence_approved_ahead_gets_in_and_takes_its_waiting_request_away() {
    let lab = Lab::new().await;
    lab.on().await;
    lab.refused(KAITO, "kaito").await;
    let ahead = |license: &str, note: Option<&str>| ApproveRequest {
        license: license.to_owned(),
        note: note.map(str::to_owned),
    };
    let field = |error: Error| refused(error).params["field"].clone();

    let approval = lab
        .admission
        .approve(
            &staff(),
            ahead(
                &KAITO["license:".len()..].to_uppercase(),
                Some("  Sarah's brother "),
            ),
        )
        .await
        .unwrap();

    assert_eq!(approval.license, KAITO);
    assert_eq!(approval.note.as_deref(), Some("Sarah's brother"));
    assert_eq!((approval.name, approval.request), (None, None));
    assert_eq!(lab.admission.refusal(&showing(&[KAITO], "kaito")).await, None);
    assert_eq!(lab.admission.requests(None, None).await.unwrap().requests, []);
    let twice = lab.admission.approve(&staff(), ahead(KAITO, None)).await;
    assert_eq!(code(twice.unwrap_err()), ErrorCode::AlreadyApproved);
    let no_licence = lab.admission.approve(&staff(), ahead("kaito", None)).await;
    assert_eq!(field(no_licence.unwrap_err()), "license");
    let long = lab
        .admission
        .approve(&staff(), ahead(MIRA, Some(&"n".repeat(65))))
        .await;
    assert_eq!(field(long.unwrap_err()), "note");
    lab.close().await;
}

#[tokio::test]
async fn an_approval_taken_back_keeps_its_player_out_again() {
    let lab = Lab::new().await;
    lab.on().await;
    let approval = lab
        .admission
        .approve(
            &staff(),
            ApproveRequest {
                license: KAITO.to_owned(),
                note: None,
            },
        )
        .await
        .unwrap();

    lab.admission
        .remove_approval(&staff(), approval.id)
        .await
        .unwrap();

    assert!(lab.admission.refusal(&showing(&[KAITO], "kaito")).await.is_some());
    let twice = lab.admission.remove_approval(&staff(), approval.id).await;
    assert_eq!(code(twice.unwrap_err()), ErrorCode::ApprovalNotFound);
    assert_eq!(lab.admission.status().borrow().allowlist.approved, 0);
    lab.close().await;
}

#[tokio::test]
async fn a_ban_refuses_before_the_allowlist_and_an_approved_player_stays_banned() {
    let lab = Lab::new().await;
    lab.on().await;
    lab.admission
        .approve(
            &staff(),
            ApproveRequest {
                license: KAITO.to_owned(),
                note: None,
            },
        )
        .await
        .unwrap();
    let mut bans = Vec::new();
    for license in [KAITO, MIRA] {
        let request = BanRequest {
            player: None,
            identifiers: vec![license.to_owned()],
            reason: "Aimbot".to_owned(),
            minutes: None,
        };
        bans.push(lab.bans.ban(&staff(), request).await.unwrap().code);
    }

    let approved = lab.admission.refusal(&showing(&[KAITO], "kaito")).await.unwrap();
    let stranger = lab.admission.refusal(&showing(&[MIRA], "Mira")).await.unwrap();

    for (refusal, ban) in [approved, stranger].into_iter().zip(bans) {
        let Refused::Ban(event) = refusal.event else {
            panic!("the allowlist answered before the ban: {refusal:?}");
        };
        assert_eq!(event.ban, ban);
    }
    assert_eq!(lab.admission.requests(None, None).await.unwrap().requests, []);
    lab.close().await;
}

#[tokio::test]
async fn the_settings_refuse_an_empty_message_and_a_change_is_audited_once() {
    let lab = Lab::new().await;
    let turned_on = AllowlistSettings {
        enabled: true,
        ..AllowlistSettings::default()
    };

    let empty = lab
        .admission
        .change_allowlist_settings(
            &staff(),
            AllowlistSettings {
                enabled: true,
                message: " \n ".to_owned(),
            },
        )
        .await;
    for _ in 0..2 {
        lab.admission
            .change_allowlist_settings(&staff(), turned_on.clone())
            .await
            .unwrap();
    }

    assert_eq!(code(empty.unwrap_err()), ErrorCode::InvalidAllowlistSettings);
    assert_eq!(lab.admission.allowlist_settings().await.unwrap(), turned_on);
    assert!(lab.admission.status().borrow().allowlist.enabled);
    let audited = lab
        .audit
        .page(AuditFilter {
            action: Some(AuditAction::AllowlistSettingsChanged),
            ..AuditFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(audited.entries.len(), 1);
    assert_eq!(audited.entries[0].details["to"]["enabled"], true);
    lab.close().await;
}

#[tokio::test]
async fn the_lists_are_searched_by_code_name_or_licence_and_refuse_an_address() {
    let lab = Lab::new().await;
    lab.on().await;
    let kaito = lab.refused(KAITO, "kaito").await;
    lab.refused(MIRA, "Mira").await;
    let codes = |page: RequestPage| -> Vec<String> {
        page.requests.into_iter().map(|request| request.code).collect()
    };

    let by_code = lab.admission.requests(Some(&kaito.replace('-', "")), None).await;
    let by_name = lab.admission.requests(Some("kai"), None).await;
    let by_license = lab.admission.requests(Some(KAITO), None).await;
    let address = lab.admission.requests(Some("ip:192.0.2.1"), None).await;
    let other = lab
        .admission
        .approved(Some("discord:123456789012345678"), None)
        .await;
    let cursor = lab.admission.requests(None, Some("nonsense")).await;

    for found in [by_code, by_name, by_license] {
        assert_eq!(codes(found.unwrap()), std::slice::from_ref(&kaito));
    }
    assert_eq!(code(address.unwrap_err()), ErrorCode::AddressSearch);
    assert_eq!(code(other.unwrap_err()), ErrorCode::InvalidSearch);
    assert_eq!(code(cursor.unwrap_err()), ErrorCode::InvalidRequest);
    lab.close().await;
}

#[tokio::test]
async fn maintenance_refuses_everyone_but_its_list_with_the_owners_message() {
    let lab = Lab::new().await;
    lab.on().await;
    lab.admission
        .change_maintenance_settings(
            &staff(),
            MaintenanceSettings {
                message: "Back at 8, {name}.".to_owned(),
                server_name: None,
            },
        )
        .await
        .unwrap();
    lab.pass(KAITO, None).await.unwrap();

    lab.maintenance(true).await;
    let listed = lab.admission.refusal(&showing(&[KAITO], "kaito")).await;
    let stranger = lab.admission.refusal(&showing(&[MIRA], "Mira")).await.unwrap();
    let unlicensed = lab
        .admission
        .refusal(&showing(&["discord:123456789012345678"], "lan"))
        .await
        .unwrap();
    let during = lab.admission.status().borrow().maintenance.clone();
    lab.maintenance(false).await;
    let after = lab.admission.refusal(&showing(&[KAITO], "kaito")).await.unwrap();

    assert_eq!(listed, None, "a player on the list gets in, approved or not");
    assert_eq!(stranger.message, "Back at 8, Mira.");
    assert_eq!(
        stranger.event,
        Refused::Maintenance(MaintenanceEvent {
            name: Some("Mira".to_owned()),
            license: Some(MIRA.to_owned()),
        })
    );
    assert_eq!(
        unlicensed.event,
        Refused::Maintenance(MaintenanceEvent {
            name: Some("lan".to_owned()),
            license: None,
        })
    );
    assert_eq!(
        during,
        MaintenanceStatus {
            on: true,
            since: Some(lab.clock.now()),
            by: Some("maxime".to_owned()),
            passes: 1,
        }
    );
    assert!(
        matches!(after.event, Refused::Allowlist(_)),
        "the allowlist has its say again: {after:?}"
    );
    assert_eq!(
        lab.admission.status().borrow().maintenance,
        MaintenanceStatus {
            passes: 1,
            ..MaintenanceStatus::default()
        }
    );
    assert_eq!(
        lab.admission.requests(None, None).await.unwrap().requests.len(),
        1,
        "maintenance made no request: only the allowlist's refusal did"
    );
    lab.close().await;
}

#[tokio::test]
async fn a_ban_refuses_before_maintenance_even_on_its_list() {
    let lab = Lab::new().await;
    lab.pass(KAITO, None).await.unwrap();
    lab.maintenance(true).await;
    let request = BanRequest {
        player: None,
        identifiers: vec![KAITO.to_owned()],
        reason: "Aimbot".to_owned(),
        minutes: None,
    };
    let ban = lab.bans.ban(&staff(), request).await.unwrap().code;

    let refusal = lab.admission.refusal(&showing(&[KAITO], "kaito")).await.unwrap();

    let Refused::Ban(event) = refusal.event else {
        panic!("maintenance answered before the ban: {refusal:?}");
    };
    assert_eq!(event.ban, ban);
    lab.close().await;
}

#[tokio::test]
async fn maintenance_is_audited_once_a_switch_and_holds_through_a_restart_of_observer() {
    let lab = Lab::new().await;
    let start = lab.clock.now();

    for _ in 0..2 {
        lab.maintenance(true).await;
    }
    lab.clock.advance(SignedDuration::from_mins(40));
    let (again, task) = Admission::start(AdmissionSetup {
        db: lab.db.clone(),
        clock: lab.clock.clone(),
        supervision: lab.supervision.clone(),
        bans: lab.bans.clone(),
        schedule: schedule(),
    })
    .await
    .unwrap();
    let restarted = again.status().borrow().maintenance.clone();
    let refused = again.refusal(&showing(&[MIRA], "Mira")).await;
    task.close().await;
    for _ in 0..2 {
        lab.maintenance(false).await;
    }

    assert_eq!(
        (restarted.on, restarted.since, restarted.by.as_deref()),
        (true, Some(start), Some("maxime"))
    );
    assert!(refused.is_some());
    assert_eq!(lab.admission.refusal(&showing(&[MIRA], "Mira")).await, None);
    let started = lab.audited(AuditAction::MaintenanceStarted).await;
    let ended = lab.audited(AuditAction::MaintenanceEnded).await;
    assert_eq!((started.len(), ended.len()), (1, 1));
    assert_eq!(started[0].target.as_deref(), Some("default"));
    assert_eq!(started[0].details["passes"], 0);
    assert_eq!(ended[0].details["since"], serde_json::json!(start));
    lab.close().await;
}

#[tokio::test]
async fn the_maintenance_list_takes_a_licence_once_with_its_note_and_gives_it_back_by_its_id() {
    let lab = Lab::new().await;
    let field = |error: Error| refused(error).params["field"].clone();

    let id = lab
        .pass(
            &KAITO["license:".len()..].to_uppercase(),
            Some("  Tester from the dev team "),
        )
        .await
        .unwrap();
    let twice = lab.pass(KAITO, None).await;
    let no_licence = lab.pass("kaito", None).await;
    let long = lab.pass(MIRA, Some(&"n".repeat(65))).await;
    let listed = lab.admission.passes().await.unwrap();
    lab.admission.remove_pass(&staff(), id).await.unwrap();
    let gone = lab.admission.remove_pass(&staff(), id).await;

    assert_eq!(code(twice.unwrap_err()), ErrorCode::PassExists);
    assert_eq!(field(no_licence.unwrap_err()), "license");
    assert_eq!(field(long.unwrap_err()), "note");
    assert_eq!(listed.zone, "Europe/Paris");
    assert_eq!(listed.passes.len(), 1);
    let pass = &listed.passes[0];
    assert_eq!((pass.id, pass.license.as_str()), (id, KAITO));
    assert_eq!(pass.note.as_deref(), Some("Tester from the dev team"));
    assert_eq!((pass.by.as_str(), pass.added_at), ("maxime", lab.clock.now()));
    assert_eq!(code(gone.unwrap_err()), ErrorCode::PassNotFound);
    assert_eq!(lab.admission.passes().await.unwrap().passes, []);
    assert_eq!(lab.admission.status().borrow().maintenance.passes, 0);
    let added = lab.audited(AuditAction::MaintenancePassAdded).await;
    let removed = lab.audited(AuditAction::MaintenancePassRemoved).await;
    assert_eq!((added.len(), removed.len()), (1, 1));
    assert_eq!(added[0].details["license"], KAITO);
    assert_eq!(removed[0].details["note"], "Tester from the dev team");
    lab.close().await;
}

#[tokio::test]
async fn the_maintenance_message_refuses_an_empty_one_and_a_change_is_audited_once() {
    let lab = Lab::new().await;
    let message = MaintenanceSettings {
        message: "Back tonight.".to_owned(),
        server_name: Some("Aperture RP | maintenance".to_owned()),
    };

    let empty = lab
        .admission
        .change_maintenance_settings(
            &staff(),
            MaintenanceSettings {
                message: " \n ".to_owned(),
                server_name: None,
            },
        )
        .await;
    let long_name = lab
        .admission
        .change_maintenance_settings(
            &staff(),
            MaintenanceSettings {
                server_name: Some("n".repeat(41)),
                ..message.clone()
            },
        )
        .await;
    for _ in 0..2 {
        lab.admission
            .change_maintenance_settings(&staff(), message.clone())
            .await
            .unwrap();
    }

    assert_eq!(code(empty.unwrap_err()), ErrorCode::InvalidMaintenanceSettings);
    assert_eq!(refused(long_name.unwrap_err()).params["maxName"], 40);
    assert_eq!(lab.admission.maintenance_settings().await.unwrap(), message);
    let preview = lab.admission.maintenance_preview().await.unwrap();
    assert_eq!(preview.message, "Back tonight.");
    assert_eq!(preview.server_name.as_deref(), Some("Aperture RP | maintenance"));
    let audited = lab.audited(AuditAction::MaintenanceSettingsChanged).await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].details["to"]["message"], "Back tonight.");
    lab.close().await;
}
