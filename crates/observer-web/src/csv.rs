//! The exports' CSV: RFC 4180, made safe to open in a spreadsheet.

use std::fmt::Write as _;

use observer_domain::audit::AuditEntry;

pub(crate) const HEADER: &str =
    "id,at,actor_kind,actor_id,actor_name,ip,user_agent,action,target,outcome,details\r\n";

/// One line per entry. Every text cell is user-influenced: a username, a user agent, a target.
pub(crate) fn rows(entries: &[AuditEntry]) -> String {
    let mut lines = String::new();
    for entry in entries {
        let cells = [
            entry.id.to_string(),
            entry.at.to_string(),
            entry.actor_kind.as_str().to_owned(),
            entry.actor_id.map(|id| id.to_string()).unwrap_or_default(),
            entry.actor_name.clone().unwrap_or_default(),
            entry.ip.clone().unwrap_or_default(),
            entry.user_agent.clone().unwrap_or_default(),
            entry.action.as_str().to_owned(),
            entry.target.clone().unwrap_or_default(),
            entry.outcome.as_str().to_owned(),
            entry.details.to_string(),
        ];
        for (index, cell) in cells.iter().enumerate() {
            if index > 0 {
                lines.push(',');
            }
            push_cell(&mut lines, cell);
        }
        lines.push_str("\r\n");
    }
    lines
}

/// One CSV line of any cells, each through the spreadsheet guard.
pub(crate) fn line(cells: &[String]) -> String {
    let mut line = String::new();
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            line.push(',');
        }
        push_cell(&mut line, cell);
    }
    line.push_str("\r\n");
    line
}

fn push_cell(line: &mut String, value: &str) {
    // A spreadsheet evaluates a cell starting with one of these as a formula; the quote makes it text.
    let guarded;
    let value = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        guarded = format!("'{value}");
        guarded.as_str()
    } else {
        value
    };
    if value.contains([',', '"', '\n', '\r']) {
        let _ = write!(line, "\"{}\"", value.replace('"', "\"\""));
    } else {
        line.push_str(value);
    }
}

#[cfg(test)]
mod tests {
    use observer_domain::audit::{ActorKind, AuditAction, Outcome};
    use serde_json::json;

    use super::*;

    fn entry(actor_name: &str, user_agent: &str, target: &str) -> AuditEntry {
        AuditEntry {
            id: 7,
            at: "2026-01-10T08:00:00.120Z".parse().unwrap(),
            actor_kind: ActorKind::User,
            actor_id: Some(2),
            actor_name: Some(actor_name.to_owned()),
            ip: Some("203.0.113.7".to_owned()),
            user_agent: Some(user_agent.to_owned()),
            action: AuditAction::UserCreated,
            target: Some(target.to_owned()),
            outcome: Outcome::Success,
            details: json!({ "userId": 3, "permissions": "80" }),
        }
    }

    #[test]
    fn a_plain_entry_is_one_line_with_quoted_json() {
        let csv = rows(&[entry("ada", "Firefox", "grace")]);

        assert_eq!(
            csv,
            "7,2026-01-10T08:00:00.12Z,user,2,ada,203.0.113.7,Firefox,user_created,grace,success,\
             \"{\"\"permissions\"\":\"\"80\"\",\"\"userId\"\":3}\"\r\n"
        );
    }

    #[test]
    fn cells_that_a_spreadsheet_would_run_are_turned_into_text() {
        let csv = rows(&[entry("=HYPERLINK(\"http://x\")", "+1+1", "@SUM(A1)")]);
        let cells: Vec<&str> = csv.trim_end().split(',').collect();

        assert_eq!(cells[4], "\"'=HYPERLINK(\"\"http://x\"\")\"");
        assert_eq!(cells[6], "'+1+1");
        assert_eq!(cells[8], "'@SUM(A1)");

        let tab = rows(&[entry("-2", "\tcmd", "\rx")]);
        assert!(tab.contains(",'-2,"));
        assert!(tab.contains(",'\tcmd,"));
        assert!(tab.contains(",\"'\rx\","));
    }

    #[test]
    fn commas_quotes_and_line_breaks_stay_inside_their_cell() {
        let csv = rows(&[entry("ada", "Mozilla/5.0 (X11, Linux)", "two\nlines")]);

        assert!(csv.contains(",\"Mozilla/5.0 (X11, Linux)\","));
        assert!(csv.contains(",\"two\nlines\","));
    }
}
