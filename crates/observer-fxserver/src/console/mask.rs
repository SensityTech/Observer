//! Secrets hidden from every console line before it is stored, logged or sent anywhere.

/// Convars whose value is a secret: the user's, the ones FXServer derives from the licence key,
/// and the credentials Observer gives FXServer on each launch.
pub(crate) const SECRET_CONVARS: [&str; 10] = [
    "sv_licenseKey",
    "sv_tebexSecret",
    "steam_webApiKey",
    "rcon_password",
    "mysql_connection_string",
    "mumble_adminPass",
    "sv_licenseKeyToken",
    "adhesive_cdnKey",
    "sv_prometheusBasicAuthPassword",
    "sv_playersToken",
];
const SET_COMMANDS: [&str; 4] = ["set", "sets", "setr", "seta"];
const WEBHOOKS: [&str; 2] = ["discord.com/api/webhooks/", "discordapp.com/api/webhooks/"];
const LICENSE_KEY_PREFIX: &str = "cfxk_";
const REDACTED: &str = "[REDACTED]";

/// The line with every secret replaced, or `None` when it holds none: the value of a secret
/// convar being set (`set rcon_password x`, or `rcon_password x` as a command of its own) or
/// printed (`"rcon_password" is "x"`, or `rcon_password = x` in `cmdlist`), the token of a Discord
/// webhook URL, and anything that looks like a licence key.
pub fn mask(text: &str) -> Option<String> {
    // ASCII lowercasing keeps every byte offset, and every needle is ASCII.
    let lower = text.to_ascii_lowercase();
    let mut secrets = Vec::new();
    convar_values(text, &lower, &mut secrets);
    for webhook in WEBHOOKS {
        for (start, _) in lower.match_indices(webhook) {
            let value = start + webhook.len();
            secrets.push(value..until(text, value, |c| c.is_whitespace() || "\"'<>".contains(c)));
        }
    }
    for (start, _) in lower.match_indices(LICENSE_KEY_PREFIX) {
        if starts_word(text, start) {
            let value = start + LICENSE_KEY_PREFIX.len();
            secrets.push(value..until(text, value, |c| !is_word(c)));
        }
    }

    secrets.retain(|secret| !secret.is_empty());
    if secrets.is_empty() {
        return None;
    }
    secrets.sort_by_key(|secret| secret.start);
    let mut masked = String::with_capacity(text.len());
    let mut copied = 0;
    for secret in secrets {
        if secret.start < copied {
            continue;
        }
        masked.push_str(&text[copied..secret.start]);
        masked.push_str(REDACTED);
        copied = secret.end;
    }
    masked.push_str(&text[copied..]);
    Some(masked)
}

fn convar_values(text: &str, lower: &str, secrets: &mut Vec<std::ops::Range<usize>>) {
    for name in SECRET_CONVARS {
        let name = name.to_ascii_lowercase();
        for (start, _) in lower.match_indices(&name) {
            let end = start + name.len();
            if !starts_word(text, start) || text[end..].starts_with(is_word) {
                continue;
            }
            let quoted = text[..start].ends_with('"') && text[end..].starts_with('"');
            let (before, after) = if quoted {
                (start - 1, end + 1)
            } else {
                (start, end)
            };
            if quoted && text[after..].starts_with(" is \"") {
                let value = after + " is \"".len();
                secrets.push(value..until(text, value, |c| c == '"'));
            } else if text[end..].starts_with(" = ") {
                let value = end + " = ".len();
                secrets.push(value..until(text, value, |c| c == '\x1b'));
            } else if follows_set(&lower[..before]) || starts_command(&lower[..before]) {
                secrets.push(value_after(text, after));
            }
        }
    }
}

/// Whether the text ends in `set` (or a variant) as a word of its own, then blanks.
fn follows_set(before: &str) -> bool {
    let trimmed = before.trim_end_matches([' ', '\t']);
    if trimmed.len() == before.len() {
        return false;
    }
    SET_COMMANDS
        .iter()
        .any(|command| trimmed.ends_with(command) && starts_word(trimmed, trimmed.len() - command.len()))
}

/// Whether the text before a convar name leaves it as the command itself, which sets the convar
/// when a value follows.
fn starts_command(before: &str) -> bool {
    let trimmed = before.trim_end_matches([' ', '\t']);
    trimmed.is_empty() || trimmed.ends_with(';')
}

/// The value that follows a convar name: a quoted string, or a word up to a blank or `;`.
fn value_after(text: &str, end: usize) -> std::ops::Range<usize> {
    let rest = &text[end..];
    let blanks = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    if blanks == 0 {
        return end..end;
    }
    let start = end + blanks;
    if text[start..].starts_with('"') {
        start + 1..until(text, start + 1, |c| c == '"')
    } else {
        start..until(text, start, |c| {
            c.is_whitespace() || c == ';' || c == '"' || c == '\x1b'
        })
    }
}

/// The offset of the first character from `start` on that ends the value, or the end of the text.
fn until(text: &str, start: usize, ends: impl Fn(char) -> bool) -> usize {
    text[start..]
        .find(ends)
        .map_or(text.len(), |offset| start + offset)
}

fn starts_word(text: &str, start: usize) -> bool {
    !text[..start].ends_with(is_word)
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(text: &str) -> String {
        mask(text).unwrap_or_else(|| panic!("nothing masked in {text:?}"))
    }

    #[test]
    fn each_set_command_of_each_secret_convar_is_masked() {
        for command in SET_COMMANDS {
            for name in SECRET_CONVARS {
                assert_eq!(
                    masked(&format!("{command} {name} \"hunter2\"")),
                    format!("{command} {name} \"[REDACTED]\""),
                );
            }
        }
    }

    #[test]
    fn the_licence_key_is_masked_when_set() {
        assert_eq!(
            masked("set sv_licenseKey cfxk_abcdefghijklmnopqrstu_ABC123"),
            "set sv_licenseKey [REDACTED]"
        );
        assert_eq!(masked("+set sv_licenseKey abc"), "+set sv_licenseKey [REDACTED]");
    }

    #[test]
    fn the_tebex_secret_is_masked_when_set() {
        assert_eq!(
            masked("sets sv_tebexSecret 0123abcd"),
            "sets sv_tebexSecret [REDACTED]"
        );
    }

    #[test]
    fn the_steam_api_key_is_masked_when_set() {
        assert_eq!(
            masked("set steam_webApiKey \"ABCDEF0123\""),
            "set steam_webApiKey \"[REDACTED]\""
        );
    }

    #[test]
    fn the_rcon_password_is_masked_when_set_case_and_quotes_aside() {
        assert_eq!(
            masked("SET RCON_PASSWORD letmein; quit"),
            "SET RCON_PASSWORD [REDACTED]; quit"
        );
        assert_eq!(
            masked("set \"rcon_password\" \"letmein\""),
            "set \"rcon_password\" \"[REDACTED]\""
        );
    }

    #[test]
    fn the_mysql_connection_string_is_masked_whole() {
        assert_eq!(
            masked("set mysql_connection_string \"mysql://root:pw@localhost/es?charset=utf8mb4\""),
            "set mysql_connection_string \"[REDACTED]\""
        );
    }

    #[test]
    fn a_secret_convar_printed_by_fxserver_is_masked() {
        assert_eq!(
            masked("[                 cmd]  \"rcon_password\" is \"letmein\""),
            "[                 cmd]  \"rcon_password\" is \"[REDACTED]\""
        );
    }

    #[test]
    fn a_secret_listed_by_cmdlist_is_masked() {
        assert_eq!(
            masked("[           CmdSystem] mysql_connection_string = mysql://root:pw@localhost/es"),
            "[           CmdSystem] mysql_connection_string = [REDACTED]"
        );
        for name in SECRET_CONVARS {
            assert_eq!(
                masked(&format!("[           CmdSystem] {name} = hunter2 two\x1b[0m")),
                format!("[           CmdSystem] {name} = [REDACTED]\x1b[0m"),
            );
        }
    }

    #[test]
    fn a_secret_set_by_typing_its_name_and_a_value_is_masked() {
        assert_eq!(masked("sv_tebexSecret hunter2"), "sv_tebexSecret [REDACTED]");
        assert_eq!(
            masked("say hi; RCON_PASSWORD \"letmein\""),
            "say hi; RCON_PASSWORD \"[REDACTED]\""
        );
        assert_eq!(
            masked("\"steam_webApiKey\" ABCDEF0123"),
            "\"steam_webApiKey\" [REDACTED]"
        );
    }

    #[test]
    fn a_discord_webhook_loses_its_id_and_token() {
        assert_eq!(
            masked("posting to https://discord.com/api/webhooks/1234567890/AbC-dEf_123 now"),
            "posting to https://discord.com/api/webhooks/[REDACTED] now"
        );
        assert_eq!(
            masked("\"https://DiscordApp.com/api/webhooks/1/x\""),
            "\"https://DiscordApp.com/api/webhooks/[REDACTED]\""
        );
    }

    #[test]
    fn a_licence_key_anywhere_is_masked() {
        assert_eq!(
            masked("The specified key (cfxk_abcdefghijklmnopqrstu_ABC123) does not exist."),
            "The specified key (cfxk_[REDACTED]) does not exist."
        );
    }

    #[test]
    fn lines_without_secrets_are_left_alone() {
        for text in [
            "[           resources] Started resource rcon_password_tools",
            "set sv_hostname \"rcon_password\"",
            "my_rcon_password is set",
            "say my rcon_password is long",
            "set rcon_passwordHint abc",
            "[           CmdSystem] sv_hostname = rcon_password",
            "notcfxk_abc",
            "set rcon_password",
            "rcon_password",
        ] {
            assert_eq!(mask(text), None, "{text:?}");
        }
    }
}
