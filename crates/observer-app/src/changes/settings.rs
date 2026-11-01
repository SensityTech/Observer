//! The owner's settings of file changes, one `settings` row each (`changes.max_text`), and the two
//! keys of the instance, written once with random bytes and never sent.

use observer_domain::changes::ChangesSettings;
use observer_fxserver::changes::Keys;
use observer_store::{Connection, settings};
use serde_json::Value;

use crate::Error;
use crate::secrets::{from_hex, hex, random};

const PREFIX: &str = "changes.";
const CONTENT_KEY: &str = "changes.content_key";
const TAG_KEY: &str = "changes.tag_key";

/// The stored settings, each missing one at its default.
pub(crate) fn load(conn: &Connection) -> Result<ChangesSettings, Error> {
    let Value::Object(mut fields) = serde_json::to_value(ChangesSettings::default()).unwrap_or_default()
    else {
        return Ok(ChangesSettings::default());
    };
    for (field, value) in &mut fields {
        if let Some(stored) = settings::get(conn, &key(field))?
            && let Ok(stored) = serde_json::from_str(&stored)
        {
            *value = stored;
        }
    }
    // A row a newer Observer wrote, or one edited by hand, leaves the defaults in force.
    Ok(serde_json::from_value(Value::Object(fields)).unwrap_or_default())
}

pub(crate) fn save(conn: &Connection, changed: &ChangesSettings) -> Result<(), Error> {
    if let Value::Object(fields) = serde_json::to_value(changed).unwrap_or_default() {
        for (field, value) in fields {
            settings::set(conn, &key(&field), &value.to_string())?;
        }
    }
    Ok(())
}

/// `maxText` is stored as `changes.max_text`.
fn key(field: &str) -> String {
    let mut key = String::from(PREFIX);
    for c in field.chars() {
        if c.is_ascii_uppercase() {
            key.push('_');
            key.push(c.to_ascii_lowercase());
        } else {
            key.push(c);
        }
    }
    key
}

/// The instance's keys, created from the operating system's random source the first time. Two
/// processes creating them at once end up with the same keys.
pub(crate) fn keys(conn: &Connection) -> Result<Keys, Error> {
    let read = |name: &str| -> Result<[u8; 32], Error> {
        let fresh = hex(&random::<32>()?);
        let stored = settings::create(conn, name, &fresh)?;
        from_hex(&stored).ok_or_else(|| {
            Error::Io(std::io::Error::other(format!(
                "the setting {name} is not a 32-byte hex key"
            )))
        })
    };
    Ok(Keys {
        content: read(CONTENT_KEY)?,
        tag: read(TAG_KEY)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_stored_under_snake_case_keys() {
        assert_eq!(key("maxText"), "changes.max_text");
        assert_eq!(key("enabled"), "changes.enabled");
        assert_eq!(key("startEvent"), "changes.start_event");
    }
}
