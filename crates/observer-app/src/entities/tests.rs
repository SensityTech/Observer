use std::net::{IpAddr, SocketAddr};

use jiff::SignedDuration;
use observer_domain::audit::AuditFilter;
use observer_domain::entities::{EntitiesUnavailable, EntityCounts, EntityKind, EntityTarget};
use observer_domain::map::MapBounds;
use tempfile::TempDir;

use super::*;
use crate::{Accounts, AuditLog, Client, PasswordCost, Running, Servers, Setup};

/// The state database in a temporary folder, with a supervision that runs no server.
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
        Self {
            _dir: dir,
            db,
            clock,
            supervision,
            _running: running,
        }
    }

    fn entities(&self) -> (Entities, EntitiesTask) {
        Entities::start(EntitiesSetup {
            db: self.db.clone(),
            clock: self.clock.clone(),
            supervision: self.supervision.clone(),
            players: None,
        })
    }
}

fn caller(permissions: Permissions) -> Caller {
    Caller {
        session_id: 1,
        user_id: 1,
        username: "ada".to_owned(),
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

fn staff() -> Caller {
    caller(Permissions::all())
}

const GENERATION: u32 = 3;
const EPOCH: i64 = 4210;
/// A car a player sits in, a train, a ped and two objects, the first from a resource.
const ROWS: &str = "1,11,2,1,3078201489,10.0,-1.0,30.0,4,0,7,1,1;\
                    2,12,2,8,1030400667,20.0,-2.0,30.0,-1,0,5,0,0;\
                    3,13,1,0,1885233650,30.0,-3.0,30.0,9,3,5,0,0;\
                    4,14,3,0,169553981,40.0,-4.0,30.0,-1,0,7,1,0;\
                    5,15,3,0,169553981,50.0,-5.0,30.0,-1,0,5,0,0";

fn end() -> SweepEnd {
    SweepEnd {
        counts: EntityCounts {
            vehicles: Some(2),
            peds: Some(3),
            objects: Some(2),
            player_peds: 2,
        },
        work_us: Some(1200),
        list_us: Some(300),
        frames: Some(2),
        every_ms: Some(15_000),
        resources: vec!["qb-garages".to_owned()],
        ..SweepEnd::default()
    }
}

fn part(seq: u32, part: u32, rows: &str, end: Option<SweepEnd>) -> EntityReport {
    EntityReport {
        generation: GENERATION,
        kind: EntityReportKind::Sweep(SweepPart {
            epoch: EPOCH,
            seq,
            part,
            rows: rows.to_owned(),
            end,
        }),
    }
}

/// The list as one whole sweep leaves it.
fn swept(entities: &Entities) {
    entities.on_report(part(1, 1, ROWS, Some(end())), &mut None, true);
}

fn target(net_id: u16, serial: u32, model: u32) -> EntityTarget {
    EntityTarget {
        net_id,
        serial,
        model,
    }
}

fn code(result: Result<EntityDeletion, Error>) -> Option<ErrorCode> {
    result.err().and_then(|error| error.code())
}

async fn all(entities: &Entities, caller: &Caller) -> EntityPage {
    entities
        .page(caller, &EntityFilters::default(), EntitySort::NetId, false, 0)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_sweep_becomes_the_list_once_its_last_part_came() {
    let lab = Lab::new().await;
    let (entities, task) = lab.entities();
    let (first, second) = ROWS.split_at(ROWS.find(';').unwrap());
    let mut gathering = None;

    entities.on_report(part(1, 1, first, None), &mut gathering, true);
    let before = all(&entities, &staff()).await;
    entities.on_report(part(1, 2, &second[1..], Some(end())), &mut gathering, true);
    let page = all(&entities, &staff()).await;
    task.close().await;

    assert_eq!(before.live, EntitiesLive::default());
    assert_eq!(before.items, []);
    assert_eq!(page.live.run.as_deref(), Some("3-4210"));
    assert_eq!((page.live.sweep, page.live.listed), (1, 5));
    assert_eq!(page.live.at, Some(lab.clock.now()));
    assert_eq!(page.live.counts.map(|counts| counts.player_peds), Some(2));
    assert_eq!((page.matching, page.page), (5, 0));
    let car = &page.items[0];
    assert_eq!((car.net_id, car.serial, car.kind), (1, 11, EntityKind::Vehicle));
    assert_eq!(car.resource.as_deref(), Some("qb-garages"));
    assert_eq!((car.owned, car.owner, car.occupied), (true, Some(4), true));
    assert_eq!(page.limits, EntityLimits::default());
}

#[tokio::test]
async fn a_sweep_is_dropped_while_nobody_looks() {
    let lab = Lab::new().await;
    let (entities, task) = lab.entities();

    entities.on_report(part(1, 1, ROWS, Some(end())), &mut None, false);
    let page = all(&entities, &staff()).await;
    task.close().await;

    assert_eq!(page.live, EntitiesLive::default());
}

#[test]
fn a_part_lost_or_repeated_spoils_its_sweep_and_the_next_whole_one_is_taken() {
    let sweep = |seq: u32, number: u32, last: bool| SweepPart {
        epoch: EPOCH,
        seq,
        part: number,
        rows: format!("{seq}.{number}"),
        end: last.then(end),
    };
    let mut gathering = None;

    assert!(gather(&mut gathering, GENERATION, sweep(1, 1, false)).is_none());
    assert!(
        gather(&mut gathering, GENERATION, sweep(1, 3, true)).is_none(),
        "part 2 never came"
    );
    assert!(gathering.is_none());
    assert!(
        gather(&mut gathering, GENERATION, sweep(2, 2, true)).is_none(),
        "no first part"
    );
    assert!(gather(&mut gathering, GENERATION, sweep(3, 1, false)).is_none());
    assert!(
        gather(&mut gathering, GENERATION, sweep(3, 1, false)).is_none(),
        "starts over"
    );
    assert!(
        gather(&mut gathering, GENERATION + 1, sweep(3, 2, true)).is_none(),
        "another server"
    );
    assert!(gather(&mut gathering, GENERATION, sweep(4, 1, false)).is_none());
    let whole = gather(&mut gathering, GENERATION, sweep(4, 2, true));

    let (gathered, _) = whole.expect("the sweep is whole");
    assert_eq!(gathered.parts, ["4.1", "4.2"]);
    assert_eq!((gathered.seq, gathered.epoch), (4, EPOCH));
    assert!(gathering.is_none());
}

#[test]
fn a_sweep_of_more_parts_than_the_bridge_sends_is_dropped() {
    let mut gathering = None;

    for number in 1..=MAX_PARTS {
        let part = SweepPart {
            epoch: EPOCH,
            seq: 1,
            part: number,
            rows: String::new(),
            end: None,
        };
        assert!(gather(&mut gathering, GENERATION, part).is_none());
    }

    assert!(gathering.is_none());
}

#[tokio::test]
async fn a_server_that_cannot_list_its_entities_has_no_list_and_says_why() {
    let lab = Lab::new().await;
    let (entities, task) = lab.entities();
    swept(&entities);

    let unavailable = SweepEnd {
        unavailable: Some(EntitiesUnavailable::Onesync),
        ..SweepEnd::default()
    };
    entities.on_report(part(2, 1, "", Some(unavailable)), &mut None, true);
    let page = all(&entities, &staff()).await;
    task.close().await;

    assert_eq!(page.live.unavailable, Some(EntitiesUnavailable::Onesync));
    assert_eq!((page.live.run, page.items.len()), (None, 0));
}

#[tokio::test]
async fn who_owns_an_entity_is_told_only_to_who_sees_where_players_stand() {
    let lab = Lab::new().await;
    let (entities, task) = lab.entities();
    swept(&entities);

    let page = all(&entities, &caller(Permissions::ENTITIES_VIEW)).await;
    task.close().await;

    assert!(!page.owners_shown);
    let car = &page.items[0];
    assert_eq!((car.owned, car.owner, car.occupied), (true, None, false));
    assert_eq!((page.facets.owned, page.facets.owners.len()), (2, 0));
}

#[tokio::test]
async fn the_map_gets_each_entity_the_filters_keep() {
    let lab = Lab::new().await;
    let (entities, task) = lab.entities();
    swept(&entities);
    let objects = EntityFilters {
        kinds: vec![EntityKind::Object],
        ..EntityFilters::default()
    };

    let map = entities.map(&staff(), &objects, 200, None).await.unwrap();
    let odd = entities.map(&staff(), &objects, 123, None).await;
    let east = MapBounds {
        west: 45.0,
        east: 60.0,
        south: -10.0,
        north: 0.0,
    };
    let in_view = entities
        .map(&staff(), &EntityFilters::default(), 200, Some(east))
        .await
        .unwrap();
    let inside_out = entities
        .map(
            &staff(),
            &objects,
            200,
            Some(MapBounds {
                west: 60.0,
                east: 45.0,
                ..east
            }),
        )
        .await;
    task.close().await;

    assert_eq!((map.matching, map.cell, map.cells.len()), (2, None, 0));
    assert_eq!(
        (map.kinds.vehicles, map.kinds.peds, map.kinds.objects),
        (2, 1, 2),
        "the whole list, whatever the filters"
    );
    assert_eq!(map.points.len(), 2);
    let shown: Vec<u16> = in_view.points.iter().map(|point| point.0).collect();
    assert_eq!(shown, [5], "only what stands in the part of the map asked for");
    assert_eq!(
        inside_out.err().and_then(|error| error.code()),
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!((map.points[0].0, map.points[0].1, map.points[0].2), (4, 14, 3));
    assert_eq!(map.live.run.as_deref(), Some("3-4210"));
    assert_eq!(
        odd.err().and_then(|error| error.code()),
        Some(ErrorCode::InvalidRequest)
    );
}

#[tokio::test]
async fn nothing_is_deleted_from_a_list_that_is_missing_changed_or_out_of_date() {
    let lab = Lab::new().await;
    let (entities, task) = lab.entities();
    let ask = |run: &str, targets: Vec<EntityTarget>| EntityDeleteRequest {
        run: run.to_owned(),
        targets,
    };
    let object = || vec![target(5, 15, 169_553_981)];

    let unlisted = entities.delete(&staff(), ask("3-4210", object())).await;
    swept(&entities);
    let none = entities.delete(&staff(), ask("3-4210", Vec::new())).await;
    let many = entities
        .delete(
            &staff(),
            ask("3-4210", vec![target(5, 15, 169_553_981); MAX_DELETE + 1]),
        )
        .await;
    let restarted = entities.delete(&staff(), ask("2-4210", object())).await;
    let stopped = entities.delete(&staff(), ask("3-4210", object())).await;
    lab.clock.advance(SignedDuration::from_secs(46));
    let old = entities.delete(&staff(), ask("3-4210", object())).await;
    task.close().await;

    assert_eq!(code(unlisted), Some(ErrorCode::EntitiesNotListed));
    assert_eq!(code(none), Some(ErrorCode::InvalidRequest));
    match many {
        Err(Error::Refused(error)) => {
            assert_eq!(error.code, ErrorCode::TooManyEntities);
            assert_eq!(error.params["max"], 100);
        }
        other => panic!("not refused: {other:?}"),
    }
    assert_eq!(code(restarted), Some(ErrorCode::EntitiesChanged));
    // No server runs here: what the list allows goes no further.
    assert_eq!(code(stopped), Some(ErrorCode::ServerNotRunning));
    assert_eq!(code(old), Some(ErrorCode::EntitiesOutOfDate));
}

#[tokio::test]
async fn what_the_list_already_rules_out_is_answered_without_asking_the_server() {
    let lab = Lab::new().await;
    let (entities, task) = lab.entities();
    let audit = AuditLog::new(lab.db.clone(), lab.clock.clone());
    swept(&entities);

    let deletion = entities
        .delete(
            &staff(),
            EntityDeleteRequest {
                run: "3-4210".to_owned(),
                targets: vec![
                    target(1, 11, 3_078_201_489),
                    target(2, 12, 1_030_400_667),
                    target(5, 99, 169_553_981),
                    target(4, 14, 7),
                    target(77, 1, 1),
                    target(1, 11, 3_078_201_489),
                ],
            },
        )
        .await
        .unwrap();
    let page = all(&entities, &staff()).await;
    let recorded = audit.page(AuditFilter::default()).await.unwrap();
    task.close().await;

    let outcomes: Vec<(u16, EntityOutcome)> = deletion
        .results
        .iter()
        .map(|result| (result.net_id, result.outcome))
        .collect();
    assert_eq!(
        outcomes,
        [
            (1, EntityOutcome::Occupied),
            (2, EntityOutcome::Train),
            (5, EntityOutcome::Changed),
            (4, EntityOutcome::Changed),
            (77, EntityOutcome::Gone),
        ]
    );
    assert_eq!(page.live.listed, 5);
    assert!(
        recorded
            .entries
            .iter()
            .all(|entry| entry.action != AuditAction::EntitiesDeleted),
        "nothing was asked of the server"
    );
}

#[tokio::test]
async fn the_cadence_changes_within_what_the_panel_offers_and_leaves_a_trace() {
    let lab = Lab::new().await;
    let (entities, task) = lab.entities();
    let audit = AuditLog::new(lab.db.clone(), lab.clock.clone());

    let refused = entities
        .change_settings(&staff(), EntitiesSettings { every_seconds: 7 })
        .await;
    entities
        .change_settings(&staff(), EntitiesSettings { every_seconds: 30 })
        .await
        .unwrap();
    entities
        .change_settings(&staff(), EntitiesSettings { every_seconds: 30 })
        .await
        .unwrap();
    let saved = entities.settings().await.unwrap();
    let recorded = audit.page(AuditFilter::default()).await.unwrap();
    task.close().await;

    assert_eq!(
        refused.err().and_then(|error| error.code()),
        Some(ErrorCode::InvalidEntitiesSettings)
    );
    assert_eq!(saved, EntitiesSettings { every_seconds: 30 });
    let changes: Vec<_> = recorded
        .entries
        .iter()
        .filter(|entry| entry.action == AuditAction::EntitiesSettingsChanged)
        .collect();
    assert_eq!(changes.len(), 1, "saving the same settings again changes nothing");
    assert_eq!(changes[0].details["to"]["everySeconds"], 30);
}
