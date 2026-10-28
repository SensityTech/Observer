//! When FXServer restarts by itself: the schedule's settings and their checks, what the schedule
//! keeps between two runs of Observer, and the status the panel shows of it.

use std::collections::HashSet;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The most fixed times a schedule takes.
pub const MAX_TIMES: usize = 48;
/// Fixed times are at least this many minutes apart, across midnight too.
pub const MIN_GAP_MINUTES: u32 = 30;
pub const MIN_INTERVAL_MINUTES: u32 = 30;
/// Seven days.
pub const MAX_INTERVAL_MINUTES: u32 = 10_080;
pub const MAX_WARNING_MINUTES: u32 = 240;
pub const MAX_WARNINGS: usize = 16;
pub const MAX_SKIP_AFTER_START: u32 = 720;
const MAX_ZONE_CHARS: usize = 64;
/// txAdmin's warnings, which resources written for it expect.
pub const DEFAULT_WARNINGS: [u32; 8] = [30, 15, 10, 5, 4, 3, 2, 1];

/// How the schedule picks its restarts. Both modes keep their values, so switching back loses
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ScheduleMode {
    /// At each of `times`, every day, in the schedule's zone.
    FixedTimes,
    /// Every `intervalMinutes` of real time from each start of FXServer.
    Interval,
}

impl ScheduleMode {
    pub const ALL: [Self; 2] = [Self::FixedTimes, Self::Interval];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FixedTimes => "fixed_times",
            Self::Interval => "interval",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ScheduleSettings {
    pub enabled: bool,
    pub mode: ScheduleMode,
    /// `HH:MM`, 24-hour, in the schedule's zone.
    pub times: Vec<String>,
    pub interval_minutes: u32,
    /// An IANA name, or null for the host's zone.
    pub time_zone: Option<String>,
    /// Minutes before a restart at which resources hear of it.
    pub warnings: Vec<u32>,
    /// A fixed time less than this many minutes after a start the schedule did not make is
    /// skipped; 0 turns the rule off.
    pub skip_after_start: u32,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ScheduleMode::FixedTimes,
            times: Vec::new(),
            interval_minutes: 360,
            time_zone: None,
            warnings: DEFAULT_WARNINGS.to_vec(),
            skip_after_start: 120,
        }
    }
}

/// `PUT /api/servers/default/schedule`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ScheduleChange {
    pub settings: ScheduleSettings,
    /// The version the change was made from, null before the first save.
    pub version: Option<i64>,
}

/// The settings as stored, with the version a change must name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSchedule {
    pub settings: ScheduleSettings,
    pub version: i64,
}

/// One thing wrong with schedule settings, which the panel shows next to its field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScheduleIssue {
    pub field: ScheduleField,
    /// The position in its list, for `time` and `warning`.
    pub index: Option<usize>,
    pub problem: ScheduleProblem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ScheduleField {
    /// The list of fixed times as a whole.
    Times,
    /// One of the fixed times.
    Time,
    IntervalMinutes,
    /// One of the warnings.
    Warning,
    /// The list of warnings as a whole.
    Warnings,
    SkipAfterStart,
    TimeZone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ScheduleProblem {
    /// No fixed time, with the schedule on in that mode.
    Required,
    TooMany,
    /// Not `HH:MM` on a 24-hour clock.
    InvalidTime,
    Duplicate,
    /// Less than 30 minutes after the time before it, across midnight too.
    TooClose,
    OutOfRange,
    UnknownTimeZone,
}

impl ScheduleSettings {
    /// Everything that keeps these settings from being stored, at most one problem per field and
    /// position. Both modes' values are checked, since both are kept.
    pub fn issues(&self) -> Vec<ScheduleIssue> {
        let mut issues = Vec::new();
        let mut note = |field, index, problem| {
            issues.push(ScheduleIssue {
                field,
                index,
                problem,
            });
        };

        if self.times.len() > MAX_TIMES {
            note(ScheduleField::Times, None, ScheduleProblem::TooMany);
        } else if self.times.is_empty() && self.enabled && self.mode == ScheduleMode::FixedTimes {
            note(ScheduleField::Times, None, ScheduleProblem::Required);
        }
        let mut valid = Vec::new();
        let mut seen = HashSet::new();
        for (index, time) in self.times.iter().take(MAX_TIMES).enumerate() {
            match parse_time(time) {
                None => note(ScheduleField::Time, Some(index), ScheduleProblem::InvalidTime),
                Some(minute) if !seen.insert(minute) => {
                    note(ScheduleField::Time, Some(index), ScheduleProblem::Duplicate);
                }
                Some(minute) => valid.push((minute, index)),
            }
        }
        valid.sort_unstable();
        for (position, &(minute, index)) in valid.iter().enumerate() {
            let previous = match position {
                0 if valid.len() > 1 => valid[valid.len() - 1].0 - 24 * 60,
                0 => continue,
                _ => valid[position - 1].0,
            };
            if minute - previous < i32::try_from(MIN_GAP_MINUTES).unwrap_or(i32::MAX) {
                note(ScheduleField::Time, Some(index), ScheduleProblem::TooClose);
            }
        }

        if !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&self.interval_minutes) {
            note(ScheduleField::IntervalMinutes, None, ScheduleProblem::OutOfRange);
        }
        if self.warnings.len() > MAX_WARNINGS {
            note(ScheduleField::Warnings, None, ScheduleProblem::TooMany);
        }
        let mut tiers = HashSet::new();
        for (index, &minutes) in self.warnings.iter().take(MAX_WARNINGS).enumerate() {
            if !(1..=MAX_WARNING_MINUTES).contains(&minutes) {
                note(ScheduleField::Warning, Some(index), ScheduleProblem::OutOfRange);
            } else if !tiers.insert(minutes) {
                note(ScheduleField::Warning, Some(index), ScheduleProblem::Duplicate);
            }
        }
        if self.skip_after_start > MAX_SKIP_AFTER_START {
            note(ScheduleField::SkipAfterStart, None, ScheduleProblem::OutOfRange);
        }
        if let Some(zone) = &self.time_zone
            && (zone.chars().count() > MAX_ZONE_CHARS || TimeZone::get(zone).is_err())
        {
            note(ScheduleField::TimeZone, None, ScheduleProblem::UnknownTimeZone);
        }
        issues
    }

    /// The settings as they are stored: times ascending, warnings descending.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.times.sort_by_key(|time| parse_time(time));
        self.warnings.sort_unstable_by(|a, b| b.cmp(a));
        self
    }
}

/// The minute of the day of a `HH:MM` time on a 24-hour clock.
pub fn parse_time(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
    {
        return None;
    }
    let digit = |index: usize| i32::from(bytes[index] - b'0');
    let hour = digit(0) * 10 + digit(1);
    let minute = digit(3) * 10 + digit(4);
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

/// Where a restart comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RestartSource {
    Schedule,
    /// A single restart someone planned.
    OneOff,
}

impl RestartSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::OneOff => "oneOff",
        }
    }
}

/// What the schedule keeps between two runs of Observer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulePlan {
    /// One occurrence of the schedule that will not happen.
    pub skip: Option<PlannedRestart>,
    pub delay: Option<PlannedDelay>,
    pub one_off: Option<PlannedRestart>,
    /// The latest occurrence fired or given up on: nothing at or before it fires again, so a
    /// clock set back replays nothing.
    pub settled: Option<Timestamp>,
}

/// A restart, or a skipped occurrence, and who asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRestart {
    pub at: Timestamp,
    pub by: String,
}

/// An occurrence of the schedule moved later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDelay {
    pub occurrence: Timestamp,
    pub to: Timestamp,
    pub by: String,
}

/// The `server.schedule` topic, and the answer to every change of the schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScheduleStatus {
    pub settings: ScheduleSettings,
    /// Null until the settings are first saved.
    pub version: Option<i64>,
    /// The IANA name of the zone in force: the settings', or the host's.
    pub zone: String,
    pub next: Option<NextRestart>,
    /// The next five fixed-time occurrences, or the next interval one while FXServer runs.
    pub upcoming: Vec<Occurrence>,
    pub one_off: Option<OneOffRestart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NextRestart {
    pub at: Timestamp,
    pub source: RestartSource,
    /// Who planned the one-off, or delayed the occurrence.
    pub by: Option<String>,
    /// The occurrence's own time, when it was delayed.
    pub delayed_from: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Occurrence {
    pub at: Timestamp,
    /// The configured `HH:MM` it comes from, which a clock change may move; null in interval mode.
    pub time: Option<String>,
    pub skipped: Option<Skipped>,
    pub delayed: Option<Delayed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Skipped {
    Manual {
        by: String,
    },
    /// FXServer started a moment before, for another reason than this schedule.
    RecentStart {
        started_at: Timestamp,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Delayed {
    pub to: Timestamp,
    pub by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct OneOffRestart {
    pub at: Timestamp,
    pub by: String,
}

/// `POST /api/servers/default/schedule/delay`.
#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct DelayRequest {
    /// 1 to 720.
    pub minutes: u32,
}

/// `PUT /api/servers/default/schedule/once`: either field, not both.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct OneOffRequest {
    /// 1 to 1,440.
    #[serde(default)]
    pub in_minutes: Option<u32>,
    /// `HH:MM`: the first such time in the schedule's zone at least a minute ahead.
    #[serde(default)]
    pub at: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixed(times: &[&str]) -> ScheduleSettings {
        ScheduleSettings {
            enabled: true,
            times: times.iter().map(|&time| time.to_owned()).collect(),
            ..ScheduleSettings::default()
        }
    }

    fn issue(field: ScheduleField, index: Option<usize>, problem: ScheduleProblem) -> ScheduleIssue {
        ScheduleIssue {
            field,
            index,
            problem,
        }
    }

    #[test]
    fn the_defaults_and_a_daily_restart_have_no_issue() {
        assert_eq!(ScheduleSettings::default().issues(), []);
        assert_eq!(fixed(&["04:00"]).issues(), []);
        assert_eq!(fixed(&["00:00", "06:00", "12:00", "18:00", "23:30"]).issues(), []);
    }

    #[test]
    fn a_schedule_on_in_fixed_times_needs_a_time() {
        let interval = ScheduleSettings {
            mode: ScheduleMode::Interval,
            ..fixed(&[])
        };

        assert_eq!(
            fixed(&[]).issues(),
            [issue(ScheduleField::Times, None, ScheduleProblem::Required)]
        );
        assert_eq!(interval.issues(), []);
        assert_eq!(
            ScheduleSettings {
                enabled: false,
                ..fixed(&[])
            }
            .issues(),
            []
        );
    }

    #[test]
    fn times_are_24_hour_hh_mm() {
        for invalid in [
            "4:00",
            "24:00",
            "04:60",
            "04-00",
            "0400",
            " 04:00",
            "04:00 ",
            "٠٤:٠٠",
            "",
        ] {
            assert_eq!(
                fixed(&[invalid]).issues(),
                [issue(ScheduleField::Time, Some(0), ScheduleProblem::InvalidTime)],
                "{invalid:?}"
            );
        }
        assert_eq!(parse_time("23:59"), Some(23 * 60 + 59));
        assert_eq!(parse_time("00:00"), Some(0));
    }

    #[test]
    fn times_are_unique_and_half_an_hour_apart_across_midnight_too() {
        assert_eq!(
            fixed(&["04:00", "04:00"]).issues(),
            [issue(ScheduleField::Time, Some(1), ScheduleProblem::Duplicate)]
        );
        assert_eq!(
            fixed(&["04:00", "04:29"]).issues(),
            [issue(ScheduleField::Time, Some(1), ScheduleProblem::TooClose)]
        );
        assert_eq!(fixed(&["04:00", "04:30"]).issues(), []);
        assert_eq!(
            fixed(&["00:10", "23:50"]).issues(),
            [issue(ScheduleField::Time, Some(0), ScheduleProblem::TooClose)]
        );
        assert_eq!(fixed(&["23:45", "00:15"]).issues(), []);
    }

    #[test]
    fn at_most_48_times_are_taken() {
        let times: Vec<String> = (0..49)
            .map(|half| format!("{:02}:{:02}", half / 2 % 24, half % 2 * 30))
            .collect();
        let settings = ScheduleSettings { times, ..fixed(&[]) };

        assert_eq!(
            settings.issues(),
            [issue(ScheduleField::Times, None, ScheduleProblem::TooMany)]
        );
    }

    #[test]
    fn numbers_stay_within_their_ranges() {
        let settings = ScheduleSettings {
            interval_minutes: 29,
            warnings: vec![30, 0, 241, 30],
            skip_after_start: 721,
            ..ScheduleSettings::default()
        };

        assert_eq!(
            settings.issues(),
            [
                issue(ScheduleField::IntervalMinutes, None, ScheduleProblem::OutOfRange),
                issue(ScheduleField::Warning, Some(1), ScheduleProblem::OutOfRange),
                issue(ScheduleField::Warning, Some(2), ScheduleProblem::OutOfRange),
                issue(ScheduleField::Warning, Some(3), ScheduleProblem::Duplicate),
                issue(ScheduleField::SkipAfterStart, None, ScheduleProblem::OutOfRange),
            ]
        );
        let widest = ScheduleSettings {
            interval_minutes: 10_080,
            warnings: vec![240, 1],
            skip_after_start: 720,
            ..ScheduleSettings::default()
        };
        assert_eq!(widest.issues(), []);
        let many = ScheduleSettings {
            warnings: (1..=17).collect(),
            ..ScheduleSettings::default()
        };
        assert_eq!(
            many.issues(),
            [issue(ScheduleField::Warnings, None, ScheduleProblem::TooMany)]
        );
    }

    #[test]
    fn the_zone_is_one_the_time_zone_database_knows() {
        let zone = |name: &str| ScheduleSettings {
            time_zone: Some(name.to_owned()),
            ..ScheduleSettings::default()
        };

        assert_eq!(zone("Europe/Paris").issues(), []);
        assert_eq!(zone("UTC").issues(), []);
        for unknown in ["Europe/Atlantis", "", "../../etc/passwd", &"A".repeat(65)] {
            assert_eq!(
                zone(unknown).issues(),
                [issue(
                    ScheduleField::TimeZone,
                    None,
                    ScheduleProblem::UnknownTimeZone
                )],
                "{unknown:?}"
            );
        }
    }

    #[test]
    fn settings_are_stored_with_times_ascending_and_warnings_descending() {
        let settings = ScheduleSettings {
            warnings: vec![1, 30, 5],
            ..fixed(&["16:00", "04:00", "10:00"])
        };

        let normalized = settings.normalized();

        assert_eq!(normalized.times, ["04:00", "10:00", "16:00"]);
        assert_eq!(normalized.warnings, [30, 5, 1]);
    }

    #[test]
    fn the_wire_form_is_camel_case_and_refuses_unknown_fields() {
        let wire = json!({
            "enabled": true,
            "mode": "fixedTimes",
            "times": ["04:00"],
            "intervalMinutes": 360,
            "timeZone": null,
            "warnings": [30, 15, 10, 5, 4, 3, 2, 1],
            "skipAfterStart": 120,
        });

        assert_eq!(
            serde_json::from_value::<ScheduleSettings>(wire.clone()).unwrap(),
            fixed(&["04:00"])
        );
        assert_eq!(serde_json::to_value(fixed(&["04:00"])).unwrap(), wire);
        let mut extra = wire;
        extra["every"] = json!(6);
        assert!(serde_json::from_value::<ScheduleSettings>(extra).is_err());
    }

    #[test]
    fn a_skip_names_its_kind() {
        assert_eq!(
            serde_json::to_value(Skipped::RecentStart {
                started_at: "2026-10-29T02:50:00Z".parse().unwrap()
            })
            .unwrap(),
            json!({ "kind": "recentStart", "startedAt": "2026-10-29T02:50:00Z" })
        );
        assert_eq!(
            serde_json::to_value(Skipped::Manual { by: "ada".to_owned() }).unwrap(),
            json!({ "kind": "manual", "by": "ada" })
        );
    }
}
