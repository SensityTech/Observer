//! Secrets hidden from file contents before they are stored: what the console hides, the values
//! cfg files give secret convars, credentials in URLs and command lines, string values given to
//! names that say they hold a secret, and the shapes of known tokens. Each secret becomes
//! `[REDACTED]` in place, so tabs, line endings and line numbers stay as they were, with a tag that
//! tells two versions of one secret apart without saying anything about it.

use std::ops::Range;

use hmac::{Hmac, KeyInit, Mac};
use observer_domain::changes::extension;
use sha2::Sha256;

use crate::console::mask as console;

/// The rules' version, stored with each text. When it rises, the upkeep pass masks the texts of
/// older versions again.
pub const VERSION: u32 = 1;
pub const REDACTED: &str = "[REDACTED]";

/// Names that hold a secret when they end in one of these, case aside and `_` and `-` left out.
const SECRET_WORDS: [&str; 18] = [
    "password",
    "passwd",
    "pass",
    "pwd",
    "token",
    "secret",
    "key",
    "webhook",
    "license",
    "licence",
    "auth",
    "credential",
    "credentials",
    "privatekey",
    "connectionstring",
    "dsn",
    "salt",
    "apikey",
];
/// Files whose `name: value` and `name = value` run to the end of the line without quotes.
const LINE_VALUES: [&str; 9] = [
    "yml",
    "yaml",
    "ini",
    "toml",
    "env",
    "properties",
    "conf",
    "txt",
    "md",
];
/// Token prefixes, kept visible so that the kind of token shows, with the fewest characters that
/// must follow.
const PREFIXES: [(&str, usize); 10] = [
    ("ghp_", 20),
    ("gho_", 20),
    ("ghs_", 20),
    ("github_pat_", 20),
    ("xoxb-", 10),
    ("xoxp-", 10),
    ("sk_live_", 10),
    ("rk_live_", 10),
    ("AKIA", 12),
    ("AIza", 30),
];
/// Webhook URLs whose path is the secret, besides Discord's, which the console already hides.
const WEBHOOK_HOSTS: [&str; 2] = ["hooks.slack.com/services/", ".webhook.office.com/"];

/// A text with its secrets hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Masked {
    pub text: String,
    pub redactions: Vec<Redaction>,
}

/// Where a `[REDACTED]` lies in a masked text, and the tag of the value it hides: a keyed hash cut
/// to 16 bits, which tells versions of one secret apart and never leaves the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Redaction {
    pub at: u32,
    pub tag: u16,
}

impl Redaction {
    /// Six bytes each, big-endian: where, then the tag.
    pub fn encode(redactions: &[Self]) -> Vec<u8> {
        redactions
            .iter()
            .flat_map(|redaction| {
                let mut bytes = [0; 6];
                bytes[..4].copy_from_slice(&redaction.at.to_be_bytes());
                bytes[4..].copy_from_slice(&redaction.tag.to_be_bytes());
                bytes
            })
            .collect()
    }

    pub fn decode(bytes: &[u8]) -> Vec<Self> {
        bytes
            .as_chunks::<6>()
            .0
            .iter()
            .map(|chunk| Self {
                at: u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                tag: u16::from_be_bytes([chunk[4], chunk[5]]),
            })
            .collect()
    }
}

/// Masks `text`, the file at `path`. `chain` for a file of the cfg exec chain, whatever its name.
pub fn mask(text: &str, path: &str, chain: bool, tag_key: &[u8; 32]) -> Masked {
    let secrets = secrets(text, path, chain);
    apply(text, secrets, &[], tag_key)
}

/// Masks again a text masked with older rules: what the current rules find is hidden too, and the
/// redactions already there keep their tags.
pub fn remask(masked: &Masked, path: &str, chain: bool, tag_key: &[u8; 32]) -> Masked {
    let secrets = secrets(&masked.text, path, chain);
    apply(&masked.text, secrets, &masked.redactions, tag_key)
}

/// Every secret of `text`, as byte ranges in no particular order.
pub fn secrets(text: &str, path: &str, chain: bool) -> Vec<Range<usize>> {
    let extension = extension(path).unwrap_or_default();
    let line_values = LINE_VALUES.contains(&extension.as_str());
    let mut found = Vec::new();
    if chain || extension == "cfg" {
        found.extend(console::set_values(text, |name| {
            secret_name(name) || console::secret_convar(name)
        }));
    }
    assignments(text, line_values, &mut found);
    private_keys(text, &mut found);
    let mut start = 0;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let shift = |range: Range<usize>| range.start + start..range.end + start;
        found.extend(console::secrets(line).into_iter().map(shift));
        found.extend(console::command_line_secrets(line).into_iter().map(shift));
        let mut shapes = Vec::new();
        token_shapes(line, &mut shapes);
        found.extend(shapes.into_iter().map(shift));
        start += line.len() + 1;
        if text.as_bytes().get(start - 1) == Some(&b'\r') {
            start += 1;
        }
    }
    found.retain(|range| !range.is_empty() && range.end <= text.len());
    found
}

/// Whether a name says it holds a secret.
fn secret_name(name: &str) -> bool {
    let name: String = name
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    SECRET_WORDS.iter().any(|word| name.ends_with(word))
}

fn apply(text: &str, secrets: Vec<Range<usize>>, existing: &[Redaction], tag_key: &[u8; 32]) -> Masked {
    let mut ranges: Vec<Range<usize>> = secrets;
    // A marker already in the text is a secret as it stands, so that what overlaps it merges with it.
    let markers: Vec<Range<usize>> = existing
        .iter()
        .filter_map(|redaction| {
            let at = usize::try_from(redaction.at).ok()?;
            (text.get(at..at + REDACTED.len()) == Some(REDACTED)).then_some(at..at + REDACTED.len())
        })
        .collect();
    ranges.extend(markers.iter().cloned());
    ranges.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start < last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    let mut masked = String::with_capacity(text.len());
    let mut redactions = Vec::with_capacity(merged.len());
    let mut copied = 0;
    for range in merged {
        masked.push_str(&text[copied..range.start]);
        let kept = markers
            .iter()
            .position(|marker| *marker == range)
            .and_then(|index| existing.get(index))
            .map(|redaction| redaction.tag);
        redactions.push(Redaction {
            at: u32::try_from(masked.len()).unwrap_or(u32::MAX),
            tag: kept.unwrap_or_else(|| tag(&text[range.clone()], tag_key)),
        });
        masked.push_str(REDACTED);
        copied = range.end;
    }
    masked.push_str(&text[copied..]);
    Masked {
        text: masked,
        redactions,
    }
}

fn tag(value: &str, key: &[u8; 32]) -> u16 {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(value.as_bytes());
    let bytes = mac.finalize().into_bytes();
    u16::from_be_bytes([bytes[0], bytes[1]])
}

/// String values given to a secret name: `name = "…"`, `name: '…'`, `["name"] = [[…]]`,
/// `"name": "…"`, `` name: `…` ``, and in files of `LINE_VALUES` an unquoted value to the end of
/// the line. Values that are not strings (`bypass = true`, a number, a call) stay.
fn assignments(text: &str, line_values: bool, found: &mut Vec<Range<usize>>) {
    let bytes = text.as_bytes();
    for (at, operator) in text.match_indices(['=', ':']) {
        let next = bytes.get(at + 1).copied();
        let previous = at.checked_sub(1).map(|index| bytes[index]);
        let comparison = operator == "="
            && (matches!(next, Some(b'=' | b'>'))
                || matches!(previous, Some(b'=' | b'!' | b'<' | b'>' | b'~')));
        if comparison || (operator == ":" && (next == Some(b':') || previous == Some(b':'))) {
            continue;
        }
        let Some(name) = name_before(text, at) else {
            continue;
        };
        if !secret_name(name) {
            continue;
        }
        if let Some(value) = value_after(text, at + 1, line_values) {
            found.push(value);
        }
    }
}

/// The name an `=` or `:` at `at` assigns to: an identifier, its last part after a `.`, or a
/// quoted key, bracketed or not. On the same line only.
fn name_before(text: &str, at: usize) -> Option<&str> {
    let before = text[..at].trim_end_matches([' ', '\t']);
    let before = before.strip_suffix(']').unwrap_or(before);
    if let Some(quoted) = before.strip_suffix(['"', '\'']) {
        let quote = &before[before.len() - 1..];
        let start = quoted.rfind(quote)?;
        let name = &quoted[start + 1..];
        return (!name.contains('\n')).then_some(name);
    }
    let start = before
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '$'))
        .map_or(0, |index| index + 1);
    let name = &before[start..];
    (!name.is_empty()).then_some(name)
}

/// The string value written from `from`, inside its quotes or brackets.
fn value_after(text: &str, from: usize, line_values: bool) -> Option<Range<usize>> {
    let rest = &text[from..];
    let start = from + rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let rest = &text[start..];
    let line_end = rest.find(['\n', '\r']).map_or(text.len(), |end| start + end);
    match rest.chars().next()? {
        quote @ ('"' | '\'') => {
            let line = &text[start + 1..line_end];
            let mut escaped = false;
            let end = line
                .char_indices()
                .find(|(_, c)| {
                    let closes = *c == quote && !escaped;
                    escaped = *c == '\\' && !escaped;
                    closes
                })
                .map_or(line_end, |(index, _)| start + 1 + index);
            Some(start + 1..end)
        }
        '`' => {
            let end = text[start + 1..]
                .find('`')
                .map_or(text.len(), |end| start + 1 + end);
            Some(start + 1..end)
        }
        '[' => {
            let level = rest[1..].len() - rest[1..].trim_start_matches('=').len();
            if !rest[1 + level..].starts_with('[') {
                return None;
            }
            let open = start + level + 2;
            let close = format!("]{}]", "=".repeat(level));
            let end = text[open..].find(&close).map_or(text.len(), |end| open + end);
            Some(open..end)
        }
        _ if line_values => {
            let value = text[start..line_end].trim_end();
            let literal = value.is_empty()
                || matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
                )
                || value.parse::<f64>().is_ok()
                || value.starts_with(['{', '[', '|', '>', '&', '*', '#', ';']);
            (!literal).then(|| start..start + value.len())
        }
        _ => None,
    }
}

/// The body of each PEM private key block, from the line after `BEGIN` to the `END` line.
fn private_keys(text: &str, found: &mut Vec<Range<usize>>) {
    for (begin, _) in text.match_indices("-----BEGIN ") {
        let Some(header_end) = text[begin..].find('\n').map(|end| begin + end + 1) else {
            continue;
        };
        if !text[begin..header_end].contains("PRIVATE KEY-----") {
            continue;
        }
        let end = text[header_end..]
            .find("-----END ")
            .map_or(text.len(), |end| header_end + end);
        found.push(header_end..end);
    }
}

/// Tokens recognised by their shape: prefixed API tokens, JWTs, Discord bot tokens, and webhook
/// URLs other than Discord's.
fn token_shapes(line: &str, found: &mut Vec<Range<usize>>) {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    for (prefix, least) in PREFIXES {
        for (start, _) in line.match_indices(prefix) {
            let after_word = line[..start].ends_with(|c: char| c.is_ascii_alphanumeric());
            let value = start + prefix.len();
            let end = value
                + line[value..]
                    .find(|c: char| !word(c))
                    .unwrap_or(line.len() - value);
            if !after_word && end - value >= least {
                found.push(value..end);
            }
        }
    }
    let lower = line.to_ascii_lowercase();
    for host in WEBHOOK_HOSTS {
        for (start, _) in lower.match_indices(host) {
            let value = start + host.len();
            let end = value
                + line[value..]
                    .find(|c: char| c.is_whitespace() || "\"'<>`".contains(c))
                    .unwrap_or(line.len() - value);
            found.push(value..end);
        }
    }
    let mut start = 0;
    for run in line.split(|c: char| !(word(c) || c == '.')) {
        let parts: Vec<&str> = run.split('.').collect();
        let jwt = parts.len() == 3
            && parts[0].starts_with("eyJ")
            && parts[1].starts_with("eyJ")
            && !parts[2].is_empty();
        let bot = parts.len() == 3
            && (23..=28).contains(&parts[0].len())
            && (6..=7).contains(&parts[1].len())
            && (27..=40).contains(&parts[2].len());
        if jwt || bot {
            found.push(start..start + run.len());
        }
        start += run.len() + 1;
    }
}

#[cfg(test)]
mod tests {
    // Every key, token and webhook below is a test fixture, not a real secret.

    use super::*;

    const KEY: [u8; 32] = [3; 32];

    fn masked(text: &str, path: &str) -> String {
        mask(text, path, false, &KEY).text
    }

    #[test]
    fn console_rules_apply_line_by_line() {
        assert_eq!(
            masked("set rcon_password hunter2\nrcon_password = abc\r\n", "a.txt"),
            "set rcon_password [REDACTED]\nrcon_password = [REDACTED]\r\n"
        );
        assert_eq!(
            masked("url = https://discord.com/api/webhooks/123/abcDEF\n", "a.lua"),
            "url = https://discord.com/api/webhooks/[REDACTED]\n"
        );
        assert_eq!(
            masked("key: cfxk_abc123_def\n", "a.lua"),
            "key: cfxk_[REDACTED]\n"
        );
    }

    #[test]
    fn a_cfg_hides_every_spelling_fxserver_accepts() {
        let cfg = "set rcon_password\"x\"\nset \"mysql_connection\\_string\" \"mysql://u:p@h/db\"\n\
                   setr my_bot_token abc # trailing\n+set api_key 123\nsets sv_projectName Lab\n";

        assert_eq!(
            masked(cfg, "server.cfg"),
            "set rcon_password[REDACTED]\nset \"mysql_connection\\_string\" [REDACTED]\n\
             setr my_bot_token [REDACTED] # trailing\n+set api_key [REDACTED]\nsets sv_projectName Lab\n"
        );
    }

    #[test]
    fn a_file_of_the_exec_chain_is_read_as_a_cfg_whatever_its_name() {
        let text = "set steam_webApiKey ABCDEF\n";

        assert_eq!(
            mask(text, "secrets.txt", true, &KEY).text,
            "set steam_webApiKey [REDACTED]\n"
        );
    }

    #[test]
    fn credentials_in_urls_and_command_lines_are_hidden() {
        assert_eq!(
            masked("Config.Db = 'mysql://root:pa55@localhost/es'\n", "a.lua"),
            "Config.Db = 'mysql://root:[REDACTED]@localhost/es'\n"
        );
        assert_eq!(
            masked("headers['Authorization: Bearer abc.def']\n", "a.js"),
            "headers['Authorization: Bearer [REDACTED]']\n"
        );
    }

    #[test]
    fn string_values_given_to_secret_names_are_hidden_in_every_language() {
        let cases = [
            (
                "Config.BotToken = \"abc\"\n",
                "a.lua",
                "Config.BotToken = \"[REDACTED]\"\n",
            ),
            ("local apiKey = 'abc'\n", "a.lua", "local apiKey = '[REDACTED]'\n"),
            (
                "t[\"api-key\"] = 'abc'\n",
                "a.lua",
                "t[\"api-key\"] = '[REDACTED]'\n",
            ),
            (
                "const webhook = `https://x`\n",
                "a.js",
                "const webhook = `[REDACTED]`\n",
            ),
            ("{ secret: 'abc' }\n", "a.ts", "{ secret: '[REDACTED]' }\n"),
            (
                "{ \"password\": \"abc\" }\n",
                "a.json",
                "{ \"password\": \"[REDACTED]\" }\n",
            ),
            ("db_password: hunter2\n", "a.yml", "db_password: [REDACTED]\n"),
            ("ApiKey = abc def\n", "a.ini", "ApiKey = [REDACTED]\n"),
            (
                "license_key = \"abc\"\n",
                "a.toml",
                "license_key = \"[REDACTED]\"\n",
            ),
        ];
        for (text, path, expected) in cases {
            assert_eq!(masked(text, path), expected, "{path}");
        }
    }

    #[test]
    fn values_that_are_not_strings_stay() {
        for (text, path) in [
            ("Config.Bypass = true\n", "a.lua"),
            ("bypass_auth: false\n", "a.yml"),
            ("if token == 'x' then\n", "a.lua"),
            ("local key = GetConvar('k', '')\n", "a.lua"),
            ("token_ttl = 3600\n", "a.ini"),
            ("const keys = (token) => token\n", "a.js"),
        ] {
            assert_eq!(masked(text, path), text, "{text}");
        }
    }

    #[test]
    fn multi_line_values_are_hidden_whole() {
        let lua = "Config.PrivateKey = [[\nline one\nline two\n]]\nnext = 1\n";
        let js = "const token = `a\nb`;\n";
        let pem = "x\n-----BEGIN RSA PRIVATE KEY-----\nMIIabc\ndef\n-----END RSA PRIVATE KEY-----\ny\n";

        assert_eq!(
            masked(lua, "a.lua"),
            "Config.PrivateKey = [[[REDACTED]]]\nnext = 1\n"
        );
        assert_eq!(masked(js, "a.js"), "const token = `[REDACTED]`;\n");
        assert_eq!(
            masked(pem, "a.txt"),
            "x\n-----BEGIN RSA PRIVATE KEY-----\n[REDACTED]-----END RSA PRIVATE KEY-----\ny\n"
        );
    }

    #[test]
    fn token_shapes_are_hidden_anywhere() {
        let cases = [
            "print('ghp_0123456789abcdefghij0123')",
            "x = xoxb-1234567890-abcdef",
            "stripe sk_live_0123456789abcdef",
            "aws AKIAABCDEFGHIJKLMNOP",
            "maps AIzaSyA0123456789abcdefghijklmnopqrstu",
            "hook https://hooks.slack.com/services/T000/B000/XXXX",
            "hook https://acme.webhook.office.com/webhookb2/abc",
            "jwt eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl",
            "bot MTEyMzQ1Njc4OTAxMjM0NTY3OA.GhIjKl.abcdefghijklmnopqrstuvwxyz0123",
        ];
        for line in cases {
            let shown = masked(line, "a.md");
            assert!(shown.contains(REDACTED), "{line} -> {shown}");
        }
        assert_eq!(
            masked("version 1.2.3 and v.1.2", "a.md"),
            "version 1.2.3 and v.1.2"
        );
    }

    #[test]
    fn masking_keeps_every_byte_outside_the_redactions() {
        let text = "a\tb = 1\r\nlocal token = \"x\"\r\n\tc = 2\nlast";

        let shown = masked(text, "a.lua");

        assert_eq!(shown, "a\tb = 1\r\nlocal token = \"[REDACTED]\"\r\n\tc = 2\nlast");
        assert_eq!(shown.lines().count(), text.lines().count());
    }

    #[test]
    fn tags_tell_two_values_apart_and_say_where_each_redaction_is() {
        let a = mask(
            "token = 'one'\ntoken = 'one'\ntoken = 'two'\n",
            "a.lua",
            false,
            &KEY,
        );

        assert_eq!(a.redactions.len(), 3);
        assert_eq!(a.redactions[0].tag, a.redactions[1].tag);
        assert_ne!(a.redactions[0].tag, a.redactions[2].tag);
        for redaction in &a.redactions {
            let at = redaction.at as usize;
            assert_eq!(&a.text[at..at + REDACTED.len()], REDACTED);
        }
        assert_eq!(Redaction::decode(&Redaction::encode(&a.redactions)), a.redactions);
    }

    #[test]
    fn masking_again_keeps_the_tags_already_there() {
        let first = mask("token = 'one'\n", "a.lua", false, &KEY);
        let planted = Masked {
            text: format!("{}hidden_later = ghp_0123456789abcdefghij0123\n", first.text),
            redactions: first.redactions.clone(),
        };

        let again = remask(&planted, "a.lua", false, &KEY);

        assert_eq!(again.redactions.len(), 2);
        assert_eq!(again.redactions[0], first.redactions[0]);
        assert!(!again.text.contains("0123456789abcdefghij"));
    }
}
