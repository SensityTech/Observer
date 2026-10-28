use jiff::Timestamp;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

/// A timestamp as the schema stores it: RFC 3339 UTC with exactly three fractional digits, so
/// that text order is time order and SQL can compare it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Utc(pub Timestamp);

impl ToSql for Utc {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(
            self.0.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        ))
    }
}

impl FromSql for Utc {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str()?.parse().map(Self).map_err(FromSqlError::other)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn timestamps_keep_milliseconds_and_sort_as_text() {
        let conn = Connection::open_in_memory().unwrap();
        let early: Timestamp = "2026-10-28T09:05:00.5Z".parse().unwrap();
        let late: Timestamp = "2026-10-28T10:00:00.123456Z".parse().unwrap();

        let text: String = conn
            .query_row("SELECT ?1", [Utc(late)], |row| row.get(0))
            .unwrap();
        let back: Utc = conn
            .query_row("SELECT ?1", [Utc(early)], |row| row.get(0))
            .unwrap();
        let ordered: bool = conn
            .query_row("SELECT ?1 < ?2", [Utc(early), Utc(late)], |row| row.get(0))
            .unwrap();

        assert_eq!(text, "2026-10-28T10:00:00.123Z");
        assert_eq!(back.0, early);
        assert!(ordered);
    }
}
