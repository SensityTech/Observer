//! When to bring a failed server back, and when to stop trying.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RecoveryPolicy {
    /// The wait before each restart in a row; past the last, it doubles up to `backoff_cap`.
    pub backoff: Vec<Duration>,
    pub backoff_cap: Duration,
    /// A server up this long since it was ready starts the sequence over.
    pub healthy_reset: Duration,
    /// This many failures within `breaker_window` stop automatic restarts, and so do this many in
    /// a row of runs that never got ready, however far apart: a server that hangs at every start
    /// takes the whole hang delay to fail, up to an hour, which no window of minutes would hold.
    pub breaker_failures: usize,
    pub breaker_window: Duration,
    /// A taken port adds this much to the wait each time, up to `bind_cap`.
    pub bind_step: Duration,
    pub bind_cap: Duration,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            backoff: [500, 5_000, 15_000, 30_000, 60_000]
                .into_iter()
                .map(Duration::from_millis)
                .collect(),
            backoff_cap: Duration::from_mins(5),
            healthy_reset: Duration::from_mins(10),
            breaker_failures: 5,
            breaker_window: Duration::from_mins(10),
            bind_step: Duration::from_secs(5),
            bind_cap: Duration::from_secs(45),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    RestartIn(Duration),
    /// Too many failures too fast: wait for someone to look.
    CrashLoop {
        failures: usize,
    },
}

#[derive(Debug)]
pub struct Recovery {
    policy: RecoveryPolicy,
    failures: VecDeque<Instant>,
    /// Failures since a run last got ready.
    unready: usize,
    attempt: u32,
    bind_delay: Duration,
}

impl Recovery {
    pub fn new(policy: RecoveryPolicy) -> Self {
        Self {
            policy,
            failures: VecDeque::new(),
            unready: 0,
            attempt: 0,
            bind_delay: Duration::ZERO,
        }
    }

    /// A crash, a hang or a fatal error.
    pub fn failure(&mut self, now: Instant) -> Next {
        self.unready += 1;
        if self.unready >= self.policy.breaker_failures {
            return Next::CrashLoop {
                failures: self.unready,
            };
        }
        self.failures.push_back(now);
        while self
            .failures
            .front()
            .is_some_and(|first| now.saturating_duration_since(*first) > self.policy.breaker_window)
        {
            self.failures.pop_front();
        }
        if self.failures.len() >= self.policy.breaker_failures {
            return Next::CrashLoop {
                failures: self.failures.len(),
            };
        }
        let delay = self.delay(self.attempt);
        self.attempt += 1;
        Next::RestartIn(delay)
    }

    /// The port was taken. It is not the server failing, so it does not count toward the breaker.
    pub fn bind_error(&mut self) -> Duration {
        self.bind_delay = (self.bind_delay + self.policy.bind_step).min(self.policy.bind_cap);
        self.bind_delay
    }

    /// The server became ready: the port is free again, and the failures before no longer make a
    /// row of starts that never got ready.
    pub fn ready(&mut self) {
        self.bind_delay = Duration::ZERO;
        self.unready = 0;
    }

    /// The server has been up since it was ready for `up`.
    pub fn up_for(&mut self, up: Duration) {
        if up >= self.policy.healthy_reset {
            self.attempt = 0;
        }
    }

    /// Someone started the server by hand: the past no longer counts.
    pub fn reset(&mut self) {
        self.failures.clear();
        self.unready = 0;
        self.attempt = 0;
        self.bind_delay = Duration::ZERO;
    }

    fn delay(&self, attempt: u32) -> Duration {
        let steps = &self.policy.backoff;
        if let Some(delay) = steps.get(attempt as usize) {
            return *delay;
        }
        let last = steps.last().copied().unwrap_or(self.policy.backoff_cap);
        let beyond = attempt.saturating_sub(u32::try_from(steps.len()).unwrap_or(u32::MAX)) + 1;
        last.saturating_mul(2u32.saturating_pow(beyond))
            .min(self.policy.backoff_cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delays(recovery: &mut Recovery, start: Instant, count: u32, every: Duration) -> Vec<Next> {
        (0..count)
            .map(|index| recovery.failure(start + every * index))
            .collect()
    }

    fn restart(ms: u64) -> Next {
        Next::RestartIn(Duration::from_millis(ms))
    }

    #[test]
    fn restarts_back_off_then_double_up_to_the_cap() {
        let mut recovery = Recovery::new(RecoveryPolicy::default());

        // Failures far enough apart, of runs that got ready, never trip the breaker.
        let start = Instant::now();
        let next: Vec<Next> = (0..9)
            .map(|index| {
                recovery.ready();
                recovery.failure(start + Duration::from_mins(11) * index)
            })
            .collect();

        assert_eq!(
            next,
            [
                restart(500),
                restart(5_000),
                restart(15_000),
                restart(30_000),
                restart(60_000),
                restart(120_000),
                restart(240_000),
                restart(300_000),
                restart(300_000),
            ]
        );
    }

    #[test]
    fn five_failures_within_ten_minutes_trip_the_breaker() {
        let mut recovery = Recovery::new(RecoveryPolicy::default());

        let next = delays(&mut recovery, Instant::now(), 5, Duration::from_mins(2));

        assert_eq!(next[3], restart(30_000));
        assert_eq!(next[4], Next::CrashLoop { failures: 5 });
    }

    #[test]
    fn failures_older_than_the_window_do_not_count() {
        let mut recovery = Recovery::new(RecoveryPolicy::default());
        let start = Instant::now();
        delays(&mut recovery, start, 4, Duration::from_mins(1));
        recovery.ready();

        assert!(matches!(
            recovery.failure(start + Duration::from_mins(11)),
            Next::RestartIn(_)
        ));
    }

    #[test]
    fn ten_healthy_minutes_start_the_sequence_over() {
        let mut recovery = Recovery::new(RecoveryPolicy::default());
        let start = Instant::now();
        delays(&mut recovery, start, 3, Duration::from_mins(11));

        recovery.ready();
        recovery.up_for(Duration::from_mins(9));
        assert_eq!(recovery.failure(start + Duration::from_mins(40)), restart(30_000));
        recovery.ready();
        recovery.up_for(Duration::from_mins(10));
        assert_eq!(recovery.failure(start + Duration::from_mins(60)), restart(500));
    }

    #[test]
    fn a_taken_port_waits_five_seconds_more_each_time_up_to_45() {
        let mut recovery = Recovery::new(RecoveryPolicy::default());

        let waits: Vec<u64> = (0..11).map(|_| recovery.bind_error().as_secs()).collect();

        assert_eq!(waits, [5, 10, 15, 20, 25, 30, 35, 40, 45, 45, 45]);
        recovery.ready();
        assert_eq!(recovery.bind_error().as_secs(), 5);
    }

    /// Each start hangs the default six minutes after its spawn, then the backoff before the next.
    fn hangs_at_every_start(recovery: &mut Recovery, start: Instant, count: usize) -> Vec<Next> {
        let hang = Duration::from_mins(6);
        let mut now = start;
        (0..count)
            .map(|_| {
                now += hang;
                let next = recovery.failure(now);
                if let Next::RestartIn(delay) = next {
                    now += delay;
                }
                next
            })
            .collect()
    }

    #[test]
    fn a_server_that_hangs_at_every_start_trips_the_breaker_however_long_the_delay() {
        let mut recovery = Recovery::new(RecoveryPolicy::default());

        // No two of these fall within ten minutes of the first of a set of five.
        let next = hangs_at_every_start(&mut recovery, Instant::now(), 5);

        assert_eq!(
            next,
            [
                restart(500),
                restart(5_000),
                restart(15_000),
                restart(30_000),
                Next::CrashLoop { failures: 5 },
            ]
        );
    }

    #[test]
    fn a_run_that_got_ready_breaks_the_row_of_starts_that_never_did() {
        let mut recovery = Recovery::new(RecoveryPolicy::default());
        let start = Instant::now();
        hangs_at_every_start(&mut recovery, start, 4);

        recovery.ready();
        let next = hangs_at_every_start(&mut recovery, start + Duration::from_hours(1), 4);

        assert!(
            next.iter().all(|next| matches!(next, Next::RestartIn(_))),
            "{next:?}"
        );
        assert_eq!(
            hangs_at_every_start(&mut recovery, start + Duration::from_hours(2), 1),
            [Next::CrashLoop { failures: 5 }]
        );
        recovery.reset();
        assert_eq!(
            hangs_at_every_start(&mut recovery, start + Duration::from_hours(3), 1),
            [restart(500)]
        );
    }

    #[test]
    fn a_start_by_hand_clears_the_breaker() {
        let mut recovery = Recovery::new(RecoveryPolicy::default());
        let start = Instant::now();
        delays(&mut recovery, start, 5, Duration::from_secs(1));

        recovery.reset();

        assert_eq!(recovery.failure(start + Duration::from_secs(6)), restart(500));
    }
}
