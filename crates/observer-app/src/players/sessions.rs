//! The sessions of the run under way, from the bridge's reports, FXServer's player list and the
//! supervisor's events. It decides; what to write comes out as [`Writes`], which the service
//! flushes once a second.

use std::collections::{HashMap, HashSet};
use std::mem;
use std::time::{Duration, Instant};

use observer_domain::drops::{DropCategory, DropKind, ReportedDrop, classify};
use observer_domain::players::IdentifierKind;
use observer_fxserver::health::PlayerList;
use observer_fxserver::players::{Dropped, Joined, PlayerReport, PlayerReportKind, RosterPage, Shown};
use observer_store::metrics::runs::RunKey;
use observer_store::metrics::sessions::{Closure, NewSession};

use super::identity::{Claim, Fingerprints};
use super::redact::redact;

/// A player list fetched this soon after a report may have been asked before it: it says nothing
/// of that player yet.
const LIST_LAG: Duration = Duration::from_secs(3);
/// Successive player lists a session must be missing from before Observer closes it.
const MISSED_LISTS: u8 = 3;
/// A join this soon after a drop of the same net id, with no session between them, is the tail of
/// a player a resource dropped inside its own `playerJoining` handler.
const EARLY_DROP: Duration = Duration::from_secs(5);
/// A drop reported this long after Observer closed the session as missed still belongs to it.
const LATE_DROP: Duration = Duration::from_mins(5);
/// The game timer and Observer's own clock may disagree by this much.
const CLOCK_SLACK_MS: i64 = 1000;

/// A session of the run under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Open {
    pub(super) id: i64,
    pub(super) net_id: u32,
    /// When Observer saw it start, or first saw it.
    pub(super) seen_ms: i64,
    pub(super) identity: Identity,
    timer_ms: Option<i64>,
    received: Instant,
    absent: u8,
    last_listed_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Identity {
    /// Waiting for FXServer's player list to agree.
    Confirming(Claim),
    Confirmed {
        license: String,
        name: Option<String>,
        /// The key of its crediting record.
        credit: [u8; 16],
        /// Once the identity is written.
        player: Option<i64>,
    },
    /// The list disagreed: no identity, no play time.
    Unconfirmed,
    /// No licence, or forgotten while connected.
    Anonymous,
}

/// What a flush writes, in this order: identities and credits in the state database, then the
/// sessions in the metrics database.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Writes {
    pub(super) inserts: Vec<NewSession>,
    pub(super) identities: Vec<IdentityWrite>,
    pub(super) closures: Vec<ClosureWrite>,
    pub(super) reclassified: Vec<Closure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IdentityWrite {
    pub(super) session: i64,
    pub(super) license: String,
    pub(super) name: Option<String>,
    pub(super) others: Vec<(IdentifierKind, String)>,
    pub(super) seen_ms: i64,
    pub(super) credit: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClosureWrite {
    pub(super) closure: Closure,
    /// The crediting record to add the session to its profile with.
    pub(super) credit: Option<[u8; 16]>,
}

/// What was dropped or doubted, counted rather than logged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub(super) dropped_identifiers: usize,
    pub(super) unconfirmed: usize,
    pub(super) clamped: usize,
    pub(super) rosters_given_up: usize,
}

struct Run {
    key: RunKey,
    open: HashMap<u32, Open>,
    roster: Option<Roster>,
    /// Drops that matched no session, for the join that may trail them.
    early_drops: HashMap<u32, Instant>,
    /// Sessions closed as missed, for the drop that may come late.
    missed: HashMap<u32, (i64, Instant)>,
    /// Why Observer is stopping the server, once it said.
    stop: Option<DropKind>,
    hung: bool,
    last_list: Option<Instant>,
}

struct Roster {
    next_page: u32,
    timer_ms: Option<i64>,
    received: Instant,
    at_ms: i64,
    players: Vec<(u32, Shown)>,
    /// Players who dropped while the pages came: never opened again from the roster.
    dropped: HashSet<u32>,
}

pub(super) struct Sessions {
    run: Option<Run>,
    next_id: i64,
    keys: Box<dyn FnMut() -> [u8; 16] + Send>,
    writes: Writes,
    pub(super) counts: Counts,
}

impl Sessions {
    pub(super) fn new(next_id: i64, keys: Box<dyn FnMut() -> [u8; 16] + Send>) -> Self {
        Self {
            run: None,
            next_id,
            keys,
            writes: Writes::default(),
            counts: Counts::default(),
        }
    }

    pub(super) fn take_writes(&mut self) -> Writes {
        mem::take(&mut self.writes)
    }

    pub(super) fn open(&self) -> impl Iterator<Item = &Open> {
        self.run.iter().flat_map(|run| run.open.values())
    }

    /// FXServer started: a new run. Sessions still open from the last one, which a lost exit
    /// left behind, are closed as lost.
    pub(super) fn spawned(&mut self, key: RunKey, at_ms: i64) {
        if let Some(run) = self.run.take() {
            let mut previous = run;
            let open: Vec<Open> = previous.open.drain().map(|(_, open)| open).collect();
            for open in open {
                self.close_by_observer(&open, at_ms, DropCategory::Unknown, DropKind::Lost);
            }
        }
        self.run = Some(Run {
            key,
            open: HashMap::new(),
            roster: None,
            early_drops: HashMap::new(),
            missed: HashMap::new(),
            stop: None,
            hung: false,
            last_list: None,
        });
    }

    /// Observer is stopping the server, for this reason.
    pub(super) fn stopping(&mut self, generation: u32, kind: DropKind) {
        if let Some(run) = self.run_of(generation) {
            run.stop = Some(kind);
        }
    }

    /// The server was found hung: the exit that follows is a hang's.
    pub(super) fn hung(&mut self, generation: u32) {
        if let Some(run) = self.run_of(generation) {
            run.hung = true;
        }
    }

    /// FXServer's process ended: every session still open ends with it.
    pub(super) fn exited(&mut self, generation: u32, at_ms: i64, expected: bool) {
        let Some(run) = self.run_of(generation) else {
            return;
        };
        let (category, kind) = if expected {
            (DropCategory::ServerStop, run.stop.unwrap_or(DropKind::Console))
        } else if run.hung {
            (DropCategory::ServerCrash, DropKind::Hang)
        } else {
            (DropCategory::ServerCrash, DropKind::ServerCrashed)
        };
        let open: Vec<Open> = run.open.drain().map(|(_, open)| open).collect();
        for open in open {
            self.close_by_observer(&open, at_ms, category, kind);
        }
        self.run = None;
    }

    pub(super) fn report(&mut self, report: &PlayerReport, fingerprints: &Fingerprints) {
        if self
            .run
            .as_ref()
            .is_none_or(|run| run.key.generation != report.generation)
        {
            return;
        }
        match &report.kind {
            PlayerReportKind::Join(joined) => self.join(joined, report, fingerprints),
            PlayerReportKind::Drop(dropped) => self.dropped(dropped, report),
            PlayerReportKind::Roster(page) => self.roster(page, report, fingerprints),
        }
    }

    fn join(&mut self, joined: &Joined, report: &PlayerReport, fingerprints: &Fingerprints) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if run
            .early_drops
            .remove(&joined.net_id)
            .is_some_and(|dropped| report.received.duration_since(dropped) < EARLY_DROP)
        {
            return;
        }
        run.missed.remove(&joined.net_id);
        if let Some(stale) = run.open.remove(&joined.net_id) {
            // Net ids come back: a join on one still open means its drop was lost.
            self.close_by_observer(&stale, report.at_ms, DropCategory::Unknown, DropKind::Missed);
        }
        let identity = self.claim(&joined.shown, fingerprints);
        self.start(
            joined.net_id,
            Some(report.at_ms),
            report.at_ms,
            joined.timer_ms,
            report.received,
            identity,
        );
    }

    fn claim(&mut self, shown: &Shown, fingerprints: &Fingerprints) -> Identity {
        let (claim, dropped) = Claim::of(shown, fingerprints);
        self.counts.dropped_identifiers += dropped;
        claim.map_or(Identity::Anonymous, Identity::Confirming)
    }

    fn start(
        &mut self,
        net_id: u32,
        joined_ms: Option<i64>,
        seen_ms: i64,
        timer_ms: Option<i64>,
        received: Instant,
        identity: Identity,
    ) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let id = self.next_id;
        self.next_id += 1;
        self.writes.inserts.push(NewSession {
            id,
            run: run.key,
            net_id,
            joined_ms,
            seen_ms,
        });
        run.open.insert(
            net_id,
            Open {
                id,
                net_id,
                seen_ms,
                identity,
                timer_ms,
                received,
                absent: 0,
                last_listed_ms: None,
            },
        );
    }

    fn dropped(&mut self, dropped: &Dropped, report: &PlayerReport) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if let Some(roster) = run.roster.as_mut() {
            roster.dropped.insert(dropped.net_id);
        }
        let classified = classify(ReportedDrop {
            reason: &dropped.reason,
            resource: dropped.resource.as_deref(),
            category: dropped.category,
        });
        let kind = match classified.category {
            DropCategory::ServerStop => Some(run.stop.unwrap_or(DropKind::Console)),
            _ => classified.kind,
        };
        let mut closure = Closure {
            id: 0,
            ended_ms: report.at_ms,
            length_ms: None,
            category: classified.category,
            kind,
            native: classified.native,
            resource: classified.kicker,
            crash_text: classified.crash_text.as_deref().map(redact),
            reason_text: classified.keeps_reason.then(|| redact(&dropped.reason)),
            loading: dropped.ped.map(|ped| !ped),
            synthesized: false,
        };

        if let Some(open) = run.open.remove(&dropped.net_id) {
            closure.id = open.id;
            closure.length_ms = Some(self.reported_length(&open, dropped.timer_ms, report));
            let credit = credit_of(&open.identity);
            self.writes.closures.push(ClosureWrite { closure, credit });
            return;
        }
        if let Some((id, _)) = run
            .missed
            .remove(&dropped.net_id)
            .filter(|(_, closed)| report.received.duration_since(*closed) < LATE_DROP)
        {
            closure.id = id;
            self.writes.reclassified.push(closure);
            return;
        }
        // A drop before its join: a resource dropped the player inside its own handler.
        run.early_drops.insert(dropped.net_id, report.received);
        let id = self.next_id;
        self.next_id += 1;
        self.writes.inserts.push(NewSession {
            id,
            run: run.key,
            net_id: dropped.net_id,
            joined_ms: None,
            seen_ms: report.at_ms,
        });
        closure.id = id;
        self.writes.closures.push(ClosureWrite {
            closure,
            credit: None,
        });
    }

    /// The length on the game timer, which a wall clock change does not move, but never more than
    /// Observer itself measured between the two reports: a forged drop cannot add hours.
    fn reported_length(&mut self, open: &Open, timer_ms: Option<i64>, report: &PlayerReport) -> i64 {
        let measured = i64::try_from(report.received.duration_since(open.received).as_millis())
            .unwrap_or(i64::MAX)
            .saturating_add(CLOCK_SLACK_MS);
        match (open.timer_ms, timer_ms) {
            (Some(start), Some(end)) => {
                let game = (end - start).max(0);
                if game > measured {
                    self.counts.clamped += 1;
                }
                game.min(measured)
            }
            _ => (report.at_ms - open.seen_ms).clamp(0, measured),
        }
    }

    fn roster(&mut self, page: &RosterPage, report: &PlayerReport, fingerprints: &Fingerprints) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if page.page == 1 {
            run.roster = Some(Roster {
                next_page: 1,
                timer_ms: page.timer_ms,
                received: report.received,
                at_ms: report.at_ms,
                players: Vec::new(),
                dropped: HashSet::new(),
            });
        }
        let Some(roster) = run.roster.as_mut().filter(|roster| roster.next_page == page.page) else {
            if run.roster.take().is_some() || page.page > 1 {
                self.counts.rosters_given_up += 1;
            }
            return;
        };
        roster.next_page += 1;
        roster.players.extend(page.players.iter().cloned());
        if !page.last {
            return;
        }
        let Some(roster) = run.roster.take() else {
            return;
        };
        let listed: HashSet<u32> = roster.players.iter().map(|(id, _)| *id).collect();
        let stale: Vec<u32> = run
            .open
            .values()
            .filter(|open| !listed.contains(&open.net_id) && open.received < roster.received)
            .map(|open| open.net_id)
            .collect();
        for net_id in stale {
            if let Some(open) = self.run.as_mut().and_then(|run| run.open.remove(&net_id)) {
                let ended = open.last_listed_ms.unwrap_or(roster.at_ms).min(roster.at_ms);
                self.miss(&open, ended, report.received);
            }
        }
        for (net_id, shown) in &roster.players {
            let known = self.run.as_ref().is_some_and(|run| run.open.contains_key(net_id));
            if known || roster.dropped.contains(net_id) {
                continue;
            }
            let identity = self.claim(shown, fingerprints);
            self.start(
                *net_id,
                None,
                roster.at_ms,
                roster.timer_ms,
                roster.received,
                identity,
            );
        }
    }

    /// A list from `/players.json`: it confirms identities, and closes sessions it no longer
    /// shows.
    pub(super) fn listed(&mut self, generation: u32, list: &PlayerList, list_ms: i64, at: Instant) {
        let Some(run) = self.run_of(generation) else {
            return;
        };
        if list.anonymous || run.last_list.is_some_and(|last| last >= at) {
            return;
        }
        run.last_list = Some(at);
        let shown: HashMap<u32, _> = list.shown.iter().map(|listed| (listed.id, listed)).collect();
        let mut gone = Vec::new();
        let mut unconfirmed = 0;
        let mut keys = Vec::new();
        for open in run.open.values_mut() {
            let settled = at >= open.received + LIST_LAG;
            match shown.get(&open.net_id) {
                Some(listed) => {
                    open.absent = 0;
                    open.last_listed_ms = Some(list_ms);
                    if let Identity::Confirming(claim) = &open.identity
                        && (settled || claim.matches(listed))
                    {
                        if claim.matches(listed) {
                            keys.push(open.net_id);
                        } else {
                            open.identity = Identity::Unconfirmed;
                            unconfirmed += 1;
                        }
                    }
                }
                None if settled => {
                    open.absent += 1;
                    if open.absent >= MISSED_LISTS {
                        gone.push(open.net_id);
                    }
                }
                None => {}
            }
        }
        for net_id in keys {
            let credit = (self.keys)();
            let Some(open) = self.run.as_mut().and_then(|run| run.open.get_mut(&net_id)) else {
                continue;
            };
            let Identity::Confirming(claim) = mem::replace(&mut open.identity, Identity::Anonymous) else {
                continue;
            };
            self.writes.identities.push(IdentityWrite {
                session: open.id,
                license: claim.license.clone(),
                name: claim.name.clone(),
                others: claim.others,
                seen_ms: open.seen_ms,
                credit,
            });
            open.identity = Identity::Confirmed {
                license: claim.license,
                name: claim.name,
                credit,
                player: None,
            };
        }
        self.counts.unconfirmed += unconfirmed;
        for net_id in gone {
            if let Some(open) = self.run.as_mut().and_then(|run| run.open.remove(&net_id)) {
                let ended = open.last_listed_ms.unwrap_or(open.seen_ms);
                self.miss(&open, ended, at);
            }
        }
    }

    fn miss(&mut self, open: &Open, ended_ms: i64, now: Instant) {
        if let Some(run) = self.run.as_mut() {
            run.missed.insert(open.net_id, (open.id, now));
        }
        self.close_by_observer(open, ended_ms, DropCategory::Unknown, DropKind::Missed);
    }

    /// A session Observer closes itself, measured on the wall clock.
    fn close_by_observer(&mut self, open: &Open, ended_ms: i64, category: DropCategory, kind: DropKind) {
        let ended_ms = ended_ms.max(open.seen_ms);
        self.writes.closures.push(ClosureWrite {
            closure: Closure {
                id: open.id,
                ended_ms,
                length_ms: Some(ended_ms - open.seen_ms),
                category,
                kind: Some(kind),
                native: None,
                resource: None,
                crash_text: None,
                reason_text: None,
                loading: None,
                synthesized: true,
            },
            credit: credit_of(&open.identity),
        });
    }

    /// The identity of `session` is written: it belongs to `player`.
    pub(super) fn identified(&mut self, session: i64, player: i64) {
        let open = self
            .run
            .iter_mut()
            .flat_map(|run| run.open.values_mut())
            .find(|open| open.id == session);
        if let Some(Open {
            identity: Identity::Confirmed { player: known, .. },
            ..
        }) = open
        {
            *known = Some(player);
        }
    }

    /// Detaches a forgotten licence from the sessions under way: they keep running, earn nothing
    /// and write no identity.
    pub(super) fn forget(&mut self, license: &str) {
        let mut keys = Vec::new();
        for open in self.run.iter_mut().flat_map(|run| run.open.values_mut()) {
            let theirs = match &open.identity {
                Identity::Confirmed {
                    license: known,
                    credit,
                    ..
                } if known == license => {
                    keys.push(*credit);
                    true
                }
                Identity::Confirming(claim) => claim.license == license,
                _ => false,
            };
            if theirs {
                open.identity = Identity::Anonymous;
            }
        }
        self.writes
            .identities
            .retain(|identity| identity.license != license);
        for closure in &mut self.writes.closures {
            if closure.credit.is_some_and(|credit| keys.contains(&credit)) {
                closure.credit = None;
            }
        }
    }

    fn run_of(&mut self, generation: u32) -> Option<&mut Run> {
        self.run.as_mut().filter(|run| run.key.generation == generation)
    }
}

fn credit_of(identity: &Identity) -> Option<[u8; 16]> {
    match identity {
        Identity::Confirmed { credit, .. } => Some(*credit),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
