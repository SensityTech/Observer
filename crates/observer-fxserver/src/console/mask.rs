//! Secrets hidden from every console line before it is stored, logged or sent anywhere.

use std::ops::Range;

use crate::command;

/// Convars whose value is a secret: the user's, the ones FXServer derives from the licence key,
/// and the credentials Observer gives FXServer on each launch.
const SECRET_CONVARS: [&str; 10] = [
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
/// A `NAME=value` on a command line whose name ends in one of these holds a secret.
const SECRET_NAMES: [&str; 5] = ["password", "pwd", "token", "secret", "key"];

/// The line with every secret replaced, or `None` when it holds none: the value of a secret
/// convar being set (`set rcon_password x`, or `rcon_password x` as a command of its own) or
/// printed (`"rcon_password" is "x"`, or `rcon_password = x` in `cmdlist`), the token of a Discord
/// webhook URL, and anything that looks like a licence key.
pub fn mask(text: &str) -> Option<String> {
    let secrets = secrets(text);
    (!secrets.is_empty()).then(|| redact(text, secrets))
}

/// What [`mask`] hides in `text`, as byte ranges in no particular order.
pub(crate) fn secrets(text: &str) -> Vec<Range<usize>> {
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
    secrets
}

/// `text` with each range replaced by `[REDACTED]`; overlapping ranges count once.
pub(crate) fn redact(text: &str, mut secrets: Vec<Range<usize>>) -> String {
    secrets.sort_by_key(|secret| secret.start);
    let mut masked = String::with_capacity(text.len());
    let mut copied = 0;
    for secret in secrets {
        if secret.end <= copied {
            continue;
        }
        let start = secret.start.max(copied);
        masked.push_str(&text[copied..start]);
        if start == secret.start {
            masked.push_str(REDACTED);
        }
        copied = secret.end;
    }
    masked.push_str(&text[copied..]);
    masked
}

/// Whether `name` is one of the convars whose value is a secret.
pub(crate) fn secret_convar(name: &str) -> bool {
    SECRET_CONVARS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(name))
}

/// For each command of `text` that sets a convar `secret` holds for, as `set` or its kin, `+set`
/// or the convar's name alone, the range of the value it gives: everything after the name, read
/// the way FXServer reads the command.
pub(crate) fn set_values(text: &str, secret: impl Fn(&str) -> bool) -> Vec<Range<usize>> {
    let mut values = Vec::new();
    for span in command::spans(text) {
        let words = command::word_spans(&text[span.clone()]);
        let set = |word: &str| {
            SET_COMMANDS
                .iter()
                .chain(&["+set"])
                .any(|command| command.eq_ignore_ascii_case(word))
        };
        let named = match words.as_slice() {
            [(_, command), (name_span, name), _, ..] if set(command) && secret(name) => name_span.end,
            [(name_span, name), _, ..] if !set(name) && secret_convar(name) => name_span.end,
            _ => continue,
        };
        let end = words.last().map_or(named, |(last, _)| last.end);
        let value = &text[span.start + named..span.start + end];
        let start = span.start + named + value.len() - value.trim_start_matches(|c: char| c <= ' ').len();
        values.push(start..span.start + end);
    }
    values
}

/// A command line as the console and the audit log may show it. Each command that sets a secret
/// convar as FXServer reads it loses all that follows the name, however the line spells it:
/// `set rcon_password"x"`, `set rcon_password/**/x` or `set "mysql_connection\_string" x` give the
/// text [`mask`] reads no secret in. The line then goes through [`mask`] too. `None` when nothing
/// was masked.
pub fn mask_command(line: &str) -> Option<String> {
    let mut shown = String::with_capacity(line.len());
    let mut copied = 0;
    for span in command::spans(line) {
        let words = command::words(&line[span.clone()]);
        let Some(named) = sets_secret(&words) else {
            continue;
        };
        let typed = &line[span.clone()];
        let start = span.start + typed.len()
            - typed
                .trim_start_matches(|c: char| c <= ' ' || !c.is_ascii())
                .len();
        shown.push_str(&line[copied..start]);
        shown.push_str(&words[..named].join(" "));
        shown.push(' ');
        shown.push_str(REDACTED);
        copied = span.end;
    }
    if copied == 0 {
        return mask(line);
    }
    shown.push_str(&line[copied..]);
    Some(mask(&shown).unwrap_or(shown))
}

/// A process's command line as the diagnostics and the audit log may show it: what [`mask`] hides,
/// and the credentials of a URL (`mysql://user:pass@host`, or a token alone as its user), the
/// value of any `NAME=value` whose name ends in `password`, `pwd`, `token`, `secret` or `key`
/// (connection strings, `MYSQL_PWD=`, `API_KEY=`, cmd's `set "NAME=value"`, PowerShell's
/// `$env:NAME = 'value'`), `--password value` and PowerShell's `-Password value`, the
/// `-p<password>` of the MySQL and MariaDB tools, the password of `-u`/`--user user:password`, and
/// the credentials of an `Authorization` header. Control characters are dropped. PowerShell's
/// `-enc <base64>` stays: it is what the program was told to run, not a secret.
pub fn mask_command_line(line: &str) -> String {
    let line: String = line.chars().filter(|c| !c.is_control()).collect();
    let shown = redact(&line, command_line_secrets(&line));
    mask(&shown).unwrap_or(shown)
}

/// What [`mask_command_line`] hides beyond [`mask`], as byte ranges of `line` as it is given.
pub(crate) fn command_line_secrets(line: &str) -> Vec<Range<usize>> {
    let lower = line.to_ascii_lowercase();
    let mut secrets = Vec::new();
    for (start, _) in lower.match_indices("://") {
        let authority = start + "://".len();
        let end = until(line, authority, |c| {
            c == '/' || c.is_whitespace() || "\"'?#".contains(c)
        });
        if let Some(at) = line[authority..end].rfind('@') {
            let userinfo = authority..authority + at;
            match line[userinfo.clone()].find(':') {
                Some(colon) => secrets.push(authority + colon + 1..userinfo.end),
                None => secrets.push(userinfo),
            }
        }
    }
    for (equals, _) in line.match_indices('=') {
        if let Some(value) = assigned_secret(line, &lower, equals) {
            secrets.push(value);
        }
    }
    for option in ["--password", "-password"] {
        for (start, _) in options(&lower, option) {
            secrets.extend(next_word(line, start + option.len()));
        }
    }
    if lower
        .split(|c: char| !is_word(c))
        .any(|word| word.starts_with("mysql") || word.starts_with("mariadb"))
    {
        for (start, quote) in options(line, "-p") {
            secrets.push(option_value(line, start + "-p".len(), quote));
        }
    }
    for option in ["-u", "--user"] {
        for (start, quote) in options(line, option) {
            let end = start + option.len();
            let user = next_word(line, end).unwrap_or_else(|| {
                option_value(line, end + usize::from(line[end..].starts_with('=')), quote)
            });
            if let Some(colon) = line[user.clone()].find(':') {
                secrets.push(user.start + colon + 1..user.end);
            }
        }
    }
    for (start, _) in lower.match_indices("authorization:") {
        let scheme = start + "authorization:".len();
        let scheme = scheme + line[scheme..].len() - line[scheme..].trim_start().len();
        let scheme_end = until(line, scheme, |c| c.is_whitespace() || "\"'".contains(c));
        let rest = &line[scheme_end..];
        let token = scheme_end + rest.len() - rest.trim_start_matches([' ', '\t']).len();
        let credentials = if token > scheme_end { token } else { scheme };
        secrets.push(credentials..until(line, credentials, |c| c.is_whitespace() || "\"'".contains(c)));
    }

    secrets.retain(|secret| !secret.is_empty());
    secrets
}

/// The value the `=` at `equals` gives a variable whose name ends in a secret word: `NAME=value`;
/// cmd's `set NAME=value`, which runs to the next command, and `set "NAME=value"`, to the quote;
/// PowerShell's `$name = 'value'` and `$env:NAME = 'value'`, with blanks around `=`.
fn assigned_secret(line: &str, lower: &str, equals: usize) -> Option<Range<usize>> {
    if line[equals + 1..].starts_with('=') {
        return None;
    }
    let before = line[..equals].trim_end_matches([' ', '\t']);
    let name_start = before.trim_end_matches(is_word).len();
    let name = &lower[name_start..before.len()];
    if name.is_empty() || !SECRET_NAMES.iter().any(|secret| name.ends_with(secret)) {
        return None;
    }
    let prefix = &lower[..name_start];
    let variable = prefix.ends_with('$') || prefix.ends_with("$env:");
    if before.len() < equals && !variable {
        return None;
    }
    let value = if variable {
        let rest = &line[equals + 1..];
        equals + 1 + rest.len() - rest.trim_start_matches([' ', '\t']).len()
    } else {
        equals + 1
    };
    let opened = prefix.chars().next_back().filter(|c| matches!(c, '"' | '\''));
    let unquoted = opened.map_or(prefix, |quote| &prefix[..prefix.len() - quote.len_utf8()]);
    Some(match opened {
        _ if !follows_set(unquoted) => option_value(line, value, None),
        Some(quote) => value..until(line, value, |c| c == quote),
        None => {
            let end = until(line, value, |c| "&|\"".contains(c));
            value..value + line[value..end].trim_end().len()
        }
    })
}

/// The word that follows the blanks after `end`, inside its quotes when it has some. `None` when
/// no blank follows.
fn next_word(line: &str, end: usize) -> Option<Range<usize>> {
    let rest = &line[end..];
    let blanks = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    (blanks > 0).then(|| option_value(line, end + blanks, None))
}

/// Where `option` starts a word of `line`, after a blank, a quote or nothing, with that quote.
fn options<'a>(line: &'a str, option: &'a str) -> impl Iterator<Item = (usize, Option<char>)> + 'a {
    line.match_indices(option).filter_map(|(start, _)| {
        let before = line[..start].chars().next_back();
        match before {
            None => Some((start, None)),
            Some(c) if c.is_whitespace() => Some((start, None)),
            Some(quote @ ('"' | '\'')) => Some((start, Some(quote))),
            Some(_) => None,
        }
    })
}

/// The value written right at `start`: up to the quote that opened the word it is in, inside the
/// quotes it starts with, or up to a blank, a quote or a separator.
fn option_value(line: &str, start: usize, quote: Option<char>) -> Range<usize> {
    if let Some(quote) = quote {
        return start..until(line, start, |c| c == quote);
    }
    match line[start..].chars().next() {
        Some(quote @ ('"' | '\'')) => start + 1..until(line, start + 1, |c| c == quote),
        _ => start..until(line, start, |c| c.is_whitespace() || ";&\"'".contains(c)),
    }
}

/// How many words name the secret convar a command sets, `set rcon_password` or `rcon_password`
/// alone, when a value follows them.
fn sets_secret(words: &[String]) -> Option<usize> {
    let secret = |word: &str| SECRET_CONVARS.iter().any(|name| name.eq_ignore_ascii_case(word));
    match words {
        [set, name, _, ..]
            if SET_COMMANDS
                .iter()
                .any(|command| command.eq_ignore_ascii_case(set)) =>
        {
            secret(name).then_some(2)
        }
        [name, _, ..] => secret(name).then_some(1),
        _ => None,
    }
}

fn convar_values(text: &str, lower: &str, secrets: &mut Vec<Range<usize>>) {
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
fn value_after(text: &str, end: usize) -> Range<usize> {
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

/// A colour sequence ends a word as a blank does: FXServer prints its text right after the one
/// closing the channel prefix.
fn starts_word(text: &str, start: usize) -> bool {
    let before = &text[..start];
    !before.ends_with(is_word) || ends_with_colour(before)
}

fn ends_with_colour(text: &str) -> bool {
    let Some(rest) = text.strip_suffix('m') else {
        return false;
    };
    rest.rfind("\x1b[").is_some_and(|start| {
        rest[start + 2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b';')
    })
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    // Every key, token and webhook below is a test fixture, not a real secret.

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
        // As FXServer prints them: the name comes right after the colour sequences of the prefix.
        let prefix = "\x1b[38;5;110m[           CmdSystem] \x1b[0m\x1b[0m";
        for name in SECRET_CONVARS {
            assert_eq!(
                masked(&format!("{prefix}{name} = hunter2 two\x1b[0m")),
                format!("{prefix}{name} = [REDACTED]\x1b[0m"),
            );
        }
        assert_eq!(
            masked(&format!("{prefix}cfxk_abcdefghijklmnopqrstu_ABC123")),
            format!("{prefix}cfxk_[REDACTED]")
        );
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
    fn a_typed_command_setting_a_secret_is_masked_however_fxserver_reads_it() {
        for (line, shown) in [
            ("set sv_tebexSecret\"hunter2\"", "set sv_tebexSecret [REDACTED]"),
            ("set rcon_password/**/hunter2", "set rcon_password [REDACTED]"),
            ("set /**/rcon_password hunter2", "set rcon_password [REDACTED]"),
            ("rcon_password\"hunter2\"", "rcon_password [REDACTED]"),
            (
                "set \"mysql_connection\\_string\" hunter2",
                "set mysql_connection_string [REDACTED]",
            ),
            (
                "say hi;  SETR Steam_WebApiKey\"k\"; status",
                "say hi;  SETR Steam_WebApiKey [REDACTED]; status",
            ),
            (
                "set sv_licenseKey cfxk_abcdefghijklmnopqrstu_ABC123",
                "set sv_licenseKey [REDACTED]",
            ),
        ] {
            assert_eq!(mask_command(line).as_deref(), Some(shown), "{line:?}");
        }
    }

    #[test]
    fn a_typed_command_keeps_the_text_masks_too() {
        assert_eq!(
            mask_command("say see https://discord.com/api/webhooks/1/x").as_deref(),
            Some("say see https://discord.com/api/webhooks/[REDACTED]")
        );
        for line in [
            "set sv_hostname \"Observer lab\"",
            "set rcon_password",
            "say rcon_password hunter2",
            "ensure chat; status",
        ] {
            assert_eq!(mask_command(line), None, "{line:?}");
        }
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

    #[test]
    fn a_url_keeps_its_user_and_loses_its_password() {
        assert_eq!(
            mask_command_line("node sync.js mysql://fivem:hunter2@127.0.0.1:3306/qbcore"),
            "node sync.js mysql://fivem:[REDACTED]@127.0.0.1:3306/qbcore"
        );
        assert_eq!(
            mask_command_line("curl https://example.com/a@b"),
            "curl https://example.com/a@b"
        );
    }

    #[test]
    fn connection_string_passwords_are_masked() {
        assert_eq!(
            mask_command_line(r#"tool.exe "Server=db;User Id=sa;Password=hunter2;" pwd=x2"#),
            r#"tool.exe "Server=db;User Id=sa;Password=[REDACTED];" pwd=[REDACTED]"#
        );
    }

    #[test]
    fn a_password_option_is_masked_with_or_without_an_equals_sign() {
        assert_eq!(
            mask_command_line("backup --password hunter2 --password=x3 --host db"),
            "backup --password [REDACTED] --password=[REDACTED] --host db"
        );
    }

    #[test]
    fn mysql_s_short_password_option_is_masked_and_its_port_is_not() {
        assert_eq!(
            mask_command_line("/usr/bin/mysqldump -ufivem -phunter2 -P3306 qbcore"),
            "/usr/bin/mysqldump -ufivem -p[REDACTED] -P3306 qbcore"
        );
        assert_eq!(
            mask_command_line("ffmpeg -pix_fmt yuv420p"),
            "ffmpeg -pix_fmt yuv420p"
        );
    }

    #[test]
    fn a_token_given_as_a_url_s_user_is_masked() {
        assert_eq!(
            mask_command_line("git clone https://ghp_hunter2token@github.com/org/private.git"),
            "git clone https://[REDACTED]@github.com/org/private.git"
        );
    }

    #[test]
    fn a_variable_naming_a_password_token_secret_or_key_loses_its_value() {
        for (line, shown) in [
            (
                "sh -c MYSQL_PWD=hunter2 mysqldump -u fivem qbcore",
                "sh -c MYSQL_PWD=[REDACTED] mysqldump -u fivem qbcore",
            ),
            (
                r#"sh -c "PGPASSWORD=hunter2 pg_dump -h db fivem""#,
                r#"sh -c "PGPASSWORD=[REDACTED] pg_dump -h db fivem""#,
            ),
            (
                "cmd /c set API_KEY=hunter2&& node sync.js",
                "cmd /c set API_KEY=[REDACTED]&& node sync.js",
            ),
            (
                "node bot.js --discord-token='hunter 2' --client_secret=x9",
                "node bot.js --discord-token='[REDACTED]' --client_secret=[REDACTED]",
            ),
            (
                "curl https://api.example.com/?apikey=hunter2",
                "curl https://api.example.com/?apikey=[REDACTED]",
            ),
        ] {
            assert_eq!(mask_command_line(line), shown, "{line:?}");
        }
    }

    #[test]
    fn a_secret_assigned_by_cmd_or_powershell_loses_its_whole_value() {
        for (line, shown) in [
            (
                r#"cmd /c set "MYSQL_PWD=hunter 2" && mysqldump -u root qbcore"#,
                r#"cmd /c set "MYSQL_PWD=[REDACTED]" && mysqldump -u root qbcore"#,
            ),
            (
                "cmd /c set MYSQL_PWD=hunter 2&& mysqldump -u root qbcore",
                "cmd /c set MYSQL_PWD=[REDACTED]&& mysqldump -u root qbcore",
            ),
            (
                r#"powershell -Command "$env:MYSQL_PWD = 'hunter2'; mysqldump -u root qbcore""#,
                r#"powershell -Command "$env:MYSQL_PWD = '[REDACTED]'; mysqldump -u root qbcore""#,
            ),
            (
                r#"pwsh -c "$password = 'hunter2'; Invoke-Sqlcmd -Password $password""#,
                r#"pwsh -c "$password = '[REDACTED]'; Invoke-Sqlcmd -Password [REDACTED]""#,
            ),
            (
                r#"pwsh -c "Invoke-Sqlcmd -Username sa -Password 'hunter 2'""#,
                r#"pwsh -c "Invoke-Sqlcmd -Username sa -Password '[REDACTED]'""#,
            ),
            (
                r#"pwsh -c "if ($token -eq $null) { $apiKey == 'x' }""#,
                r#"pwsh -c "if ($token -eq $null) { $apiKey == 'x' }""#,
            ),
        ] {
            assert_eq!(mask_command_line(line), shown, "{line:?}");
        }
    }

    #[test]
    fn the_short_password_option_of_mariadb_tools_is_masked_quoted_or_not() {
        for (line, shown) in [
            (
                "mariadb-dump -u fivem -phunter2 qbcore",
                "mariadb-dump -u fivem -p[REDACTED] qbcore",
            ),
            (
                r"C:\mariadb\bin\mariadb.exe -u root -phunter2 fivem",
                r"C:\mariadb\bin\mariadb.exe -u root -p[REDACTED] fivem",
            ),
            (
                "mysqladmin -u root -phunter2 status",
                "mysqladmin -u root -p[REDACTED] status",
            ),
            (
                r#"mysqldump.exe -u root "-phunter 2" qbcore"#,
                r#"mysqldump.exe -u root "-p[REDACTED]" qbcore"#,
            ),
            (
                r#"mysql.exe -u root -p"hunter 2" qbcore"#,
                r#"mysql.exe -u root -p"[REDACTED]" qbcore"#,
            ),
        ] {
            assert_eq!(mask_command_line(line), shown, "{line:?}");
        }
    }

    #[test]
    fn the_password_of_a_user_option_is_masked_and_its_user_kept() {
        assert_eq!(
            mask_command_line("curl -u api:hunter2 https://api.example.com/backup"),
            "curl -u api:[REDACTED] https://api.example.com/backup"
        );
        assert_eq!(
            mask_command_line("curl --user=api:hunter2 -uops:x9 https://api.example.com"),
            "curl --user=api:[REDACTED] -uops:[REDACTED] https://api.example.com"
        );
        assert_eq!(
            mask_command_line("curl -u 'api:hunter 2' --password 'x 9' https://api.example.com"),
            "curl -u 'api:[REDACTED]' --password '[REDACTED]' https://api.example.com"
        );
        assert_eq!(
            mask_command_line("mysql -u fivem qbcore"),
            "mysql -u fivem qbcore"
        );
    }

    #[test]
    fn an_authorization_header_keeps_its_scheme() {
        assert_eq!(
            mask_command_line(r#"curl -H "Authorization: Bearer eyJhbGciOi.x.y" https://api.example.com"#),
            r#"curl -H "Authorization: Bearer [REDACTED]" https://api.example.com"#
        );
    }

    #[test]
    fn console_secrets_and_control_characters_go_and_encoded_commands_stay() {
        assert_eq!(
            mask_command_line("FXServer.exe +set sv_licenseKey cfxk_abc123 +set\u{7}x"),
            "FXServer.exe +set sv_licenseKey [REDACTED] +setx"
        );
        assert_eq!(
            mask_command_line("powershell -NoProfile -enc SQBFAFgAIAAoAE4AZQB3AC0ATwBiAGoA"),
            "powershell -NoProfile -enc SQBFAFgAIAAoAE4AZQB3AC0ATwBiAGoA"
        );
    }
}
