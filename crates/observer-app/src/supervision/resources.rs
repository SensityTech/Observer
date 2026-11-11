//! The server's resources as the panel follows them: what FXServer's bridge lists and its console
//! says of each, who asked for what on the Resources page, and the orders the page sends, which
//! are FXServer's own `start`, `stop`, `restart` and `refresh`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use observer_domain::audit::AuditAction;
use observer_domain::changes::LiveChangeKind;
use observer_domain::error::ErrorCode;
use observer_domain::resources::{
    MAX_ADDED_NAMED, ResourceAck, ResourceAsked, ResourceOrigin, ResourceRow, ResourceState, ResourceTurn,
    ResourceWarnings, ResourcesRescan, ResourcesStage, ResourcesView,
};
use observer_domain::server::Delivery;
use observer_fxserver::resources::{Dependants, Known, Origin, State as Held, TurnKind};
use observer_fxserver::start::Marks;
use observer_fxserver::stdin::{ResourceName, ResourceVerb, StdinError};
use observer_fxserver::supervisor::{self as fx, ControlError, EventKind, State, Supervisor};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;

use super::{Supervision, control_error};
use crate::audit::Record;
use crate::{Caller, Error, Servers};

/// An order FXServer has not answered within this is given up: a resource that takes minutes to
/// start answers late, and what happens to it after is no longer put down to who asked.
const ANSWERED_WITHIN: Duration = Duration::from_secs(120);
/// The view is worked out again at most this often while reports come in.
const VIEW_EVERY: Duration = Duration::from_millis(250);
/// What the console says of a resource it could not start comes with no report of the bridge: the
/// view is worked out again this often while a panel follows it.
const WATCHED_EVERY: Duration = Duration::from_secs(1);
/// A rescan's count of changed manifests is the console's, read within this of the new list.
const RESCAN_SAID_WITHIN_MS: i64 = 60_000;

/// What the page asked, until it shows.
struct Asked {
    by: String,
    at: Instant,
    /// When the resource last started or stopped as the order was made, or the list last grew.
    before: Option<i64>,
}

/// An order on one resource, until FXServer answers it.
struct Order {
    asked: Asked,
    kind: LiveChangeKind,
    /// When FXServer last said it could not start the resource, as the order was made.
    failed_before: Option<i64>,
}

/// Who asked for what on the page in the run that is up, by resource.
#[derive(Default)]
pub(super) struct Orders {
    generation: u32,
    asked: HashMap<String, Order>,
    /// Who a resource's latest start or stop is put down to, with when it happened.
    credited: HashMap<String, (i64, String)>,
    rescan: Option<Asked>,
    rescanned: Option<(i64, String)>,
}

impl Orders {
    fn ask(&mut self, resources: &fx::Resources, name: &str, kind: LiveChangeKind, by: &str) {
        self.of(resources.generation);
        let before = resources
            .book
            .find(name)
            .and_then(|known| Some(known.turn.as_ref()?.at_ms));
        self.asked.insert(
            name.to_owned(),
            Order {
                asked: Asked {
                    by: by.to_owned(),
                    at: Instant::now(),
                    before,
                },
                kind,
                failed_before: failed_at(&resources.marks, name),
            },
        );
    }

    /// The order's line never reached FXServer.
    fn withdraw(&mut self, name: &str) {
        self.asked.remove(name);
    }

    /// The order FXServer has not answered on a resource.
    fn waiting(&self, name: &str) -> Option<ResourceAsked> {
        self.asked.get(name).map(|order| ResourceAsked {
            kind: order.kind,
            by: order.asked.by.clone(),
        })
    }

    fn ask_rescan(&mut self, resources: &fx::Resources, by: &str) {
        self.of(resources.generation);
        self.rescan = Some(Asked {
            by: by.to_owned(),
            at: Instant::now(),
            before: resources.book.added().map(|added| added.at_ms),
        });
    }

    /// The orders of another run are of no resource of this one.
    fn of(&mut self, generation: u32) {
        if self.generation != generation {
            *self = Self {
                generation,
                ..Self::default()
            };
        }
    }

    /// Puts down to who asked what happened to a resource since their order, until FXServer
    /// answered it: the resource is as the order wanted it, or FXServer said it could not start
    /// it. A restart may show as a stop, then as a start.
    fn credit(&mut self, resources: &fx::Resources) {
        self.of(resources.generation);
        let (book, marks) = (&resources.book, &resources.marks);
        let credited = &mut self.credited;
        self.asked.retain(|name, order| {
            if order.asked.at.elapsed() >= ANSWERED_WITHIN {
                return false;
            }
            let stops = order.kind == LiveChangeKind::Stopped;
            let turned = book
                .find(name)
                .and_then(|known| known.turn.as_ref())
                .filter(|turn| Some(turn.at_ms) != order.asked.before);
            if let Some(turn) = turned {
                credited.insert(name.clone(), (turn.at_ms, order.asked.by.clone()));
                if stops == (turn.kind == TurnKind::Stopped) {
                    return false;
                }
            }
            stops || failed_at(marks, name) == order.failed_before
        });
        if let Some(asked) = &self.rescan {
            let grew = book
                .added()
                .map(|added| added.at_ms)
                .filter(|at| Some(*at) != asked.before);
            if let Some(at) = grew {
                self.rescanned = Some((at, asked.by.clone()));
            }
            if grew.is_some() || asked.at.elapsed() >= ANSWERED_WITHIN {
                self.rescan = None;
            }
        }
    }

    fn by(&self, name: &str, at_ms: i64) -> Option<String> {
        self.credited
            .get(name)
            .filter(|(at, _)| *at == at_ms)
            .map(|(_, by)| by.clone())
    }
}

/// When FXServer last said it could not start a resource. The console's marks go by lower case.
fn failed_at(marks: &Marks, name: &str) -> Option<i64> {
    marks.failed.get(&name.to_ascii_lowercase()).copied()
}

/// The view of the resources as they stand, with what `orders` knows of who asked.
fn view(supervisor: &Supervisor, orders: &Mutex<Orders>) -> ResourcesView {
    let state = supervisor.status().borrow().state;
    let unlisted = |generation: u32| ResourcesView {
        stage: match state {
            State::Starting => ResourcesStage::Starting,
            State::Online | State::Degraded | State::Hung | State::Stopping => ResourcesStage::Unreported,
            State::Stopped | State::Crashed | State::CrashLoop => ResourcesStage::Stopped,
        },
        generation,
        ..ResourcesView::default()
    };
    let Some(resources) = supervisor.resources() else {
        return unlisted(0);
    };
    if !resources.book.listed() {
        return unlisted(resources.generation);
    }
    let mut orders = orders.lock().unwrap_or_else(PoisonError::into_inner);
    orders.credit(&resources);
    let (book, marks) = (&resources.book, &resources.marks);
    let dependants = book.dependants();
    ResourcesView {
        stage: ResourcesStage::Listed,
        generation: resources.generation,
        resources: book
            .resources()
            .iter()
            .map(|known| row(known, &dependants, marks, &orders))
            .collect(),
        truncated: book.truncated(),
        rescan: book.added().map(|added| ResourcesRescan {
            at: at(added.at_ms),
            by: orders
                .rescanned
                .as_ref()
                .filter(|(at_ms, _)| *at_ms == added.at_ms)
                .map(|(_, by)| by.clone()),
            added: added.names.iter().take(MAX_ADDED_NAMED).cloned().collect(),
            added_count: u32::try_from(added.names.len()).unwrap_or(u32::MAX),
            changed: marks
                .refreshed
                .filter(|said| (added.at_ms - said.at_ms).abs() <= RESCAN_SAID_WITHIN_MS)
                .map(|said| said.changed),
        }),
        rescanning: orders.rescan.is_some(),
    }
}

fn row(known: &Known, dependants: &Dependants<'_>, marks: &Marks, orders: &Orders) -> ResourceRow {
    let key = known.name.to_ascii_lowercase();
    let started = known.state == Held::Started;
    ResourceRow {
        name: known.name.clone(),
        folder: known.folder.clone(),
        origin: match known.origin {
            Origin::Server => ResourceOrigin::Server,
            Origin::Build => ResourceOrigin::Build,
        },
        version: known.version.clone(),
        state: match known.state {
            Held::Started => ResourceState::Started,
            Held::Starting => ResourceState::Starting,
            Held::Stopped => ResourceState::Stopped,
            Held::Stopping => ResourceState::Stopping,
            Held::Unknown => ResourceState::Unknown,
        },
        failed_at: marks
            .failed
            .get(&key)
            .filter(|_| !started)
            .map(|at_ms| at(*at_ms)),
        warnings: marks.warned.get(&key).map(|count| {
            let (heavy, oversized) = marks.heavy.get(&key).copied().unwrap_or_default();
            ResourceWarnings {
                count: *count,
                heavy,
                oversized,
                missing: marks.missing.get(&key).copied().unwrap_or_default(),
            }
        }),
        dependants: if started {
            dependants.of(&known.name)
        } else {
            Vec::new()
        },
        controllable: ResourceName::new(&known.name).is_some(),
        last: known.turn.as_ref().map(|turn| ResourceTurn {
            at: at(turn.at_ms),
            kind: match turn.kind {
                TurnKind::Started => LiveChangeKind::Started,
                TurnKind::Stopped => LiveChangeKind::Stopped,
                TurnKind::Restarted => LiveChangeKind::Restarted,
            },
            by: orders.by(&known.name, turn.at_ms),
            with: turn.with.clone(),
        }),
        asked: orders.waiting(&known.name),
    }
}

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap_or_default()
}

/// Keeps the view in step with what the supervisor hears and the page asks: a report of the
/// bridge, a change of state, an order, and each second while a panel follows it, for what only
/// the console says.
pub(super) async fn follow(
    supervisor: Supervisor,
    orders: Arc<Mutex<Orders>>,
    ordered: Arc<Notify>,
    live: watch::Sender<ResourcesView>,
    stop: CancellationToken,
) {
    let mut events = supervisor.events();
    let mut status = supervisor.status();
    let mut watched = tokio::time::interval(WATCHED_EVERY);
    watched.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last = tokio::time::Instant::now();
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            event = events.recv() => match event {
                Ok(event) => match event.kind {
                    EventKind::Resources { .. }
                    | EventKind::ResourceList { .. }
                    | EventKind::State { .. }
                    | EventKind::Exited { .. } => {}
                    _ => continue,
                },
                // The supervisor still holds what the missed events said.
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            changed = status.changed() => {
                if changed.is_err() {
                    return;
                }
                status.borrow_and_update();
            }
            () = ordered.notified() => {}
            // The service keeps one receiver of its own.
            _ = watched.tick() => {
                if live.receiver_count() <= 1 {
                    continue;
                }
            }
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep_until(last + VIEW_EVERY) => {}
        }
        last = tokio::time::Instant::now();
        let next = view(&supervisor, &orders);
        live.send_if_modified(|current| {
            let changed = *current != next;
            *current = next;
            changed
        });
    }
}

impl Supervision {
    /// The resources of the run that is up, as they change.
    pub fn resources(&self) -> watch::Receiver<ResourcesView> {
        self.inner.resources.clone()
    }

    pub async fn start_resource(&self, caller: &Caller, name: &str) -> Result<ResourceAck, Error> {
        self.order(caller, ResourceVerb::Start, name, false).await
    }

    /// Stops a resource. FXServer stops with it every started resource that depends on it, and
    /// starts none of them again: the answer names them.
    pub async fn stop_resource(&self, caller: &Caller, name: &str) -> Result<ResourceAck, Error> {
        self.order(caller, ResourceVerb::Stop, name, false).await
    }

    /// Restarts a started resource. With `again`, the resources that depend on it, which FXServer
    /// stops with it, are started once it is back.
    pub async fn restart_resource(
        &self,
        caller: &Caller,
        name: &str,
        again: bool,
    ) -> Result<ResourceAck, Error> {
        self.order(caller, ResourceVerb::Restart, name, again).await
    }

    /// The order is audited before anything is written, as a console command is, and counts as
    /// one of the caller's commands of the window. It shows on the list from then on, and no
    /// other is taken on its resource until FXServer answers it.
    async fn order(
        &self,
        caller: &Caller,
        verb: ResourceVerb,
        name: &str,
        again: bool,
    ) -> Result<ResourceAck, Error> {
        let resources = self.listed()?;
        let known = resources.book.find(name).ok_or(ErrorCode::ResourceNotFound)?;
        let target = ResourceName::new(&known.name).ok_or(ErrorCode::ResourceNameRefused)?;
        let started = known.state == Held::Started;
        // FXServer starts nothing that is up and stops nothing that is not: no answer would come.
        if started == (verb == ResourceVerb::Start) {
            return Err(ErrorCode::ResourceChanged.into());
        }
        // Nothing depends on a resource that is not up.
        let dependants = if started {
            resources.book.dependants().of(&known.name)
        } else {
            Vec::new()
        };
        self.throttle(caller)?;

        let command = format!("{} {}", verb.as_str(), target.as_str());
        let (kind, action) = match verb {
            ResourceVerb::Start => (LiveChangeKind::Started, AuditAction::ResourceStarted),
            ResourceVerb::Stop => (LiveChangeKind::Stopped, AuditAction::ResourceStopped),
            ResourceVerb::Restart => (LiveChangeKind::Restarted, AuditAction::ResourceRestarted),
        };
        // The log names what stopped with the resource, and whether a restart started it again.
        let details = if dependants.is_empty() {
            json!({ "command": command })
        } else if verb == ResourceVerb::Restart {
            json!({ "command": command, "dependants": dependants, "startedAgain": again })
        } else {
            json!({ "command": command, "dependants": dependants })
        };
        let again: Vec<ResourceName> = if again {
            dependants
                .iter()
                .filter_map(|dependant| ResourceName::new(dependant))
                .collect()
        } else {
            Vec::new()
        };
        {
            let mut orders = self.orders();
            orders.credit(&resources);
            if orders.waiting(target.as_str()).is_some() {
                return Err(ErrorCode::ResourceBusy.into());
            }
            orders.ask(&resources, target.as_str(), kind, &caller.username);
            // FXServer stops them with the resource, then the order starts each again.
            for dependant in &again {
                orders.ask(
                    &resources,
                    dependant.as_str(),
                    LiveChangeKind::Restarted,
                    &caller.username,
                );
            }
        }
        self.inner.ordered.notify_one();

        let record = Record::by(caller, action)
            .target(known.name.clone())
            .details(details);
        let mut answer = self.audit(record).await.map(|()| Delivery::Written);
        let lines = std::iter::once((verb, target)).chain(
            again
                .into_iter()
                .map(|dependant| (ResourceVerb::Start, dependant)),
        );
        // The orders whose line never reached FXServer: no answer comes for those.
        let mut unwritten = Vec::new();
        for (verb, name) in lines {
            let asked = name.as_str().to_owned();
            if matches!(answer, Ok(Delivery::Written)) {
                answer = self.write(verb, name, caller).await;
                if matches!(answer, Ok(Delivery::Written | Delivery::Pending)) {
                    continue;
                }
            }
            unwritten.push(asked);
        }
        if !unwritten.is_empty() {
            let mut orders = self.orders();
            for name in &unwritten {
                orders.withdraw(name);
            }
            drop(orders);
            self.inner.ordered.notify_one();
        }
        Ok(ResourceAck {
            delivery: answer?,
            dependants,
        })
    }

    /// Has FXServer read its resource folders again: new resources and changed manifests. Nothing
    /// starts or stops.
    pub async fn rescan_resources(&self, caller: &Caller) -> Result<ResourceAck, Error> {
        let resources = self.listed()?;
        self.throttle(caller)?;
        let record = Record::by(caller, AuditAction::ResourcesRescanned)
            .target(Servers::DEFAULT)
            .details(json!({ "command": "refresh" }));
        self.audit(record).await?;
        self.orders().ask_rescan(&resources, &caller.username);
        self.inner.ordered.notify_one();
        let written = self.inner.supervisor.refresh_resources(&caller.username).await;
        let answer = delivery(written);
        if !matches!(answer, Ok(Delivery::Written | Delivery::Pending)) {
            self.orders().rescan = None;
            self.inner.ordered.notify_one();
        }
        Ok(ResourceAck {
            delivery: answer?,
            dependants: Vec::new(),
        })
    }

    fn orders(&self) -> MutexGuard<'_, Orders> {
        self.inner.orders.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The resources of a server that listed them: an order names one of those, and no other.
    fn listed(&self) -> Result<fx::Resources, Error> {
        match self.inner.supervisor.resources() {
            Some(resources) if resources.book.listed() => Ok(resources),
            Some(_) => Err(ErrorCode::ResourcesNotListed.into()),
            None => Err(ErrorCode::ServerNotRunning.into()),
        }
    }

    async fn write(
        &self,
        verb: ResourceVerb,
        name: ResourceName,
        caller: &Caller,
    ) -> Result<Delivery, Error> {
        delivery(self.inner.supervisor.resource(verb, name, &caller.username).await)
    }
}

/// How a line went: a server that is not reading its console is an answer, not a failure.
fn delivery(written: Result<(), ControlError>) -> Result<Delivery, Error> {
    match written {
        Ok(()) => Ok(Delivery::Written),
        Err(ControlError::Stdin(StdinError::Pending)) => Ok(Delivery::Pending),
        Err(ControlError::Stdin(StdinError::Busy)) => Ok(Delivery::Busy),
        Err(error) => Err(control_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use observer_fxserver::resources::{Book, Reported};

    use super::*;

    const BRIDGE: &str = "sys_k3jx7qmw2pza";

    fn reported(name: &str, state: &str, needs: &[&str]) -> Reported {
        Reported {
            name: name.to_owned(),
            state: state.to_owned(),
            path: Some(format!("C:/srv/resources//[core]/{name}")),
            version: Some("1.2.0".to_owned()),
            needs: needs.iter().map(|&need| need.to_owned()).collect(),
            provides: Vec::new(),
        }
    }

    fn resources(generation: u32) -> fx::Resources {
        let mut book = Book::default();
        book.page(
            1,
            1,
            true,
            vec![
                reported("ox_lib", "started", &[]),
                reported("ox_target", "started", &["ox_lib"]),
                reported("taxi", "stopped", &[]),
                reported("bad name", "started", &[]),
            ],
            BRIDGE,
            1000,
        );
        fx::Resources {
            generation,
            book,
            marks: Marks::default(),
        }
    }

    fn turned(resources: &mut fx::Resources, started: &[&str], events: &[(&str, bool)], at_ms: i64) {
        let started: Vec<String> = started.iter().map(|&name| name.to_owned()).collect();
        let events: Vec<(String, bool)> = events
            .iter()
            .map(|&(name, started)| (name.to_owned(), started))
            .collect();
        resources.book.started(&started, &events, at_ms);
    }

    #[test]
    fn a_row_says_what_the_bridge_listed_and_what_the_console_said() {
        let mut resources = resources(3);
        resources.marks.failed.insert("taxi".to_owned(), 4000);
        resources.marks.failed.insert("ox_lib".to_owned(), 4000);
        resources.marks.warned.insert("ox_target".to_owned(), 3);
        resources.marks.heavy.insert("ox_target".to_owned(), (2, 1));
        resources.marks.missing.insert("ox_target".to_owned(), 1);
        let mut orders = Orders::default();
        orders.ask(&resources, "taxi", LiveChangeKind::Started, "ada");
        let row = |name: &str| {
            row(
                resources.book.find(name).unwrap(),
                &resources.book.dependants(),
                &resources.marks,
                &orders,
            )
        };

        let lib = row("ox_lib");
        assert_eq!(
            (lib.folder.as_str(), lib.origin, lib.state, lib.version.as_deref()),
            (
                "[core]",
                ResourceOrigin::Server,
                ResourceState::Started,
                Some("1.2.0")
            )
        );
        assert_eq!(lib.dependants, ["ox_target"]);
        assert_eq!(lib.failed_at, None, "it started since");
        assert!(lib.controllable);
        assert_eq!(lib.asked, None);
        assert_eq!(
            row("ox_target").warnings,
            Some(ResourceWarnings {
                count: 3,
                heavy: 2,
                oversized: 1,
                missing: 1
            })
        );
        let taxi = row("taxi");
        assert_eq!(
            (taxi.state, taxi.failed_at),
            (ResourceState::Stopped, Some(at(4000)))
        );
        assert_eq!(taxi.dependants, Vec::<String>::new());
        assert_eq!(
            taxi.asked,
            Some(asked(LiveChangeKind::Started)),
            "an order FXServer has not answered"
        );
        assert!(!row("bad name").controllable);
    }

    fn asked(kind: LiveChangeKind) -> ResourceAsked {
        ResourceAsked {
            kind,
            by: "ada".to_owned(),
        }
    }

    #[test]
    fn what_follows_an_order_is_put_down_to_who_made_it_until_fxserver_answered() {
        let mut resources = resources(3);
        let mut orders = Orders::default();
        orders.ask(&resources, "ox_lib", LiveChangeKind::Restarted, "ada");
        orders.ask(&resources, "bad name", LiveChangeKind::Stopped, "ada");
        orders.credit(&resources);
        assert_eq!(orders.by("ox_lib", 1000), None, "nothing happened yet");
        assert_eq!(orders.waiting("ox_lib"), Some(asked(LiveChangeKind::Restarted)));

        // A restart may fall in two reports of the bridge: its stop, then its start.
        turned(
            &mut resources,
            &["bad name"],
            &[("ox_target", false), ("ox_lib", false)],
            2000,
        );
        orders.credit(&resources);
        assert_eq!(orders.by("ox_lib", 2000).as_deref(), Some("ada"));
        assert_eq!(orders.by("ox_target", 2000), None, "it stopped with the other");
        assert_eq!(
            orders.waiting("ox_lib"),
            Some(asked(LiveChangeKind::Restarted)),
            "it is not back up yet"
        );
        assert_eq!(
            orders.waiting("bad name"),
            Some(asked(LiveChangeKind::Stopped)),
            "its order still waits"
        );

        turned(&mut resources, &["bad name", "ox_lib"], &[("ox_lib", true)], 3000);
        orders.credit(&resources);
        assert_eq!(orders.by("ox_lib", 3000).as_deref(), Some("ada"));
        assert_eq!(orders.waiting("ox_lib"), None, "FXServer answered");

        turned(&mut resources, &["bad name"], &[("ox_lib", false)], 4000);
        orders.credit(&resources);
        assert_eq!(
            orders.by("ox_lib", 4000),
            None,
            "another's stop, after the answer"
        );

        turned(&mut resources, &[], &[("bad name", false)], 5000);
        orders.credit(&resources);
        assert_eq!(orders.by("bad name", 5000).as_deref(), Some("ada"));
        assert!(orders.asked.is_empty());
    }

    #[test]
    fn a_restart_told_in_one_report_is_answered_at_once() {
        let mut resources = resources(3);
        let mut orders = Orders::default();
        orders.ask(&resources, "ox_lib", LiveChangeKind::Restarted, "ada");

        turned(
            &mut resources,
            &["bad name", "ox_lib"],
            &[("ox_target", false), ("ox_lib", false), ("ox_lib", true)],
            2000,
        );
        orders.credit(&resources);

        assert_eq!(orders.by("ox_lib", 2000).as_deref(), Some("ada"));
        assert_eq!(orders.waiting("ox_lib"), None);
    }

    #[test]
    fn a_start_fxserver_could_not_do_is_answered_and_a_silent_order_is_given_up() {
        let mut resources = resources(3);
        let mut orders = Orders::default();
        orders.ask(&resources, "taxi", LiveChangeKind::Started, "ada");
        orders.credit(&resources);
        assert_eq!(orders.waiting("taxi"), Some(asked(LiveChangeKind::Started)));

        resources.marks.failed.insert("taxi".to_owned(), 2000);
        orders.credit(&resources);
        assert_eq!(orders.waiting("taxi"), None, "FXServer said it could not");
        turned(
            &mut resources,
            &["bad name", "ox_lib", "ox_target", "taxi"],
            &[("taxi", true)],
            3000,
        );
        orders.credit(&resources);
        assert_eq!(orders.by("taxi", 3000), None, "another's start, after the answer");

        orders.ask(&resources, "taxi", LiveChangeKind::Stopped, "ada");
        orders.withdraw("taxi");
        assert_eq!(orders.waiting("taxi"), None, "its line never reached FXServer");

        orders.ask(&resources, "ox_lib", LiveChangeKind::Stopped, "ada");
        for order in orders.asked.values_mut() {
            order.asked.at -= ANSWERED_WITHIN;
        }
        turned(
            &mut resources,
            &["bad name", "taxi"],
            &[("ox_target", false), ("ox_lib", false)],
            9000,
        );
        orders.credit(&resources);
        assert_eq!(orders.by("ox_lib", 9000), None, "the order is too old to count");
        assert!(orders.asked.is_empty());
    }

    #[test]
    fn the_orders_of_a_run_say_nothing_of_the_next() {
        let mut first = resources(3);
        let mut orders = Orders::default();
        orders.ask(&first, "ox_lib", LiveChangeKind::Stopped, "ada");
        orders.ask_rescan(&first, "ada");
        turned(&mut first, &["ox_target"], &[("ox_lib", false)], 2000);
        orders.credit(&first);
        assert_eq!(orders.by("ox_lib", 2000).as_deref(), Some("ada"));

        let mut next = resources(4);
        turned(&mut next, &["ox_target"], &[("ox_lib", false)], 2000);
        orders.credit(&next);

        assert_eq!(orders.by("ox_lib", 2000), None);
        assert!(orders.rescan.is_none());
    }

    #[test]
    fn a_rescan_is_put_down_to_who_asked_once_the_list_came_again() {
        let mut resources = resources(3);
        let mut orders = Orders::default();
        orders.ask_rescan(&resources, "ada");
        orders.credit(&resources);
        assert_eq!(orders.rescanned, None);

        resources.book.page(
            2,
            1,
            true,
            vec![
                reported("ox_lib", "started", &[]),
                reported("bank", "stopped", &[]),
            ],
            BRIDGE,
            5000,
        );
        orders.credit(&resources);

        assert_eq!(orders.rescanned, Some((5000, "ada".to_owned())));
        assert!(orders.rescan.is_none());
    }
}
