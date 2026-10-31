//! What is taken out of a drop's text before it is stored: secrets, identifiers, addresses,
//! hardware tokens, URL queries and Windows user folders. Kick texts are written by resources,
//! and anticheats print identifiers and tokens in them; identities belong in their own tables.

use std::net::IpAddr;

use observer_fxserver::console::mask;

/// The longest text kept, in bytes.
pub(super) const MAX_TEXT_BYTES: usize = 512;

const IDENTIFIERS: [(&str, &str); 8] = [
    ("license2:", "[license2]"),
    ("license:", "[license]"),
    ("steam:", "[steam]"),
    ("discord:", "[discord]"),
    ("fivem:", "[fivem]"),
    ("xbl:", "[xbl]"),
    ("live:", "[live]"),
    ("ip:", "[ip]"),
];
const SECRET_NAMES: [&str; 5] = ["password", "pwd", "token", "secret", "key"];

pub(super) fn redact(text: &str) -> String {
    let text = mask(text).unwrap_or_else(|| text.to_owned());
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text = url_queries(&text);
    let text = identifiers(&text);
    let text = secret_pairs(&text);
    let text = tokens(&text);
    let text = addresses(&text);
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
fn replace(text: &str, ranges: Vec<(usize, usize, &str)>) -> String {
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

/// Invite codes and tokens live in queries and fragments.
fn url_queries(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for scheme in ["http://", "https://"] {
        for (start, _) in lower.match_indices(scheme) {
            let end = end_of(text, start, |c| !c.is_whitespace() && !"\"'<>".contains(c));
            if let Some(cut) = text[start..end].find(['?', '#']) {
                ranges.push((start + cut, end, "?[redacted]"));
            }
        }
    }
    ranges.sort_unstable();
    replace(text, ranges)
}

fn identifiers(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for (prefix, replacement) in IDENTIFIERS {
        for (start, _) in lower.match_indices(prefix) {
            if !word_start(text, start) {
                continue;
            }
            let end = end_of(text, start + prefix.len(), |c| {
                c.is_ascii_alphanumeric() || ".:[]_-".contains(c)
            });
            ranges.push((start, end, replacement));
        }
    }
    ranges.sort_unstable();
    replace(text, ranges)
}

fn secret_pairs(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for (equals, _) in lower.match_indices('=') {
        let name_start = lower[..equals]
            .rfind(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map_or(0, |at| at + 1);
        let name = &lower[name_start..equals];
        if SECRET_NAMES.iter().any(|secret| name.ends_with(secret)) {
            let end = end_of(text, equals + 1, |c| !c.is_whitespace() && !"&;,\"'".contains(c));
            if end > equals + 1 {
                ranges.push((equals + 1, end, "[REDACTED]"));
            }
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
            if is_address(&text[at..end]) && word_start(text, at) {
                ranges.push((at, end, "[ip]"));
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

/// `C:\Users\<name>\` and its forward-slash form: the account name is often the person's.
fn user_folders(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for marker in [":\\users\\", ":/users/"] {
        for (start, _) in lower.match_indices(marker) {
            let name = start + marker.len();
            let end = end_of(text, name, |c| c != '\\' && c != '/' && !c.is_whitespace());
            if end > name {
                ranges.push((name, end, "[user]"));
            }
        }
    }
    ranges.sort_unstable();
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
    fn hardware_tokens_come_out_whatever_their_case() {
        let token = format!("3:{}", "AB".repeat(32));

        assert_eq!(redact(&format!("hwid {token} matched")), "hwid [token] matched");
        assert_eq!(redact(&format!("12:{}", "0f".repeat(32))), "[token]");
    }

    #[test]
    fn secrets_queries_and_user_folders_come_out() {
        assert_eq!(
            redact("join https://discord.gg/abc?code=XYZ#frag or token=s3cr3t&x=1"),
            "join https://discord.gg/abc?[redacted] or token=[REDACTED]&x=1"
        );
        assert_eq!(
            redact(r"crash in C:\Users\jdupont\AppData\Local\FiveM and C:/users/Ada/x"),
            r"crash in C:\Users\[user]\AppData\Local\FiveM and C:/users/[user]/x"
        );
        assert_eq!(
            redact("webhook https://discord.com/api/webhooks/1/AAAA"),
            "webhook https://discord.com/api/webhooks/[REDACTED]"
        );
    }

    #[test]
    fn ordinary_words_and_numbers_stay() {
        for text in [
            "AFK for 31 minutes",
            "You were kicked: 12:30 restart",
            "Error 0xC0000005 at deadbeef",
            "Version 1.2 is too old",
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
