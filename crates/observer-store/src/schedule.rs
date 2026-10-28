use jiff::Timestamp;
use observer_domain::schedule::{
    PlannedDelay, PlannedRestart, ScheduleMode, SchedulePlan, ScheduleSettings, StoredSchedule,
};
use rusqlite::types::{FromSqlError, Type};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

/// A server's schedule settings, or `None` until they are first saved.
pub fn settings(conn: &Connection, server_id: &str) -> Result<Option<StoredSchedule>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT enabled, mode, times, interval_minutes, time_zone, warnings, skip_after_start, version
             FROM schedule_settings WHERE server_id = ?1",
            [server_id],
            read_settings,
        )
        .optional()?)
}

/// Stores the settings under their version, which the caller has already moved on.
pub fn save_settings(
    conn: &Connection,
    server_id: &str,
    stored: &StoredSchedule,
    now: Timestamp,
) -> Result<(), StoreError> {
    let settings = &stored.settings;
    conn.execute(
        "INSERT INTO schedule_settings (server_id, enabled, mode, times, interval_minutes, time_zone,
             warnings, skip_after_start, version, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (server_id) DO UPDATE SET enabled = excluded.enabled, mode = excluded.mode,
             times = excluded.times, interval_minutes = excluded.interval_minutes,
             time_zone = excluded.time_zone, warnings = excluded.warnings,
             skip_after_start = excluded.skip_after_start, version = excluded.version,
             updated_at = excluded.updated_at",
        params![
            server_id,
            settings.enabled,
            settings.mode.as_str(),
            serde_json::to_string(&settings.times).map_err(conversion)?,
            settings.interval_minutes,
            settings.time_zone,
            serde_json::to_string(&settings.warnings).map_err(conversion)?,
            settings.skip_after_start,
            stored.version,
            Utc(now),
        ],
    )?;
    Ok(())
}

/// What the schedule planned for a server; empty until something was.
pub fn plan(conn: &Connection, server_id: &str) -> Result<SchedulePlan, StoreError> {
    Ok(conn
        .query_row(
            "SELECT skip_at, skip_by, delay_occurrence, delay_to, delay_by, one_off_at, one_off_by,
                 settled_at
             FROM schedule_plan WHERE server_id = ?1",
            [server_id],
            read_plan,
        )
        .optional()?
        .unwrap_or_default())
}

pub fn save_plan(
    conn: &Connection,
    server_id: &str,
    plan: &SchedulePlan,
    now: Timestamp,
) -> Result<(), StoreError> {
    let skip = plan.skip.as_ref();
    let delay = plan.delay.as_ref();
    let one_off = plan.one_off.as_ref();
    conn.execute(
        "INSERT INTO schedule_plan (server_id, skip_at, skip_by, delay_occurrence, delay_to, delay_by,
             one_off_at, one_off_by, settled_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (server_id) DO UPDATE SET skip_at = excluded.skip_at, skip_by = excluded.skip_by,
             delay_occurrence = excluded.delay_occurrence, delay_to = excluded.delay_to,
             delay_by = excluded.delay_by, one_off_at = excluded.one_off_at,
             one_off_by = excluded.one_off_by, settled_at = excluded.settled_at,
             updated_at = excluded.updated_at",
        params![
            server_id,
            skip.map(|skip| Utc(skip.at)),
            skip.map(|skip| &skip.by),
            delay.map(|delay| Utc(delay.occurrence)),
            delay.map(|delay| Utc(delay.to)),
            delay.map(|delay| &delay.by),
            one_off.map(|one_off| Utc(one_off.at)),
            one_off.map(|one_off| &one_off.by),
            plan.settled.map(Utc),
            Utc(now),
        ],
    )?;
    Ok(())
}

fn conversion(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn read_settings(row: &Row<'_>) -> rusqlite::Result<StoredSchedule> {
    let mode: String = row.get(1)?;
    let list = |index: usize| -> rusqlite::Result<String> { row.get(index) };
    let parse_error = |index: usize| {
        move |error| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    };
    let settings = ScheduleSettings {
        enabled: row.get(0)?,
        mode: ScheduleMode::from_name(&mode).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                Type::Text,
                Box::new(FromSqlError::Other("unknown schedule mode".into())),
            )
        })?,
        times: serde_json::from_str(&list(2)?).map_err(parse_error(2))?,
        interval_minutes: row.get(3)?,
        time_zone: row.get(4)?,
        warnings: serde_json::from_str(&list(5)?).map_err(parse_error(5))?,
        skip_after_start: row.get(6)?,
    };
    Ok(StoredSchedule {
        settings,
        version: row.get(7)?,
    })
}

fn read_plan(row: &Row<'_>) -> rusqlite::Result<SchedulePlan> {
    let at = |index: usize| -> rusqlite::Result<Option<Timestamp>> {
        Ok(row.get::<_, Option<Utc>>(index)?.map(|at| at.0))
    };
    let planned = |at: Option<Timestamp>, by: Option<String>| Some(PlannedRestart { at: at?, by: by? });
    let delay = match (at(2)?, at(3)?, row.get(4)?) {
        (Some(occurrence), Some(to), Some(by)) => Some(PlannedDelay { occurrence, to, by }),
        _ => None,
    };
    Ok(SchedulePlan {
        skip: planned(at(0)?, row.get(1)?),
        delay,
        one_off: planned(at(5)?, row.get(6)?),
        settled: at(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn settings_round_trip_and_are_replaced_on_save() {
        let conn = memory();
        let now = at("2026-10-28T09:00:00Z");
        assert_eq!(settings(&conn, "default").unwrap(), None);

        let first = StoredSchedule {
            settings: ScheduleSettings {
                enabled: true,
                times: vec!["04:00".to_owned(), "16:00".to_owned()],
                time_zone: Some("Europe/Paris".to_owned()),
                ..ScheduleSettings::default()
            },
            version: 1,
        };
        save_settings(&conn, "default", &first, now).unwrap();
        assert_eq!(settings(&conn, "default").unwrap(), Some(first.clone()));

        let second = StoredSchedule {
            settings: ScheduleSettings {
                mode: ScheduleMode::Interval,
                interval_minutes: 480,
                time_zone: None,
                warnings: vec![10, 1],
                skip_after_start: 0,
                ..first.settings
            },
            version: 2,
        };
        save_settings(&conn, "default", &second, now).unwrap();
        assert_eq!(settings(&conn, "default").unwrap(), Some(second));
    }

    #[test]
    fn a_plan_round_trips_down_to_the_millisecond() {
        let conn = memory();
        let now = at("2026-10-28T09:00:00Z");
        assert_eq!(plan(&conn, "default").unwrap(), SchedulePlan::default());

        let full = SchedulePlan {
            skip: Some(PlannedRestart {
                at: at("2026-10-29T03:00:00Z"),
                by: "ada".to_owned(),
            }),
            delay: Some(PlannedDelay {
                occurrence: at("2026-10-29T09:00:00Z"),
                to: at("2026-10-29T09:30:00.250Z"),
                by: "grace".to_owned(),
            }),
            one_off: Some(PlannedRestart {
                at: at("2026-10-28T10:17:00Z"),
                by: "ada".to_owned(),
            }),
            settled: Some(at("2026-10-28T03:00:00Z")),
        };
        save_plan(&conn, "default", &full, now).unwrap();
        assert_eq!(plan(&conn, "default").unwrap(), full);

        let cleared = SchedulePlan {
            settled: full.settled,
            ..SchedulePlan::default()
        };
        save_plan(&conn, "default", &cleared, now).unwrap();
        assert_eq!(plan(&conn, "default").unwrap(), cleared);
    }

    #[test]
    fn the_schema_refuses_half_a_plan_and_settings_out_of_range() {
        let conn = memory();
        let plan_row = |columns: &str, values: &str| {
            conn.execute(
                &format!(
                    "INSERT OR REPLACE INTO schedule_plan (server_id, {columns}, updated_at)
                     VALUES ('default', {values}, '2026-10-28T09:00:00.000Z')"
                ),
                [],
            )
        };
        assert!(plan_row("skip_at", "'2026-10-29T03:00:00.000Z'").is_err());
        assert!(plan_row("one_off_by", "'ada'").is_err());
        assert!(
            plan_row(
                "delay_occurrence, delay_to, delay_by",
                "'2026-10-29T03:00:00.000Z', '2026-10-29T02:00:00.000Z', 'ada'"
            )
            .is_err()
        );
        plan_row("skip_at, skip_by", "'2026-10-29T03:00:00.000Z', 'ada'").unwrap();

        let settings_row = |mode: &str, interval: i64, times: &str| {
            conn.execute(
                "INSERT OR REPLACE INTO schedule_settings (server_id, enabled, mode, times, interval_minutes,
                     warnings, skip_after_start, updated_at)
                 VALUES ('default', 1, ?1, ?3, ?2, '[30]', 120, '2026-10-28T09:00:00.000Z')",
                params![mode, interval, times],
            )
        };
        assert!(settings_row("weekly", 360, "[]").is_err());
        assert!(settings_row("interval", 29, "[]").is_err());
        assert!(settings_row("interval", 360, "{}").is_err());
        settings_row("interval", 360, "[\"04:00\"]").unwrap();
    }
}
