use jiff::Timestamp;
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter, Outcome};
use rusqlite::types::{FromSqlError, Type};
use rusqlite::{Connection, Row, ToSql, params, params_from_iter};
use serde_json::Value;

use crate::database::StoreError;
use crate::time::Utc;

#[derive(Debug, Clone, PartialEq)]
pub struct NewAuditEntry {
    pub at: Timestamp,
    pub actor_kind: ActorKind,
    pub actor_id: Option<i64>,
    pub actor_name: Option<String>,
    pub ip: Option<String>,
    /// At most 256 bytes; the schema refuses more.
    pub user_agent: Option<String>,
    pub action: AuditAction,
    pub target: Option<String>,
    pub outcome: Outcome,
    pub details: Value,
    pub expires_at: Timestamp,
}

pub fn insert(conn: &Connection, entry: &NewAuditEntry) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO audit_log (at, actor_kind, actor_id, actor_name, ip, user_agent, action, target,
             outcome, details, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            Utc(entry.at),
            entry.actor_kind.as_str(),
            entry.actor_id,
            entry.actor_name,
            entry.ip,
            entry.user_agent,
            entry.action.as_str(),
            entry.target,
            entry.outcome.as_str(),
            entry.details.to_string(),
            Utc(entry.expires_at),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The entries matching `filter`, newest first, with an id below `before` when given. The
/// filter's own `before` and `limit` are the caller's to apply.
pub fn page(
    conn: &Connection,
    filter: &AuditFilter,
    before: Option<i64>,
    limit: u32,
) -> Result<Vec<AuditEntry>, StoreError> {
    let mut clauses = String::new();
    let mut values: Vec<Box<dyn ToSql>> = Vec::new();
    let mut add = |clause: &str, value: Box<dyn ToSql>| {
        values.push(value);
        clauses.push_str(" AND ");
        clauses.push_str(&clause.replace('?', &format!("?{}", values.len())));
    };

    if let Some(actor) = &filter.actor {
        add("actor_name = ? COLLATE NOCASE", Box::new(actor.clone()));
    }
    if let Some(action) = filter.action {
        add("action = ?", Box::new(action.as_str()));
    }
    if let Some(outcome) = filter.outcome {
        add("outcome = ?", Box::new(outcome.as_str()));
    }
    if let Some(from) = filter.from {
        add("at >= ?", Box::new(Utc(from)));
    }
    if let Some(to) = filter.to {
        add("at < ?", Box::new(Utc(to)));
    }
    if let Some(before) = before {
        add("id < ?", Box::new(before));
    }
    values.push(Box::new(limit));

    let sql = format!(
        "SELECT id, at, actor_kind, actor_id, actor_name, ip, user_agent, action, target, outcome, details
         FROM audit_log WHERE 1{clauses} ORDER BY id DESC LIMIT ?{}",
        values.len()
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), read)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The entries of `actions` from `from` to `to`, oldest first, at most `limit`.
pub fn of_actions(
    conn: &Connection,
    actions: &[AuditAction],
    from: Timestamp,
    to: Timestamp,
    limit: u32,
) -> Result<Vec<AuditEntry>, StoreError> {
    let names: Vec<&str> = actions.iter().map(|action| action.as_str()).collect();
    let placeholders = (0..names.len())
        .map(|index| format!("?{}", index + 4))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT id, at, actor_kind, actor_id, actor_name, ip, user_agent, action, target, outcome, details
         FROM audit_log WHERE at >= ?1 AND at <= ?2 AND action IN ({placeholders}) ORDER BY id LIMIT ?3"
    );
    let mut values: Vec<Box<dyn ToSql>> = vec![Box::new(Utc(from)), Box::new(Utc(to)), Box::new(limit)];
    values.extend(names.into_iter().map(|name| Box::new(name) as Box<dyn ToSql>));
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), read)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The entries written after `after`, oldest first.
pub fn after(conn: &Connection, after: i64, limit: u32) -> Result<Vec<AuditEntry>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, at, actor_kind, actor_id, actor_name, ip, user_agent, action, target, outcome, details
         FROM audit_log WHERE id > ?1 ORDER BY id LIMIT ?2",
    )?;
    let rows = statement.query_map(params![after, limit], read)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The id of the newest entry, or `None` while the log is empty.
pub fn latest_id(conn: &Connection) -> Result<Option<i64>, StoreError> {
    Ok(conn.query_row("SELECT max(id) FROM audit_log", [], |row| row.get(0))?)
}

/// Lets the retention job delete what expired before `cutoff`. The schema refuses a cutoff that
/// moves back or lies in the future.
pub fn advance_cutoff(conn: &Connection, cutoff: Timestamp) -> Result<(), StoreError> {
    conn.execute("UPDATE audit_purge SET cutoff = ?1 WHERE id = 1", [Utc(cutoff)])?;
    Ok(())
}

/// Deletes the entries that expired before the cutoff.
pub fn purge(conn: &Connection) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM audit_log WHERE expires_at <= (SELECT cutoff FROM audit_purge WHERE id = 1)",
        [],
    )?)
}

fn read(row: &Row<'_>) -> rusqlite::Result<AuditEntry> {
    let name = |index: usize| -> rusqlite::Result<String> { row.get(index) };
    let invalid = |index: usize, what: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Text,
            Box::new(FromSqlError::Other(format!("unknown {what}").into())),
        )
    };

    let details: String = row.get(10)?;
    Ok(AuditEntry {
        id: row.get(0)?,
        at: row.get::<_, Utc>(1)?.0,
        actor_kind: ActorKind::from_name(&name(2)?).ok_or_else(|| invalid(2, "actor kind"))?,
        actor_id: row.get(3)?,
        actor_name: row.get(4)?,
        ip: row.get(5)?,
        user_agent: row.get(6)?,
        action: AuditAction::from_name(&name(7)?).ok_or_else(|| invalid(7, "audit action"))?,
        target: row.get(8)?,
        outcome: Outcome::from_name(&name(9)?).ok_or_else(|| invalid(9, "outcome"))?,
        details: serde_json::from_str(&details)
            .map_err(|error| rusqlite::Error::FromSqlConversionFailure(10, Type::Text, Box::new(error)))?,
    })
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;
    use serde_json::json;

    use super::*;
    use crate::database::memory;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn entry(written: &str, action: AuditAction, actor: &str, outcome: Outcome) -> NewAuditEntry {
        let written = at(written);
        NewAuditEntry {
            at: written,
            actor_kind: ActorKind::User,
            actor_id: Some(1),
            actor_name: Some(actor.to_owned()),
            ip: Some("203.0.113.7".to_owned()),
            user_agent: Some("Mozilla/5.0".to_owned()),
            action,
            target: None,
            outcome,
            details: json!({ "sessionId": 4 }),
            expires_at: written + SignedDuration::from_hours(24 * 90),
        }
    }

    #[test]
    fn entries_after_an_id_come_oldest_first_up_to_the_limit() {
        let conn = memory();
        assert_eq!(latest_id(&conn).unwrap(), None);
        let ids: Vec<i64> = [
            "2026-01-10T08:00:00Z",
            "2026-01-10T08:01:00Z",
            "2026-01-10T08:02:00Z",
        ]
        .into_iter()
        .map(|written| {
            insert(
                &conn,
                &entry(written, AuditAction::LoginSucceeded, "ada", Outcome::Success),
            )
            .unwrap()
        })
        .collect();

        let ids_after = |after, limit| -> Vec<i64> {
            self::after(&conn, after, limit)
                .unwrap()
                .iter()
                .map(|entry| entry.id)
                .collect()
        };

        assert_eq!(ids_after(0, 10), ids);
        assert_eq!(ids_after(ids[0], 1), [ids[1]]);
        assert_eq!(ids_after(ids[2], 10), Vec::<i64>::new());
        assert_eq!(latest_id(&conn).unwrap(), Some(ids[2]));
    }

    #[test]
    fn audit_entries_cannot_be_updated() {
        let conn = memory();
        let id = insert(
            &conn,
            &entry(
                "2026-01-10T08:00:00Z",
                AuditAction::LoginSucceeded,
                "ada",
                Outcome::Success,
            ),
        )
        .unwrap();

        let error = conn
            .execute("UPDATE audit_log SET outcome = 'failure' WHERE id = ?1", [id])
            .unwrap_err();

        assert!(error.to_string().contains("append-only"), "{error}");
    }

    #[test]
    fn audit_entries_cannot_be_deleted_until_the_cutoff_passes_their_expiry() {
        let conn = memory();
        let old = insert(
            &conn,
            &entry(
                "2025-01-10T08:00:00Z",
                AuditAction::LoginSucceeded,
                "ada",
                Outcome::Success,
            ),
        )
        .unwrap();
        let recent = insert(
            &conn,
            &entry(
                "2026-01-10T08:00:00Z",
                AuditAction::Logout,
                "ada",
                Outcome::Success,
            ),
        )
        .unwrap();

        let refused = conn
            .execute("DELETE FROM audit_log WHERE id = ?1", [old])
            .unwrap_err();
        assert!(refused.to_string().contains("retention"), "{refused}");
        assert_eq!(purge(&conn).unwrap(), 0);

        // The old entry expired in April 2025, the recent one only in April 2026.
        advance_cutoff(&conn, at("2025-06-01T00:00:00Z")).unwrap();
        assert_eq!(purge(&conn).unwrap(), 1);
        let refused = conn
            .execute("DELETE FROM audit_log WHERE id = ?1", [recent])
            .unwrap_err();
        assert!(refused.to_string().contains("retention"), "{refused}");

        let left = page(&conn, &AuditFilter::default(), None, 10).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, recent);
    }

    #[test]
    fn the_cutoff_only_moves_forward_and_never_past_the_present() {
        let conn = memory();
        advance_cutoff(&conn, at("2025-06-01T00:00:00Z")).unwrap();

        assert!(advance_cutoff(&conn, at("2025-05-01T00:00:00Z")).is_err());
        assert!(advance_cutoff(&conn, Timestamp::now() + SignedDuration::from_hours(1)).is_err());
        assert!(conn.execute("DELETE FROM audit_purge", []).is_err());
        advance_cutoff(&conn, at("2025-07-01T00:00:00Z")).unwrap();
    }

    #[test]
    fn an_oversized_user_agent_or_invalid_details_are_refused() {
        let conn = memory();
        let mut long = entry(
            "2026-01-10T08:00:00Z",
            AuditAction::Logout,
            "ada",
            Outcome::Success,
        );
        long.user_agent = Some("x".repeat(257));

        assert!(insert(&conn, &long).is_err());
        let invalid_details = conn.execute(
            "INSERT INTO audit_log (at, actor_kind, action, outcome, details, expires_at)
             VALUES ('2026-01-10T08:00:00.000Z', 'system', 'logout', 'success', '{oops',
                 '2027-01-10T08:00:00.000Z')",
            [],
        );
        assert!(invalid_details.is_err());
    }

    #[test]
    fn pages_are_filtered_newest_first_and_continue_below_the_last_id() {
        let conn = memory();
        let rows = [
            (
                "2026-01-10T08:00:00Z",
                AuditAction::LoginSucceeded,
                "ada",
                Outcome::Success,
            ),
            (
                "2026-01-10T09:00:00Z",
                AuditAction::LoginSucceeded,
                "Grace",
                Outcome::Success,
            ),
            (
                "2026-01-10T10:00:00Z",
                AuditAction::UserCreated,
                "ada",
                Outcome::Success,
            ),
            (
                "2026-01-10T11:00:00Z",
                AuditAction::LoginSucceeded,
                "ada",
                Outcome::Success,
            ),
            (
                "2026-01-10T12:00:00Z",
                AuditAction::PermissionDenied,
                "grace",
                Outcome::Denied,
            ),
        ];
        let ids: Vec<i64> = rows
            .iter()
            .map(|&(written, action, actor, outcome)| {
                insert(&conn, &entry(written, action, actor, outcome)).unwrap()
            })
            .collect();
        let found = |filter: &AuditFilter, before, limit| -> Vec<i64> {
            page(&conn, filter, before, limit)
                .unwrap()
                .iter()
                .map(|entry| entry.id)
                .collect()
        };

        let logins = AuditFilter {
            action: Some(AuditAction::LoginSucceeded),
            ..AuditFilter::default()
        };
        assert_eq!(found(&logins, None, 2), [ids[3], ids[1]]);
        assert_eq!(found(&logins, Some(ids[1]), 2), [ids[0]]);

        let grace = AuditFilter {
            actor: Some("GRACE".to_owned()),
            ..AuditFilter::default()
        };
        assert_eq!(found(&grace, None, 10), [ids[4], ids[1]]);

        let morning = AuditFilter {
            from: Some(at("2026-01-10T09:00:00Z")),
            to: Some(at("2026-01-10T11:00:00Z")),
            outcome: Some(Outcome::Success),
            ..AuditFilter::default()
        };
        assert_eq!(found(&morning, None, 10), [ids[2], ids[1]]);

        let entry = &page(&conn, &AuditFilter::default(), None, 1).unwrap()[0];
        assert_eq!(entry.action, AuditAction::PermissionDenied);
        assert_eq!(entry.details, json!({ "sessionId": 4 }));
        assert_eq!(entry.at, at("2026-01-10T12:00:00Z"));
    }
}
