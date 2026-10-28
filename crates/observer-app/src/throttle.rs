//! Login throttling. Repeated failures lock a pair of username and client network for a while;
//! an account as a whole only gets slower, so a stranger cannot lock the owner out.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use observer_domain::error::{ApiError, ErrorCode};
use observer_store::login_failures::LoginFailureRow;

use crate::{Clock, Error};

/// Failures count toward a lock, and reach the audit log, in windows of this length.
pub(crate) const WINDOW: SignedDuration = SignedDuration::from_mins(15);
const LOCK_AFTER: u32 = 5;
const FIRST_LOCK: SignedDuration = SignedDuration::from_mins(1);
const LONGEST_LOCK: SignedDuration = SignedDuration::from_mins(15);
/// After a failure, the network waits this long before its next attempt.
const COOLDOWN: SignedDuration = SignedDuration::from_secs(1);
const ACCOUNT_SLOWDOWN_AFTER: u32 = 10;
pub(crate) const ACCOUNT_SLOWDOWN: Duration = Duration::from_secs(1);

/// The client network a lock applies to: the address itself for IPv4, its /64 for IPv6, since a
/// single IPv6 host usually holds a whole /64.
pub(crate) fn network(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                ip.to_string()
            } else {
                let prefix = u128::from(ip) & !u128::from(u64::MAX);
                format!("{}/64", std::net::Ipv6Addr::from(prefix))
            }
        }
    }
}

pub(crate) fn too_many_attempts(wait: SignedDuration) -> Error {
    // Whole seconds, rounded up, so that a client retrying on time is never early.
    let seconds = wait.as_secs() + i64::from(wait.subsec_nanos() > 0);
    ApiError::new(ErrorCode::TooManyAttempts)
        .with("retryAfter", seconds.max(1))
        .into()
}

/// One login attempt in flight per client network, and a short pause after each failure.
#[derive(Debug)]
pub(crate) struct Attempts {
    clock: Clock,
    networks: Mutex<HashMap<String, Slot>>,
}

#[derive(Debug, Clone, Copy)]
enum Slot {
    InFlight,
    CoolingUntil(Timestamp),
}

impl Attempts {
    pub(crate) fn new(clock: Clock) -> Self {
        Self {
            clock,
            networks: Mutex::default(),
        }
    }

    /// Takes the network's slot before any password is hashed.
    pub(crate) fn reserve(&self, network: &str) -> Result<Reservation<'_>, Error> {
        let now = self.clock.now();
        let mut networks = self.networks.lock().unwrap_or_else(PoisonError::into_inner);
        networks.retain(|_, slot| !matches!(slot, Slot::CoolingUntil(until) if *until <= now));
        match networks.get(network) {
            Some(Slot::InFlight) => return Err(too_many_attempts(COOLDOWN)),
            Some(Slot::CoolingUntil(until)) => return Err(too_many_attempts(until.duration_since(now))),
            None => {}
        }
        networks.insert(network.to_owned(), Slot::InFlight);
        Ok(Reservation {
            attempts: self,
            network: network.to_owned(),
            succeeded: false,
        })
    }
}

/// A network's slot, given back when dropped: at once after a success, after the cooldown
/// otherwise, including when the request was abandoned halfway.
pub(crate) struct Reservation<'a> {
    attempts: &'a Attempts,
    network: String,
    succeeded: bool,
}

impl Reservation<'_> {
    pub(crate) fn succeeded(mut self) {
        self.succeeded = true;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let now = self.attempts.clock.now();
        let mut networks = self
            .attempts
            .networks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.succeeded {
            networks.remove(&self.network);
        } else {
            networks.insert(self.network.clone(), Slot::CoolingUntil(now + COOLDOWN));
        }
    }
}

/// Failures of one pair that the audit log has not seen yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unaudited {
    pub username: String,
    pub network: String,
    pub failures: u32,
    pub first: Timestamp,
    pub last: Timestamp,
}

/// How long the pair stays locked, if it is.
pub(crate) fn locked_for(row: &LoginFailureRow, now: Timestamp) -> Option<SignedDuration> {
    row.locked_until
        .filter(|until| *until > now)
        .map(|until| until.duration_since(now))
}

/// Counts a failed password. Five within a window lock the pair; locks that follow each other
/// double, up to the longest, and a quiet window starts them over. Returns the updated row and the
/// failures of a closed audit window, which go to the audit log first.
pub(crate) fn record_failure(
    row: Option<LoginFailureRow>,
    username: &str,
    network: &str,
    now: Timestamp,
) -> (LoginFailureRow, Option<Unaudited>) {
    let mut row = row.unwrap_or_else(|| new_row(username, network, now));
    let closed = count_for_audit(&mut row, now);
    count_toward_lock(&mut row, now);
    (row, closed)
}

/// Counts a wrong password from a signed-in user toward the pair's lock, like a failed login. The
/// user's own audit entry records it, so it is not one of the anonymous failures the log
/// aggregates.
pub(crate) fn record_confirmation_failure(
    row: Option<LoginFailureRow>,
    username: &str,
    network: &str,
    now: Timestamp,
) -> LoginFailureRow {
    let mut row = row.unwrap_or_else(|| new_row(username, network, now));
    count_toward_lock(&mut row, now);
    row
}

fn new_row(username: &str, network: &str, now: Timestamp) -> LoginFailureRow {
    LoginFailureRow {
        username: username.to_owned(),
        network: network.to_owned(),
        failures: 0,
        window_started_at: now,
        locked_until: None,
        lockouts: 0,
        unaudited: 0,
        unaudited_since: None,
        last_failure_at: now,
    }
}

fn count_toward_lock(row: &mut LoginFailureRow, now: Timestamp) {
    row.last_failure_at = now;
    if now >= row.window_started_at + WINDOW {
        row.failures = 0;
        row.window_started_at = now;
    }
    row.failures += 1;
    if row.failures >= LOCK_AFTER {
        let follows_a_lock = row.locked_until.is_some_and(|until| now < until + WINDOW);
        let lockouts = if follows_a_lock { row.lockouts } else { 0 };
        row.locked_until = Some(now + lock_length(lockouts));
        row.lockouts = lockouts + 1;
        row.failures = 0;
        row.window_started_at = now;
    }
}

/// Counts an attempt refused by a lock. It changes nothing about the lock, but the audit log
/// still hears of it.
pub(crate) fn record_refused(
    mut row: LoginFailureRow,
    now: Timestamp,
) -> (LoginFailureRow, Option<Unaudited>) {
    let closed = count_for_audit(&mut row, now);
    (row, closed)
}

/// Takes what the audit log has not seen yet out of the row.
pub(crate) fn take_unaudited(row: &mut LoginFailureRow) -> Option<Unaudited> {
    let first = row.unaudited_since.take()?;
    let failures = std::mem::take(&mut row.unaudited);
    (failures > 0).then(|| Unaudited {
        username: row.username.clone(),
        network: row.network.clone(),
        failures,
        first,
        last: row.last_failure_at,
    })
}

/// The pause an account gets when it fails often from several networks at once.
pub(crate) fn account_slowdown(rows: &[LoginFailureRow], now: Timestamp) -> Option<Duration> {
    let recent: u32 = rows
        .iter()
        .filter(|row| row.unaudited_since.is_some_and(|since| now < since + WINDOW))
        .map(|row| row.unaudited)
        .sum();
    (recent >= ACCOUNT_SLOWDOWN_AFTER).then_some(ACCOUNT_SLOWDOWN)
}

fn count_for_audit(row: &mut LoginFailureRow, now: Timestamp) -> Option<Unaudited> {
    let closed = match row.unaudited_since {
        Some(since) if now >= since + WINDOW => take_unaudited(row),
        _ => None,
    };
    row.unaudited += 1;
    row.unaudited_since.get_or_insert(now);
    closed
}

fn lock_length(earlier_locks: u32) -> SignedDuration {
    let doubled = FIRST_LOCK * (1 << earlier_locks.min(4));
    doubled.min(LONGEST_LOCK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> Timestamp {
        "2026-03-01T12:00:00Z".parse::<Timestamp>().unwrap() + SignedDuration::from_mins(minutes)
    }

    fn fail(row: Option<LoginFailureRow>, minute: i64) -> LoginFailureRow {
        record_failure(row, "ada", "203.0.113.7", at(minute)).0
    }

    #[test]
    fn networks_are_addresses_for_ipv4_and_prefixes_for_ipv6() {
        assert_eq!(network("203.0.113.7".parse().unwrap()), "203.0.113.7");
        assert_eq!(network("::ffff:203.0.113.7".parse().unwrap()), "203.0.113.7");
        assert_eq!(
            network("2001:db8:1:2:aaaa:bbbb:cccc:dddd".parse().unwrap()),
            "2001:db8:1:2::/64"
        );
        assert_eq!(
            network("2001:db8:1:2::1".parse().unwrap()),
            network("2001:db8:1:2:ffff::9".parse().unwrap())
        );
    }

    #[test]
    fn the_fifth_failure_in_a_window_locks_the_pair_for_a_minute() {
        let mut row = None;
        for minute in 0..4 {
            let next = fail(row, minute);
            assert_eq!(locked_for(&next, at(minute)), None);
            row = Some(next);
        }

        let locked = fail(row, 4);

        assert_eq!(locked_for(&locked, at(4)), Some(SignedDuration::from_mins(1)));
        assert_eq!(locked_for(&locked, at(5)), None);
    }

    #[test]
    fn failures_spread_over_more_than_a_window_never_lock() {
        let mut row = None;
        for minute in [0, 5, 10, 14, 16, 20, 25, 29] {
            let next = fail(row, minute);
            assert_eq!(locked_for(&next, at(minute)), None, "minute {minute}");
            row = Some(next);
        }
    }

    #[test]
    fn locks_that_follow_each_other_double_up_to_fifteen_minutes() {
        let mut row = None;
        let mut minute = 0;
        let mut lengths = Vec::new();
        for _ in 0..6 {
            for _ in 0..5 {
                row = Some(fail(row, minute));
            }
            let length = locked_for(row.as_ref().unwrap(), at(minute)).unwrap();
            lengths.push(length.as_mins());
            minute += length.as_mins();
        }

        assert_eq!(lengths, [1, 2, 4, 8, 15, 15]);

        // A quiet quarter of an hour after the lock ends starts over at one minute.
        minute += 15;
        for _ in 0..5 {
            row = Some(fail(row, minute));
        }
        assert_eq!(
            locked_for(row.as_ref().unwrap(), at(minute)),
            Some(SignedDuration::from_mins(1))
        );
    }

    #[test]
    fn failures_reach_the_audit_log_once_per_window() {
        let mut row = None;
        let mut closed = Vec::new();
        for minute in [0, 1, 2, 14, 15, 16] {
            let (next, window) = record_failure(row, "ada", "203.0.113.7", at(minute));
            closed.extend(window);
            row = Some(next);
        }

        assert_eq!(
            closed,
            [Unaudited {
                username: "ada".to_owned(),
                network: "203.0.113.7".to_owned(),
                failures: 4,
                first: at(0),
                last: at(14),
            }]
        );
        let mut row = row.unwrap();
        let rest = take_unaudited(&mut row).unwrap();
        assert_eq!((rest.failures, rest.first, rest.last), (2, at(15), at(16)));
        assert_eq!(take_unaudited(&mut row), None);
    }

    #[test]
    fn an_attempt_refused_by_a_lock_is_audited_without_extending_it() {
        let mut row = None;
        for minute in 0..5 {
            row = Some(fail(row, minute));
        }
        let locked = row.unwrap();

        let (refused, _) = record_refused(locked.clone(), at(4));

        assert_eq!(refused.locked_until, locked.locked_until);
        assert_eq!(refused.unaudited, 6);
    }

    #[test]
    fn an_account_failing_from_many_networks_is_slowed_but_not_locked() {
        let now = at(10);
        let rows: Vec<_> = (0..5)
            .map(|host| {
                let network = format!("198.51.100.{host}");
                let first = record_failure(None, "ada", &network, at(1)).0;
                record_failure(Some(first), "ada", &network, at(2)).0
            })
            .collect();

        assert_eq!(account_slowdown(&rows, now), Some(ACCOUNT_SLOWDOWN));
        assert_eq!(account_slowdown(&rows[..4], now), None);
        assert_eq!(account_slowdown(&rows, at(20)), None);
        assert!(rows.iter().all(|row| locked_for(row, now).is_none()));
    }

    #[test]
    fn one_attempt_per_network_is_in_flight_and_a_failure_cools_it_down() {
        let clock = Clock::manual(at(0));
        let attempts = Attempts::new(clock.clone());

        let first = attempts.reserve("203.0.113.7").unwrap();
        assert!(attempts.reserve("203.0.113.7").is_err());
        let other = attempts.reserve("198.51.100.4").unwrap();
        other.succeeded();
        drop(first);

        assert!(attempts.reserve("203.0.113.7").is_err());
        assert!(attempts.reserve("198.51.100.4").is_ok());
        clock.advance(COOLDOWN);
        attempts.reserve("203.0.113.7").unwrap().succeeded();
        attempts.reserve("203.0.113.7").unwrap();
    }
}
