//! FXServer's tick time histograms, from the Prometheus text `/perf/` serves: per thread, the
//! ticks so far, the time they took, and how many fit under each bound.

use std::collections::BTreeMap;

/// FXServer prints bounds such as `0.07000000000000001`.
const TOLERANCE: f64 = 1e-9;

/// The threads whose ticks Observer follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Thread {
    Main,
    Network,
    Sync,
}

impl Thread {
    pub const ALL: [Self; 3] = [Self::Main, Self::Network, Self::Sync];

    pub fn name(self) -> &'static str {
        match self {
            Self::Main => "svMain",
            Self::Network => "svNetwork",
            Self::Sync => "svSync",
        }
    }

    /// The tick each thread should stay under, in seconds: svMain runs at 20 Hz.
    pub fn budget(self) -> f64 {
        match self {
            Self::Main => 0.050,
            Self::Network => 0.010,
            Self::Sync => 0.008,
        }
    }

    fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|thread| thread.name() == name)
    }
}

/// One thread's histogram: cumulative counts under each finite bound, in seconds, ascending.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    pub bounds: Vec<f64>,
    pub cumulative: Vec<u64>,
    pub count: u64,
    /// Seconds.
    pub sum: f64,
}

/// What one answer of `/perf/` said, for the threads it described properly with the family's bounds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TickReport {
    threads: BTreeMap<Thread, Histogram>,
}

/// A thread's ticks between two reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickDelta {
    pub ticks: u64,
    /// Whole milliseconds, which is what FXServer observes.
    pub total_ms: u64,
    /// Ticks longer than the thread's budget; `None` when the budget is not one of the bounds, as
    /// with builds before mid-2024.
    pub over: Option<u64>,
}

impl TickReport {
    /// Reads every `tickTime_bucket`, `_count` and `_sum` line, labels in any order. A thread
    /// whose histogram is malformed, or whose bounds differ from the family's, is left out; the
    /// others are kept. `None` when no thread is left.
    pub fn parse(text: &str) -> Option<Self> {
        let mut raw: BTreeMap<Thread, Raw> = BTreeMap::new();
        for line in text.lines() {
            let Some((metric, labels, value)) = sample(line) else {
                continue;
            };
            let Some(thread) = label(labels, "name").and_then(Thread::named) else {
                continue;
            };
            let entry = raw.entry(thread).or_default();
            match metric {
                "tickTime_bucket" => match label(labels, "le") {
                    Some(bound) => entry.buckets.push((bound.to_owned(), value.to_owned())),
                    None => entry.malformed = true,
                },
                "tickTime_count" => entry.count = value.parse().ok(),
                "tickTime_sum" => entry.sum = value.parse().ok().filter(|sum: &f64| sum.is_finite()),
                _ => {}
            }
        }
        let mut histograms: Vec<(Thread, Histogram)> = raw
            .into_iter()
            .filter_map(|(thread, raw)| Some((thread, raw.histogram()?)))
            .collect();
        // svMain's bounds, or the next thread's in order without it.
        let family = histograms
            .first()
            .map(|(_, histogram)| histogram.bounds.clone())?;
        histograms.retain(|(_, histogram)| same_bounds(&histogram.bounds, &family));
        Some(Self {
            threads: histograms.into_iter().collect(),
        })
    }

    pub fn thread(&self, thread: Thread) -> Option<&Histogram> {
        self.threads.get(&thread)
    }

    /// `svMain`'s tick count, which health watches.
    pub fn main_ticks(&self) -> Option<u64> {
        self.thread(Thread::Main).map(|histogram| histogram.count)
    }

    /// Each thread's ticks since `earlier`, for the threads both describe with the same bounds and
    /// whose counts did not go back (FXServer restarted its counters).
    pub fn since(&self, earlier: &Self) -> BTreeMap<Thread, TickDelta> {
        self.threads
            .iter()
            .filter_map(|(&thread, now)| {
                let before = earlier.thread(thread)?;
                if !same_bounds(&now.bounds, &before.bounds) {
                    return None;
                }
                let ticks = now.count.checked_sub(before.count)?;
                let seconds = now.sum - before.sum;
                if seconds < 0.0 {
                    return None;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let total_ms = (seconds * 1000.0).round() as u64;
                let over = now
                    .bounds
                    .iter()
                    .position(|bound| (bound - thread.budget()).abs() <= TOLERANCE)
                    .and_then(|at| now.cumulative[at].checked_sub(before.cumulative[at]))
                    .and_then(|within| ticks.checked_sub(within));
                Some((
                    thread,
                    TickDelta {
                        ticks,
                        total_ms,
                        over,
                    },
                ))
            })
            .collect()
    }
}

#[derive(Default)]
struct Raw {
    buckets: Vec<(String, String)>,
    count: Option<u64>,
    sum: Option<f64>,
    malformed: bool,
}

impl Raw {
    /// Finite bounds strictly ascending, counts that never decrease, and a `+Inf` bucket equal to
    /// the count.
    fn histogram(self) -> Option<Histogram> {
        if self.malformed {
            return None;
        }
        let count = self.count?;
        let sum = self.sum?;
        let mut infinite = None;
        let mut finite: Vec<(f64, u64)> = Vec::new();
        for (bound, value) in &self.buckets {
            let value: u64 = value.parse().ok()?;
            if bound == "+Inf" {
                if infinite.replace(value).is_some() {
                    return None;
                }
                continue;
            }
            let bound: f64 = bound.parse().ok().filter(|bound: &f64| bound.is_finite())?;
            finite.push((bound, value));
        }
        finite.sort_by(|a, b| a.0.total_cmp(&b.0));
        let ascending = finite
            .windows(2)
            .all(|pair| pair[1].0 - pair[0].0 > TOLERANCE && pair[1].1 >= pair[0].1);
        let last = finite.last().map_or(0, |&(_, value)| value);
        if finite.is_empty() || !ascending || infinite != Some(count) || last > count {
            return None;
        }
        let (bounds, cumulative) = finite.into_iter().unzip();
        Some(Histogram {
            bounds,
            cumulative,
            count,
            sum,
        })
    }
}

fn same_bounds(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() <= TOLERANCE)
}

/// `metric{labels} value`, the labels without their braces.
fn sample(line: &str) -> Option<(&str, &str, &str)> {
    let (metric, rest) = line.split_once('{')?;
    let (labels, value) = rest.split_once('}')?;
    Some((metric.trim(), labels, value.trim()))
}

fn label<'a>(labels: &'a str, name: &str) -> Option<&'a str> {
    labels.split(',').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim().trim_matches('"'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bounds FXServer 35245 prints, in its own spelling.
    const BOUNDS: [&str; 14] = [
        "0.001",
        "0.002",
        "0.004",
        "0.006",
        "0.008",
        "0.01",
        "0.015",
        "0.02",
        "0.03",
        "0.05",
        "0.07000000000000001",
        "0.1",
        "0.15",
        "0.25",
    ];
    /// Before mid-2024: no 8 ms bound, so svSync's budget cannot be read.
    const OLD_BOUNDS: [&str; 8] = ["0.005", "0.01", "0.025", "0.05", "0.075", "0.1", "0.25", "0.5"];

    /// A histogram whose every observation took `ms`.
    fn thread(name: &str, bounds: &[&str], count: u32, ms: f64) -> String {
        use std::fmt::Write as _;
        let sum = f64::from(count) * ms / 1000.0;
        let mut text = format!(
            "tickTime_count{{name=\"{name}\"}} {count}
tickTime_sum{{name=\"{name}\"}} {sum}
"
        );
        for bound in bounds {
            let within = if ms / 1000.0 <= bound.parse::<f64>().unwrap() + TOLERANCE {
                count
            } else {
                0
            };
            let _ = writeln!(text, "tickTime_bucket{{name=\"{name}\",le=\"{bound}\"}} {within}");
        }
        let _ = writeln!(text, "tickTime_bucket{{le=\"+Inf\",name=\"{name}\"}} {count}");
        text
    }

    fn report(threads: &[(&str, u32, f64)]) -> TickReport {
        let text: String = threads
            .iter()
            .map(|&(name, count, ms)| thread(name, &BOUNDS, count, ms))
            .collect();
        TickReport::parse(&format!(
            "# HELP tickTime Time spent on server ticks\n# TYPE tickTime histogram\n{text}"
        ))
        .unwrap()
    }

    #[test]
    fn every_thread_of_fxserver_35245_is_read_whatever_the_order_of_threads_and_labels() {
        let report = report(&[
            ("svNetwork", 9000, 1.0),
            ("svSync", 7000, 2.0),
            ("svMain", 388, 12.0),
        ]);

        let main = report.thread(Thread::Main).unwrap();
        assert_eq!(main.count, 388);
        assert_eq!(main.bounds.len(), 14);
        assert!((main.bounds[10] - 0.07).abs() < 1e-12);
        assert_eq!(main.cumulative[6], 388, "12 ms fits under 15 ms");
        assert_eq!(main.cumulative[5], 0);
        assert!((main.sum - 4.656).abs() < 1e-9);
        assert_eq!(report.thread(Thread::Network).unwrap().count, 9000);
        assert_eq!(report.thread(Thread::Sync).unwrap().count, 7000);
        assert_eq!(report.main_ticks(), Some(388));
    }

    #[test]
    fn other_metrics_and_threads_are_ignored() {
        let text = format!(
            "# HELP other\nhttp_requests{{name=\"svMain\"}} 3\n{}{}",
            thread("svScripts", &BOUNDS, 5, 1.0),
            thread("svMain", &BOUNDS, 10, 1.0)
        );

        let report = TickReport::parse(&text).unwrap();

        assert_eq!(report.threads.keys().copied().collect::<Vec<_>>(), [Thread::Main]);
    }

    #[test]
    fn the_old_bounds_are_read_too() {
        let report = TickReport::parse(&thread("svMain", &OLD_BOUNDS, 40, 30.0)).unwrap();

        assert_eq!(report.thread(Thread::Main).unwrap().bounds.len(), 8);
    }

    #[test]
    fn a_missing_thread_leaves_the_others() {
        let report = report(&[("svMain", 10, 1.0), ("svSync", 10, 1.0)]);

        assert!(report.thread(Thread::Network).is_none());
        assert!(report.thread(Thread::Sync).is_some());
    }

    #[test]
    fn a_thread_with_other_bounds_than_svmain_is_dropped() {
        let text = thread("svMain", &BOUNDS, 10, 1.0) + &thread("svNetwork", &OLD_BOUNDS, 10, 1.0);

        let report = TickReport::parse(&text).unwrap();

        assert!(report.thread(Thread::Main).is_some());
        assert!(report.thread(Thread::Network).is_none());
    }

    #[test]
    fn without_svmain_the_first_thread_sets_the_bounds() {
        let text = thread("svNetwork", &OLD_BOUNDS, 10, 1.0) + &thread("svSync", &BOUNDS, 10, 1.0);

        let report = TickReport::parse(&text).unwrap();

        assert!(report.thread(Thread::Network).is_some());
        assert!(report.thread(Thread::Sync).is_none());
        assert_eq!(report.main_ticks(), None);
    }

    #[test]
    fn a_malformed_thread_is_dropped_and_the_others_kept() {
        let infinity_not_the_count = thread("svNetwork", &BOUNDS, 10, 1.0).replace(
            "le=\"+Inf\",name=\"svNetwork\"} 10",
            "le=\"+Inf\",name=\"svNetwork\"} 9",
        );
        let decreasing = thread("svSync", &BOUNDS, 10, 1.0).replace("le=\"0.25\"} 10", "le=\"0.25\"} 3");
        let text = thread("svMain", &BOUNDS, 10, 1.0) + &infinity_not_the_count + &decreasing;

        let report = TickReport::parse(&text).unwrap();

        assert_eq!(report.threads.keys().copied().collect::<Vec<_>>(), [Thread::Main]);
        let no_sum = thread("svMain", &BOUNDS, 10, 1.0).replace("tickTime_sum", "tickTime_other");
        assert_eq!(TickReport::parse(&no_sum), None);
        assert_eq!(TickReport::parse("<html>login</html>"), None);
        assert_eq!(TickReport::parse(""), None);
    }

    #[test]
    fn a_delta_counts_ticks_milliseconds_and_ticks_over_the_budget_exactly() {
        let earlier = report(&[("svMain", 100, 12.0), ("svSync", 1000, 2.0)]);
        let mut text = thread("svMain", &BOUNDS, 100, 12.0) + &thread("svSync", &BOUNDS, 1000, 2.0);
        // 100 more svMain ticks: 90 of 12 ms and 10 of 60 ms, past the 50 ms budget.
        text = text
            .replace(
                "tickTime_count{name=\"svMain\"} 100",
                "tickTime_count{name=\"svMain\"} 200",
            )
            .replace(
                "tickTime_sum{name=\"svMain\"} 1.2",
                "tickTime_sum{name=\"svMain\"} 2.88",
            );
        for (bound, within) in [
            ("0.015", 190),
            ("0.02", 190),
            ("0.03", 190),
            ("0.05", 190),
            ("0.07000000000000001", 200),
            ("0.1", 200),
            ("0.15", 200),
            ("0.25", 200),
        ] {
            text = text.replace(
                &format!("name=\"svMain\",le=\"{bound}\"}} 100"),
                &format!("name=\"svMain\",le=\"{bound}\"}} {within}"),
            );
        }
        text = text.replace(
            "le=\"+Inf\",name=\"svMain\"} 100",
            "le=\"+Inf\",name=\"svMain\"} 200",
        );
        let now = TickReport::parse(&text).unwrap();

        let deltas = now.since(&earlier);

        assert_eq!(
            deltas[&Thread::Main],
            TickDelta {
                ticks: 100,
                total_ms: 1680,
                over: Some(10)
            }
        );
        assert_eq!(
            deltas[&Thread::Sync],
            TickDelta {
                ticks: 0,
                total_ms: 0,
                over: Some(0)
            }
        );
    }

    #[test]
    fn without_the_budget_among_the_bounds_the_share_over_it_is_unknown() {
        let earlier = TickReport::parse(&thread("svMain", &OLD_BOUNDS, 10, 30.0)).unwrap();
        let now = TickReport::parse(&thread("svMain", &OLD_BOUNDS, 30, 30.0)).unwrap();

        assert_eq!(
            now.since(&earlier)[&Thread::Main],
            TickDelta {
                ticks: 20,
                total_ms: 600,
                over: Some(0)
            },
            "50 ms is among the old bounds"
        );
        let sync_earlier = TickReport::parse(&thread("svSync", &OLD_BOUNDS, 10, 3.0)).unwrap();
        let sync_now = TickReport::parse(&thread("svSync", &OLD_BOUNDS, 30, 3.0)).unwrap();
        assert_eq!(
            sync_now.since(&sync_earlier)[&Thread::Sync].over,
            None,
            "8 ms is not"
        );
    }

    #[test]
    fn a_reset_or_new_bounds_give_no_delta() {
        let earlier = report(&[("svMain", 500, 1.0), ("svNetwork", 500, 1.0)]);
        let restarted = report(&[("svMain", 20, 1.0), ("svNetwork", 600, 1.0)]);
        let rebuilt = TickReport::parse(&thread("svMain", &OLD_BOUNDS, 600, 1.0)).unwrap();

        let deltas = restarted.since(&earlier);
        assert!(!deltas.contains_key(&Thread::Main));
        assert_eq!(deltas[&Thread::Network].ticks, 100);
        assert!(rebuilt.since(&earlier).is_empty());
    }
}
