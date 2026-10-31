//! What is taken out of a drop's text before it is stored: secrets, identifiers, addresses,
//! hardware tokens, URL credentials and queries, and user folders. Kick texts are written by
//! resources, and anticheats print identifiers and tokens in them; identities belong in their own
//! tables.

use std::net::IpAddr;

use observer_fxserver::console::mask;

/// The longest text kept, in bytes.
pub(super) const MAX_TEXT_BYTES: usize = 512;

/// Names as they precede a value: `license:x`, `license: x`, `"license":"x"`, `license=x`.
const IDENTIFIERS: [(&str, &str); 8] = [
    ("license2", "[license2]"),
    ("license", "[license]"),
    ("steam", "[steam]"),
    ("discord", "[discord]"),
    ("fivem", "[fivem]"),
    ("xbl", "[xbl]"),
    ("live", "[live]"),
    ("ip", "[ip]"),
];
const SECRET_NAMES: [&str; 5] = ["password", "pwd", "token", "secret", "key"];
/// Authorization schemes whose credential follows them.
const SCHEMES: [&str; 2] = ["bearer ", "basic "];
/// Where a user's folder starts, lowercased: Windows, Windows in JSON, Linux, macOS.
const USER_FOLDERS: [&str; 5] = [":\\users\\", ":\\\\users\\\\", ":/users/", "/home/", "/users/"];
/// A user folder's name is never longer than this.
const MAX_USER_NAME: usize = 64;

pub(super) fn redact(text: &str) -> String {
    let text = mask(text).unwrap_or_else(|| text.to_owned());
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text = urls(&text);
    let text = authorizations(&text);
    let text = identifiers(&text);
    let text = secret_pairs(&text);
    let text = tokens(&text);
    let text = addresses(&text);
    let text = bare_identifiers(&text);
    let text = user_folders(&text);
    let text = text.trim();
    let mut end = text.len().min(MAX_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Replaces each `(start, end)` range of `text`, in order and not overlapping, with its
/// replacement.
fn replace(text: &str, mut ranges: Vec<(usize, usize, &str)>) -> String {
    ranges.sort_unstable_by_key(|&(start, end, _)| (start, end));
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (start, end, replacement) in ranges {
        if start < copied {
            continue;
        }
        out.push_str(&text[copied..start]);
        out.push_str(replacement);
        copied = end;
    }
    out.push_str(&text[copied..]);
    out
}

fn end_of(text: &str, from: usize, inside: impl Fn(char) -> bool) -> usize {
    text[from..]
        .char_indices()
        .find(|&(_, c)| !inside(c))
        .map_or(text.len(), |(at, _)| from + at)
}

fn word_start(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

fn word_end(text: &str, at: usize) -> bool {
    text[at..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

/// Credentials in a URL's user part, and invite codes and tokens in its query and fragment.
fn urls(text: &str) -> String {
    let mut ranges = Vec::new();
    for (start, _) in text.match_indices("://") {
        let host = start + 3;
        let end = end_of(text, host, |c| !c.is_whitespace() && !"\"'<>".contains(c));
        let url = &text[host..end];
        let authority = url.find(['/', '?', '#']).unwrap_or(url.len());
        if let Some(at) = url[..authority].rfind('@') {
            ranges.push((host, host + at, "[REDACTED]"));
        }
        if let Some(cut) = url.find(['?', '#']) {
            ranges.push((host + cut, end, "?[redacted]"));
        }
    }
    replace(text, ranges)
}

/// `Bearer x`, and `Basic x` when `x` looks encoded: "basic" is also an ordinary word.
fn authorizations(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for scheme in SCHEMES {
        for (start, _) in lower.match_indices(scheme) {
            if !word_start(text, start) {
                continue;
            }
            let value = start + scheme.len();
            let end = end_of(text, value, |c| !c.is_whitespace() && !"\"',;".contains(c));
            if end > value && (scheme == "bearer " || encoded(&text[value..end])) {
                ranges.push((value, end, "[REDACTED]"));
            }
        }
    }
    replace(text, ranges)
}

fn encoded(value: &str) -> bool {
    value.len() >= 8
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
        && value
            .chars()
            .any(|c| c.is_ascii_digit() || c.is_ascii_uppercase() || "+/=".contains(c))
}

fn identifiers(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for (name, replacement) in IDENTIFIERS {
        for (start, _) in lower.match_indices(name) {
            let after = start + name.len();
            if !word_start(text, start) || !word_end(text, after) {
                continue;
            }
            // The name, perhaps quoted, then `:` or `=`, perhaps spaced or quoted, then the value.
            let separator = end_of(text, after, |c| c == '"' || c == ' ');
            if !text[separator..].starts_with([':', '=']) {
                continue;
            }
            // `=>`, as in a Lua or PHP table.
            let arrow = usize::from(text[separator..].starts_with("=>"));
            let value = end_of(text, separator + 1 + arrow, |c| c == '"' || c == ' ');
            let end = end_of(text, value, |c| c.is_ascii_alphanumeric() || ".:[]_-".contains(c));
            if end > value {
                ranges.push((start, end, replacement));
            }
        }
    }
    replace(text, ranges)
}

/// `token=x`, `token: x`, `"token":"x"` and `token => x`, a quoted value whole.
fn secret_pairs(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for (separator, _) in lower.match_indices([':', '=']) {
        let before = lower[..separator].trim_end_matches([' ', '"']);
        let name_start = before
            .rfind(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map_or(0, |at| at + 1);
        if !SECRET_NAMES
            .iter()
            .any(|secret| before[name_start..].ends_with(secret))
        {
            continue;
        }
        let arrow = usize::from(text[separator..].starts_with("=>"));
        let mut value = end_of(text, separator + 1 + arrow, |c| c == ' ');
        let end = match text[value..].chars().next() {
            Some(quote @ ('"' | '\'')) => {
                value += 1;
                text[value..].find(quote).map_or(text.len(), |at| value + at)
            }
            _ => end_of(text, value, |c| !c.is_whitespace() && !"&;,\"'".contains(c)),
        };
        if end > value {
            ranges.push((value, end, "[REDACTED]"));
        }
    }
    replace(text, ranges)
}

/// Hardware tokens, `[0-9a-f]{1,2}:[0-9a-f]{64}`, which ban resources print.
fn tokens(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_hexdigit);
    let mut ranges = Vec::new();
    for (colon, _) in text.match_indices(':') {
        let digits = (1..=2)
            .take_while(|&back| colon >= back && hex(colon - back))
            .count();
        let run = bytes.get(colon + 1..).map_or(0, |rest| {
            rest.iter().take_while(|byte| byte.is_ascii_hexdigit()).count()
        });
        let start = colon - digits;
        if digits > 0 && run == 64 && (start == 0 || !hex(start - 1)) {
            ranges.push((start, colon + 1 + run, "[token]"));
        }
    }
    replace(text, ranges)
}

fn addresses(text: &str) -> String {
    let mut ranges = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let end = end_of(text, at, |c| c.is_ascii_hexdigit() || ".:[]".contains(c));
        if end > at {
            // `endpoint:203.0.113.9`: the address starts after the name's colon, unless the
            // colons are its own, as in `::ffff:10.0.0.1`.
            let after_colons = end - text[at..end].trim_start_matches(':').len();
            let start = if is_address(&text[at..end]) {
                at
            } else {
                after_colons
            };
            if start < end && is_address(&text[start..end]) && word_start(text, start) {
                ranges.push((start, end, "[ip]"));
            }
            at = end;
        } else {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    replace(text, ranges)
}

fn is_address(candidate: &str) -> bool {
    let candidate = candidate.trim_end_matches(['.', ':']);
    if !candidate.bytes().any(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let host = match candidate.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None if candidate.matches(':').count() == 1 && candidate.contains('.') => {
            candidate.split(':').next().unwrap_or("")
        }
        None => candidate,
    };
    host.parse::<IpAddr>().is_ok()
}

/// A licence without its prefix (40 hex digits), a Steam id in hex (`1100001` and 8 more) and a
/// Discord id or mention (17 to 20 digits).
fn bare_identifiers(text: &str) -> String {
    let mut ranges = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let end = end_of(text, at, |c| c.is_ascii_hexdigit());
        if end > at {
            let run = &text[at..end];
            let digits = run.bytes().all(|byte| byte.is_ascii_digit());
            if word_start(text, at) && word_end(text, end) {
                if run.len() == 40 {
                    ranges.push((at, end, "[license]"));
                } else if run.len() == 15 && run.starts_with("1100001") {
                    ranges.push((at, end, "[steam]"));
                } else if digits && (17..=20).contains(&run.len()) {
                    ranges.push((at, end, "[discord]"));
                }
            }
            at = end;
        } else {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    replace(text, ranges)
}

/// The account name in a user's folder, often the person's own: up to the next separator, or,
/// when none comes soon or blanks surround it, up to its first blank.
fn user_folders(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for marker in USER_FOLDERS {
        for (start, _) in lower.match_indices(marker) {
            let name = end_of(text, start + marker.len(), |c| c == ' ');
            let mut end = end_of(text, name, |c| !"\\/\"'<>".contains(c) && !c.is_control());
            if end - name > MAX_USER_NAME || text[name..end].trim_end() != &text[name..end] {
                end = text[name..end]
                    .find(char::is_whitespace)
                    .map_or(end, |at| name + at);
            }
            if end > name {
                ranges.push((name, end, "[user]"));
            }
        }
    }
    replace(text, ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_addresses_come_out_of_a_kick_text() {
        assert_eq!(
            redact(
                "Banned license:0123456789abcdef0123456789abcdef01234567 (discord:123456789012345678) from 192.0.2.7:30120"
            ),
            "Banned [license] ([discord]) from [ip]"
        );
        assert_eq!(
            redact("seen at [2001:db8::1]:30120 and ::ffff:10.0.0.1"),
            "seen at [ip] and [ip]"
        );
    }

    #[test]
    fn identifiers_come_out_however_they_are_written() {
        assert_eq!(
            redact("license: 0123456789abcdef0123456789abcdef01234567 banned"),
            "[license] banned"
        );
        assert_eq!(
            redact(r#"{"discord":"123456789012345678","steam": "110000112345678"}"#),
            r#"{"[discord]","[steam]"}"#
        );
        assert_eq!(
            redact("ban 0123456789abcdef0123456789abcdef01234567 by <@123456789012345678>"),
            "ban [license] by <@[discord]>"
        );
        assert_eq!(
            redact("license => 0123456789abcdef0123456789abcdef01234567, hex 11000011a2b3c4d"),
            "[license], hex [steam]"
        );
    }

    #[test]
    fn an_address_after_a_name_and_a_colon_comes_out() {
        assert_eq!(
            redact("endpoint:203.0.113.9 PlayerIP:203.0.113.10 addr:2001:db8::1"),
            "endpoint:[ip] PlayerIP:[ip] addr:[ip]"
        );
    }

    #[test]
    fn hardware_tokens_come_out_whatever_their_case() {
        let token = format!("3:{}", "AB".repeat(32));

        assert_eq!(redact(&format!("hwid {token} matched")), "hwid [token] matched");
        assert_eq!(redact(&format!("12:{}", "0f".repeat(32))), "[token]");
    }

    #[test]
    fn secrets_credentials_queries_and_user_folders_come_out() {
        assert_eq!(
            redact("join https://discord.gg/abc?code=XYZ#frag or token=s3cr3t&x=1"),
            "join https://discord.gg/abc?[redacted] or token=[REDACTED]&x=1"
        );
        assert_eq!(
            redact("fetch https://bob:hunter2@example.test/x with Authorization: Bearer abc.def"),
            "fetch https://[REDACTED]@example.test/x with Authorization: Bearer [REDACTED]"
        );
        assert_eq!(
            redact(r"crash in C:\Users\Jean Dupont\AppData\Local\FiveM and C:/users/Ada/x"),
            r"crash in C:\Users\[user]\AppData\Local\FiveM and C:/users/[user]/x"
        );
        assert_eq!(
            redact(r#"{"path":"C:\\Users\\jdupont\\FiveM","home":"/home/ada/fivem"}"#),
            r#"{"path":"C:\\Users\\[user]\\FiveM","home":"/home/[user]/fivem"}"#
        );
        assert_eq!(
            redact("webhook https://discord.com/api/webhooks/1/AAAA"),
            "webhook https://discord.com/api/webhooks/[REDACTED]"
        );
    }

    #[test]
    fn secrets_come_out_however_they_are_written() {
        assert_eq!(
            redact(r#"{"password": "hunter 2", "api_key":"k3y"} secret => abc, token: xyz"#),
            r#"{"password": "[REDACTED]", "api_key":"[REDACTED]"} secret => [REDACTED], token: [REDACTED]"#
        );
        assert_eq!(
            redact("Authorization: Basic dXNlcjpwYXNz"),
            "Authorization: Basic [REDACTED]"
        );
    }

    #[test]
    fn a_user_folder_with_no_separator_after_it_ends_at_its_first_blank() {
        let words = " then a long story of what happened while the game was loading its world";

        assert_eq!(
            redact(&format!(r"crash in C:\Users\Ada{words}")),
            format!(r"crash in C:\Users\[user]{words}")
        );
        assert_eq!(
            redact(r"crash in C:\Users\Jean Dupont"),
            r"crash in C:\Users\[user]"
        );
        assert_eq!(redact(r"seen in /home/ ada /x"), r"seen in /home/ [user] /x");
    }

    #[test]
    fn ordinary_words_and_numbers_stay() {
        for text in [
            "AFK for 31 minutes",
            "You were kicked: 12:30 restart",
            "Error 0xC0000005 at deadbeef",
            "Version 1.2 is too old",
            "Server restarting in 1761900000000 ms",
            "live performance license plate",
            "Kicked for basic settings",
        ] {
            assert_eq!(redact(text), text, "{text}");
        }
    }

    #[test]
    fn control_characters_go_and_the_text_is_cut_on_a_character_boundary() {
        assert_eq!(redact("a\u{0007}b\nc"), "a b c");
        let long = "é".repeat(400);

        let cut = redact(&long);

        assert_eq!(cut.len(), MAX_TEXT_BYTES);
    }
}
