//! Secrets hidden from file contents before they are stored: what the console hides, the values
//! cfg files give secret convars, commented out or not, credentials in URLs and command lines,
//! string values given to names that say they hold a secret, in code, data and XML, the
//! credentials of `Authorization` headers, private key blocks, and the shapes of known tokens.
//! Each secret becomes `[REDACTED]` in place, so tabs, line endings and line numbers stay as they
//! were, with a tag that tells two versions of one secret apart without saying anything about it.

use std::ops::Range;

use hmac::{Hmac, KeyInit, Mac};
use observer_domain::changes::extension;
use sha2::Sha256;

use crate::command;
use crate::console::mask as console;

/// The rules' version, stored with each text. When it rises, the upkeep pass masks the texts of
/// older versions again.
pub const VERSION: u32 = 2;
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
/// Webhook URLs whose path is the secret, besides Discord's, written as they are or escaped as
/// JSON may write them.
const WEBHOOK_HOSTS: [&str; 4] = [
    "hooks.slack.com/services/",
    "hooks.slack.com\\/services\\/",
    ".webhook.office.com/",
    ".webhook.office.com\\/",
];

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
        found.extend(console::set_values(text, cfg_secret));
        commented_sets(text, &mut found);
    }
    assignments(text, line_values, &mut found);
    authorization_headers(text, line_values, &mut found);
    private_keys(text, &mut found);
    xml(text, &mut found);
    sql_passwords(text, &mut found);
    let mut start = 0;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let shift = |range: Range<usize>| range.start + start..range.end + start;
        found.extend(console::secrets(line).into_iter().map(shift));
        found.extend(console::command_line_secrets(line).into_iter().map(shift));
        let mut shapes = Vec::new();
        token_shapes(line, &mut shapes);
        discord_webhooks(line, &mut shapes);
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
    // `tokens`, `apiKeys`, `TOKEN_2`: a plural or a number after the word names a secret too.
    let numbered = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let singular = numbered.strip_suffix('s').unwrap_or(numbered);
    [name.as_str(), numbered, singular]
        .iter()
        .any(|name| SECRET_WORDS.iter().any(|word| name.ends_with(word)))
}

/// SQL's `IDENTIFIED BY '…'`, which gives a database account its password.
fn sql_passwords(text: &str, found: &mut Vec<Range<usize>>) {
    // ASCII lowercasing keeps every byte offset.
    let lower = text.to_ascii_lowercase();
    for (at, phrase) in lower.match_indices("identified") {
        let rest = lower[at + phrase.len()..].trim_start_matches([' ', '\t']);
        let Some(rest) = rest.strip_prefix("by") else {
            continue;
        };
        let value = rest.trim_start_matches([' ', '\t']);
        let Some(quote) = value.chars().next().filter(|c| matches!(c, '\'' | '"' | '`')) else {
            continue;
        };
        let open = text.len() - value.len() + 1;
        let line_end = text[open..].find('\n').map_or(text.len(), |end| open + end);
        let close = text[open..line_end]
            .find(quote)
            .map_or(line_end, |end| open + end);
        found.push(open..close);
    }
}

/// Whether a convar a cfg sets holds a secret: one the console knows, or one named like a secret.
fn cfg_secret(name: &str) -> bool {
    secret_name(name) || console::secret_convar(name)
}

/// The values of the `set` commands a cfg comments out, `# set mysql_connection_string "…"` or
/// `// setr steam_webApiKey …`: a secret commented out is a secret all the same.
fn commented_sets(text: &str, found: &mut Vec<Range<usize>>) {
    let mut start = 0;
    for line in text.split('\n') {
        let body = line.trim_start_matches([' ', '\t']);
        if body.starts_with('#') || body.starts_with("//") {
            let commented = body.trim_start_matches(['#', '/', ' ', '\t']);
            let at = start + line.len() - commented.len();
            if command::words(commented)
                .first()
                .is_some_and(|word| console::set_command(word))
            {
                let values = console::set_values(commented, cfg_secret);
                found.extend(values.into_iter().map(|value| value.start + at..value.end + at));
            }
        }
        start += line.len() + 1;
    }
}

fn apply(text: &str, secrets: Vec<Range<usize>>, existing: &[Redaction], tag_key: &[u8; 32]) -> Masked {
    let mut ranges: Vec<Range<usize>> = secrets
        .into_iter()
        .map(|range| {
            let start = (0..=range.start)
                .rev()
                .find(|at| text.is_char_boundary(*at))
                .unwrap_or(0);
            let end = (range.end..=text.len())
                .find(|at| text.is_char_boundary(*at))
                .unwrap_or(text.len());
            start..end
        })
        .collect();
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

/// The tag of a hidden value: a keyed hash cut to 16 bits, which never leaves the server.
pub fn tag(value: &str, key: &[u8; 32]) -> u16 {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(value.as_bytes());
    let bytes = mac.finalize().into_bytes();
    u16::from_be_bytes([bytes[0], bytes[1]])
}

/// String values given to a secret name: `name = "…"`, `name: '…'`, `["name"] = [[…]]`,
/// `"name": "…"`, `` name: `…` ``, `name = """…"""`, typed (`const apiKey: string = '…'`,
/// `local token <const> = "…"`) or alone on the next line, and in files of `LINE_VALUES` an
/// unquoted value to the end of the line or a YAML block below it. Values that are not strings
/// (`bypass = true`, a number, a call) stay.
fn assignments(text: &str, line_values: bool, found: &mut Vec<Range<usize>>) {
    let bytes = text.as_bytes();
    // An operator inside a value already hidden is part of that value. Skipping it keeps the scan
    // linear: an unclosed `[[` hides the rest of the text, and each later `token = [[` would
    // otherwise look for its own `]]` to the end again.
    let mut covered = 0;
    for (at, operator) in text.match_indices(['=', ':']) {
        if at < covered {
            continue;
        }
        let next = bytes.get(at + 1).copied();
        let previous = at.checked_sub(1).map(|index| bytes[index]);
        let comparison = operator == "="
            && (matches!(next, Some(b'=' | b'>'))
                || matches!(previous, Some(b'=' | b'!' | b'<' | b'>' | b'~')));
        if comparison || (operator == ":" && (next == Some(b':') || previous == Some(b':'))) {
            continue;
        }
        let secret = name_before(text, at).is_some_and(secret_name)
            || (operator == "=" && annotated_name(text, at).is_some_and(secret_name));
        if !secret {
            continue;
        }
        if let Some(value) = value_after(text, at + 1, line_values) {
            covered = value.end;
            found.push(value);
        }
    }
}

/// The name an `=` or `:` at `at` assigns to: an identifier, its last part after a `.`, or a
/// quoted key, bracketed or not, past a Lua attribute (`local token <const> =`). On the same line
/// only.
fn name_before(text: &str, at: usize) -> Option<&str> {
    let before = text[..at].trim_end_matches([' ', '\t']);
    let before = without_attribute(before);
    let before = before.strip_suffix(']').unwrap_or(before);
    if let Some(quoted) = before.strip_suffix(['"', '\'']) {
        let quote = &before[before.len() - 1..];
        let start = quoted.rfind(quote)?;
        let name = &quoted[start + 1..];
        return (!name.contains('\n')).then_some(name);
    }
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '$'))
        .map_or(0, |(index, c)| index + c.len_utf8());
    let name = &before[start..];
    (!name.is_empty()).then_some(name)
}

/// `before` without the Lua attribute it may end in: `local token <const>` gives `local token`.
fn without_attribute(before: &str) -> &str {
    before
        .strip_suffix('>')
        .and_then(|inner| {
            let open = inner.rfind('<')?;
            matches!(inner[open + 1..].trim(), "const" | "close")
                .then(|| inner[..open].trim_end_matches([' ', '\t']))
        })
        .unwrap_or(before)
}

/// The name a type annotation stands between and the `=` at `at`: `apiKey` in
/// `const apiKey: string = …` or `token?: string | undefined = …`.
fn annotated_name(text: &str, at: usize) -> Option<&str> {
    let before = &text[..at];
    let head = before.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || " \t_.|<>[],?".contains(c));
    if !before[head.len()..]
        .trim_start()
        .starts_with(|c: char| c.is_ascii_alphabetic())
    {
        return None;
    }
    let head = head.strip_suffix(':')?;
    name_before(text, head.trim_end_matches(['?', '!']).len())
}

/// The string value written from `from`, inside its quotes or brackets, or on the next line when
/// nothing follows on this one.
fn value_after(text: &str, from: usize, line_values: bool) -> Option<Range<usize>> {
    let rest = &text[from..];
    let start = from + rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let rest = &text[start..];
    let line_end = rest.find(['\n', '\r']).map_or(text.len(), |end| start + end);
    if start == line_end {
        return next_line_value(text, line_end);
    }
    match rest.chars().next()? {
        '"' | '\'' if rest.starts_with("\"\"\"") || rest.starts_with("'''") => {
            let open = start + 3;
            let end = text[open..].find(&rest[..3]).map_or(text.len(), |end| open + end);
            Some(open..end)
        }
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
        _ if line_values => line_value(text, start, line_end),
        _ => None,
    }
}

/// A string value alone on the line that starts after `line_end` (`Config.Token =\n    "…"`),
/// unless it is the key of a mapping.
fn next_line_value(text: &str, line_end: usize) -> Option<Range<usize>> {
    let next = line_end + text[line_end..].find('\n')? + 1;
    let rest = &text[next..];
    let start = next + rest.len() - rest.trim_start_matches([' ', '\t']).len();
    if !text[start..].starts_with(['"', '\'', '`', '[']) {
        return None;
    }
    let value = value_after(text, start, false)?;
    let after = text[value.end..]
        .trim_start_matches(['"', '\'', '`', ']', '='])
        .trim_start_matches([' ', '\t']);
    (!after.starts_with(':')).then_some(value)
}

/// An unquoted value from `start` to the end of its line, or the YAML block scalar its line opens.
fn line_value(text: &str, start: usize, line_end: usize) -> Option<Range<usize>> {
    let value = text[start..line_end].trim_end();
    if block_indicator(value) {
        return block_scalar(text, start, line_end);
    }
    let literal = value.is_empty()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
        )
        || value.parse::<f64>().is_ok()
        || value.starts_with(['{', '[', '|', '>', '&', '*', '#', ';']);
    (!literal).then(|| start..start + value.len())
}

/// Whether a value opens a YAML block scalar: `|` or `>`, a chomping sign and an indentation
/// digit, then nothing but a comment.
fn block_indicator(value: &str) -> bool {
    let Some(rest) = value.strip_prefix(['|', '>']) else {
        return false;
    };
    let (header, after) = rest.split_at(rest.find([' ', '\t']).unwrap_or(rest.len()));
    let after = after.trim_start();
    header.len() <= 2
        && header.chars().all(|c| matches!(c, '+' | '-' | '1'..='9'))
        && (after.is_empty() || after.starts_with('#'))
}

/// The lines below the key whose line holds `start` that are indented deeper than the key, blank
/// lines among them: from the first character of the first to the last of the last.
fn block_scalar(text: &str, start: usize, line_end: usize) -> Option<Range<usize>> {
    let line_start = text[..start].rfind('\n').map_or(0, |end| end + 1);
    let key_line = &text[line_start..start];
    let column = key_line.len() - key_line.trim_start_matches([' ', '-']).len();
    let mut value: Option<Range<usize>> = None;
    let mut at = line_end;
    while let Some(next) = text[at..].find('\n').map(|end| at + end + 1) {
        let end = text[next..]
            .find(['\r', '\n'])
            .map_or(text.len(), |end| next + end);
        let line = &text[next..end];
        let content = line.trim_start_matches(' ');
        if !content.trim().is_empty() {
            if line.len() - content.len() <= column {
                break;
            }
            let first = value
                .as_ref()
                .map_or(next + line.len() - content.len(), |value| value.start);
            value = Some(first..next + line.trim_end().len());
        }
        at = end;
    }
    value
}

/// The body of each private key block, PEM or PGP: from the dashes that close its `BEGIN` marker,
/// or the line break right after them, to its `END` marker. The block may span lines, fit on one,
/// or sit in a string with its line breaks escaped (`\n`).
fn private_keys(text: &str, found: &mut Vec<Range<usize>>) {
    // A block hidden to the end of the text covers every marker after it: none is read again.
    let mut covered = 0;
    for (begin, marker) in text.match_indices("-----BEGIN ") {
        if begin < covered {
            continue;
        }
        let label = begin + marker.len();
        let Some(close) = find_near(text, label, "-----") else {
            continue;
        };
        let name = &text[label..close];
        if name.len() > 64 || name.contains('\n') || !name.contains("PRIVATE KEY") {
            continue;
        }
        let after = close + "-----".len();
        let rest = &text[after..];
        let body_rest = rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
            .unwrap_or(rest);
        let body = text.len() - body_rest.len();
        let end = text[body..].find("-----END").map_or(text.len(), |end| body + end);
        found.push(body..end);
        covered = end;
    }
}

/// The credentials an `Authorization` header is given in code and data, after the scheme when
/// there is one: `["Authorization"] = "Bearer …"`, `Authorization: 'Bot …'`,
/// `"Authorization": "Basic …"`, `("Authorization", "…")`.
fn authorization_headers(text: &str, line_values: bool, found: &mut Vec<Range<usize>>) {
    // ASCII lowercasing keeps every byte offset.
    let lower = text.to_ascii_lowercase();
    let mut covered = 0;
    for (start, name) in lower.match_indices("authorization") {
        if start < covered {
            continue;
        }
        let after = &text[start + name.len()..];
        if after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let unquoted = after.trim_start_matches(['"', '\'']);
        let quoted = unquoted.len() < after.len();
        let rest = unquoted.trim_start_matches(']').trim_start_matches([' ', '\t']);
        let assigned = match rest.chars().next() {
            Some('=' | ':') => true,
            Some(',') => quoted,
            _ => false,
        };
        if !assigned || rest[1..].starts_with(['=', ':']) {
            continue;
        }
        if let Some(value) = value_after(text, text.len() - rest.len() + 1, line_values) {
            covered = value.end;
            found.push(credentials(text, value));
        }
    }
}

/// The credentials of an `Authorization` value: what follows its scheme (`Bearer `, `Bot `,
/// `Basic `), or all of it when it has none.
fn credentials(text: &str, value: Range<usize>) -> Range<usize> {
    let written = &text[value.clone()];
    let scheme = written.len()
        - written
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .len();
    let rest = &written[scheme..];
    let blanks = rest.len() - rest.trim_start_matches(' ').len();
    if scheme > 0 && blanks > 0 {
        value.start + scheme + blanks..value.end
    } else {
        value
    }
}

/// Secrets in XML and HTML: the text of an element named like a secret (`<ApiKey>…</ApiKey>`),
/// the `value` attribute of such an element, and the `value` of a tag whose `key` or `name` is a
/// secret's (`<add key="ApiKey" value="…"/>`). Attributes named like a secret are assignments. An
/// element named `key`, as in a property list, names something rather than holds it.
/// How far a quoted attribute value or a CDATA section is looked for: a text with many `<` and
/// no closing quote would otherwise be read again from each of them.
const MAX_MARKUP_VALUE: usize = 16 * 1024;

/// Where `needle` first appears in `text` from `start`, within [`MAX_MARKUP_VALUE`] bytes.
fn find_near(text: &str, start: usize, needle: &str) -> Option<usize> {
    let mut limit = text.len().min(start + MAX_MARKUP_VALUE);
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    text[start..limit].find(needle).map(|offset| start + offset)
}

fn xml(text: &str, found: &mut Vec<Range<usize>>) {
    for (open, _) in text.match_indices('<') {
        let from = open + 1;
        if !text[from..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let end = text[from..]
            .find(|c: char| !xml_name_char(c))
            .map_or(text.len(), |end| from + end);
        let name = &text[from..end];
        let local = name.rsplit(':').next().unwrap_or(name);
        let secret = secret_name(local) && !local.eq_ignore_ascii_case("key");
        let Some(tag) = Tag::read(text, end) else {
            continue;
        };
        // Texts masked with the first rules hid `key="ApiKey"` whole: a hidden key names a secret too.
        let names_secret = tag.attributes.iter().any(|(attribute, value)| {
            let named = &text[value.clone()];
            (attribute.eq_ignore_ascii_case("key") || attribute.eq_ignore_ascii_case("name"))
                && (secret_name(named) || named == REDACTED)
        });
        if secret || names_secret {
            found.extend(
                tag.attributes
                    .into_iter()
                    .filter(|(attribute, _)| attribute.eq_ignore_ascii_case("value"))
                    .map(|(_, value)| value),
            );
        }
        if secret && let Some(content) = tag.content {
            found.extend(element_text(text, name, content));
        }
    }
}

fn xml_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')
}

/// An XML or HTML tag: its attributes, each with the range of its quoted value, and where its
/// content starts, unless it closes itself.
struct Tag<'a> {
    attributes: Vec<(&'a str, Range<usize>)>,
    content: Option<usize>,
}

impl<'a> Tag<'a> {
    /// The tag whose name ends at `from`, `None` when what follows is not one.
    fn read(text: &'a str, from: usize) -> Option<Self> {
        let mut attributes = Vec::new();
        let mut at = from;
        loop {
            let rest = text[at..].trim_start();
            at = text.len() - rest.len();
            if rest.starts_with('>') || rest.starts_with("/>") {
                let content = rest.starts_with('>').then_some(at + 1);
                return Some(Self { attributes, content });
            }
            let name_len = rest.find(|c: char| !xml_name_char(c)).unwrap_or(rest.len());
            if name_len == 0 {
                return None;
            }
            let after = rest[name_len..].trim_start();
            let Some(value) = after.strip_prefix('=') else {
                at = text.len() - after.len();
                continue;
            };
            let value = value.trim_start();
            let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
            let start = text.len() - value.len() + 1;
            let end = find_near(text, start, if quote == '"' { "\"" } else { "'" })?;
            attributes.push((&rest[..name_len], start..end));
            at = end + 1;
        }
    }
}

/// The text of the element `name` whose content starts at `start`, without the blanks around it,
/// when it holds only text or a CDATA section.
fn element_text(text: &str, name: &str, start: usize) -> Option<Range<usize>> {
    let next = start + text[start..].find('<')?;
    let (value, after) = if let Some(cdata) = text[next..].strip_prefix("<![CDATA[") {
        if !text[start..next].trim().is_empty() {
            return None;
        }
        let open = text.len() - cdata.len();
        let close = find_near(text, open, "]]>")?;
        (open..close, close + "]]>".len())
    } else {
        let content = &text[start..next];
        let first = start + content.len() - content.trim_start().len();
        (first..start + content.trim_end().len(), next)
    };
    let closing = text[after..]
        .trim_start()
        .strip_prefix("</")?
        .strip_prefix(name)?;
    closing.trim_start().starts_with('>').then_some(value)
}

/// Discord webhook URLs on any of Discord's hosts, with or without an API version, their `/`
/// written as they are or escaped as JSON may (`\/`): the id and token after `webhooks/`.
fn discord_webhooks(line: &str, found: &mut Vec<Range<usize>>) {
    // ASCII lowercasing keeps every byte offset.
    let lower = line.to_ascii_lowercase();
    for host in ["discord.com", "discordapp.com"] {
        for (start, _) in lower.match_indices(host) {
            let Some(("api", at)) = path_segment(&lower, start + host.len()) else {
                continue;
            };
            let Some((mut segment, mut at)) = path_segment(&lower, at) else {
                continue;
            };
            let version = segment
                .strip_prefix('v')
                .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()));
            if version && let Some(next) = path_segment(&lower, at) {
                (segment, at) = next;
            }
            if segment != "webhooks" {
                continue;
            }
            let rest = &lower[at..];
            let Some(token) = rest.strip_prefix('/').or_else(|| rest.strip_prefix("\\/")) else {
                continue;
            };
            let value = line.len() - token.len();
            let end = line[value..]
                .find(|c: char| c.is_whitespace() || "\"'<>`".contains(c))
                .map_or(line.len(), |end| value + end);
            found.push(value..end);
        }
    }
}

/// The path segment after the `/` or `\/` at `at`, and where it ends.
fn path_segment(lower: &str, at: usize) -> Option<(&str, usize)> {
    let rest = &lower[at..];
    let segment = rest.strip_prefix('/').or_else(|| rest.strip_prefix("\\/"))?;
    let start = lower.len() - segment.len();
    let end = segment
        .find(|c: char| !c.is_ascii_alphanumeric())
        .map_or(lower.len(), |end| start + end);
    Some((&lower[start..end], end))
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
    let mut runs = Vec::new();
    let mut opened = None;
    for (index, c) in line.char_indices().chain([(line.len(), ' ')]) {
        match (opened, word(c) || c == '.') {
            (None, true) => opened = Some(index),
            (Some(start), false) => {
                runs.push(start..index);
                opened = None;
            }
            _ => {}
        }
    }
    for range in runs {
        let run = &line[range.clone()];
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
            found.push(range);
        }
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
    fn tokens_after_characters_of_several_bytes_are_hidden_whole() {
        for line in [
            "print('€ eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl')",
            "-- Jeton d’accès: eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl",
            "-- été  eyJhbGciOi.eyJzdWIiOi.c2VjcmV0",
            "print(\"🚨 MTEyMzQ1Njc4OTAxMjM0NTY3OA.GhIjKl.abcdefghijklmnopqrstuvwxyz0123\")",
        ] {
            let shown = masked(line, "a.lua");
            assert!(shown.contains(REDACTED), "{line} -> {shown}");
            assert!(
                !shown.contains("c2Vj") && !shown.contains("abcdefghij"),
                "{line} -> {shown}"
            );
        }
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

    #[test]
    fn authorization_headers_lose_their_credentials_and_keep_their_scheme() {
        let cases = [
            (
                "headers[\"Authorization\"] = \"Bearer abc123\"\n",
                "a.lua",
                "headers[\"Authorization\"] = \"Bearer [REDACTED]\"\n",
            ),
            (
                "fetch(url, { headers: { Authorization: 'Bot MTIz.abc' } })\n",
                "a.js",
                "fetch(url, { headers: { Authorization: 'Bot [REDACTED]' } })\n",
            ),
            (
                "{ \"Authorization\": \"Basic dXNlcjpwYXNz\" }\n",
                "a.json",
                "{ \"Authorization\": \"Basic [REDACTED]\" }\n",
            ),
            (
                "request.setHeader('Authorization: Basic dXNlcjpwYXNz')\n",
                "a.js",
                "request.setHeader('Authorization: Basic [REDACTED]')\n",
            ),
            (
                "client.Headers.Add(\"Authorization\", \"abc123\");\n",
                "a.cs",
                "client.Headers.Add(\"Authorization\", \"[REDACTED]\");\n",
            ),
        ];
        for (text, path, expected) in cases {
            assert_eq!(masked(text, path), expected, "{text}");
        }
    }

    #[test]
    fn webhooks_are_hidden_escaped_versioned_and_on_every_discord_host() {
        let cases = [
            (
                r#"{"url": "https:\/\/discord.com\/api\/webhooks\/123\/tokenABC"}"#,
                r#"{"url": "https:\/\/discord.com\/api\/webhooks\/[REDACTED]"}"#,
            ),
            (
                "url = 'https://discord.com/api/v10/webhooks/123/tokenABC'",
                "url = 'https://discord.com/api/v10/webhooks/[REDACTED]'",
            ),
            (
                "https://ptb.discordapp.com/api/v9/webhooks/123/tokenABC?wait=true",
                "https://ptb.discordapp.com/api/v9/webhooks/[REDACTED]",
            ),
            (
                r"https:\/\/canary.discord.com\/api\/v10\/webhooks\/123\/tokenABC",
                r"https:\/\/canary.discord.com\/api\/v10\/webhooks\/[REDACTED]",
            ),
            (
                r#""https:\/\/hooks.slack.com\/services\/T000\/B000\/XXXX""#,
                r#""https:\/\/hooks.slack.com\/services\/[REDACTED]""#,
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(masked(text, "a.json"), expected, "{text}");
        }
    }

    #[test]
    fn private_key_blocks_are_hidden_on_one_line_in_escaped_strings_and_as_pgp_armour() {
        let cases = [
            (
                "pem -----BEGIN EC PRIVATE KEY----- MHcCAQEE -----END EC PRIVATE KEY-----\n",
                "pem -----BEGIN EC PRIVATE KEY-----[REDACTED]-----END EC PRIVATE KEY-----\n",
            ),
            (
                "local pem = \"-----BEGIN PRIVATE KEY-----\\nMIIEvQIB\\nAKcwggSj\\n-----END PRIVATE KEY-----\\n\"\n",
                "local pem = \"-----BEGIN PRIVATE KEY-----[REDACTED]-----END PRIVATE KEY-----\\n\"\n",
            ),
            (
                "-----BEGIN PGP PRIVATE KEY BLOCK-----\nVersion: GnuPG\n\nlQOYBGV\n=Xq4a\n-----END PGP PRIVATE KEY BLOCK-----\n",
                "-----BEGIN PGP PRIVATE KEY BLOCK-----\n[REDACTED]-----END PGP PRIVATE KEY BLOCK-----\n",
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(masked(text, "a.lua"), expected, "{text}");
        }
    }

    #[test]
    fn typed_assignments_hide_their_string_values() {
        let cases = [
            (
                "const apiKey: string = 'abc';\n",
                "a.ts",
                "const apiKey: string = '[REDACTED]';\n",
            ),
            (
                "private readonly token?: string | undefined = \"abc\";\n",
                "a.ts",
                "private readonly token?: string | undefined = \"[REDACTED]\";\n",
            ),
            (
                "local token <const> = \"abc\"\n",
                "a.lua",
                "local token <const> = \"[REDACTED]\"\n",
            ),
            (
                "local Webhook < const > = 'https://x'\n",
                "a.lua",
                "local Webhook < const > = '[REDACTED]'\n",
            ),
        ];
        for (text, path, expected) in cases {
            assert_eq!(masked(text, path), expected, "{text}");
        }
    }

    #[test]
    fn a_value_alone_on_the_line_after_its_name_is_hidden() {
        assert_eq!(
            masked("Config.Token =\n    \"abc\"\n", "a.lua"),
            "Config.Token =\n    \"[REDACTED]\"\n"
        );
        assert_eq!(
            masked("{\n  \"password\":\r\n    \"abc\"\r\n}\n", "a.json"),
            "{\n  \"password\":\r\n    \"[REDACTED]\"\r\n}\n"
        );
        for (text, path) in [
            ("auth:\n  \"user\": \"ada\"\n", "a.yml"),
            ("Config.Token =\n    GetConvar('bot', '')\n", "a.lua"),
        ] {
            assert_eq!(masked(text, path), text, "{text}");
        }
    }

    #[test]
    fn yaml_blocks_and_toml_multi_line_strings_are_hidden_whole() {
        let cases = [
            (
                "private_key: |\n  line one\n\n  line two\nnext: 1\n",
                "a.yml",
                "private_key: |\n  [REDACTED]\nnext: 1\n",
            ),
            (
                "bots:\n  - api_key: >-\n      abc\n      def\n    name: Lab\n",
                "a.yaml",
                "bots:\n  - api_key: >-\n      [REDACTED]\n    name: Lab\n",
            ),
            (
                "secret = \"\"\"\nabc\ndef\"\"\"\nport = 1\n",
                "a.toml",
                "secret = \"\"\"[REDACTED]\"\"\"\nport = 1\n",
            ),
            ("token = '''abc'''\n", "a.toml", "token = '''[REDACTED]'''\n"),
            (
                "description: |\n  Our lab server\n",
                "a.yml",
                "description: |\n  Our lab server\n",
            ),
        ];
        for (text, path, expected) in cases {
            assert_eq!(masked(text, path), expected, "{text}");
        }
    }

    #[test]
    fn xml_elements_and_attributes_named_like_secrets_are_hidden() {
        let xml = "<config>\n  <ApiKey>abc</ApiKey>\n  <db:Password>\n    hunter2\n  </db:Password>\n  \
                   <Token><![CDATA[abc]]></Token>\n  <add key=\"ApiKey\" value=\"abc\"/>\n  \
                   <param name=\"password\" value='abc' />\n  <service token=\"abc\" url=\"https://x\"/>\n  \
                   <License key=\"abc\"/>\n</config>\n";

        assert_eq!(
            masked(xml, "app.config"),
            "<config>\n  <ApiKey>[REDACTED]</ApiKey>\n  <db:Password>\n    [REDACTED]\n  </db:Password>\n  \
             <Token><![CDATA[[REDACTED]]]></Token>\n  <add key=\"[REDACTED]\" value=\"[REDACTED]\"/>\n  \
             <param name=\"password\" value='[REDACTED]' />\n  <service token=\"[REDACTED]\" url=\"https://x\"/>\n  \
             <License key=\"[REDACTED]\"/>\n</config>\n"
        );
    }

    #[test]
    fn masking_again_hides_the_value_of_a_key_the_first_rules_hid() {
        let older = Masked {
            text: "<add key=\"[REDACTED]\" value=\"abc\"/>\n".to_owned(),
            redactions: vec![Redaction { at: 10, tag: 7 }],
        };

        let again = remask(&older, "app.config", false, &KEY);

        assert_eq!(again.text, "<add key=\"[REDACTED]\" value=\"[REDACTED]\"/>\n");
        assert_eq!(again.redactions[0], older.redactions[0]);
    }

    #[test]
    fn a_commented_out_cfg_secret_is_hidden_too() {
        let cfg = "# set mysql_connection_string \"server=db;password=x\"\n// setr steam_webApiKey ABCDEF\n\
                   #set discord_bot_token xyz\n  ## sets my_api_key \"k\"\n";

        assert_eq!(
            masked(cfg, "server.cfg"),
            "# set mysql_connection_string [REDACTED]\n// setr steam_webApiKey [REDACTED]\n\
             #set discord_bot_token [REDACTED]\n  ## sets my_api_key [REDACTED]\n"
        );
    }

    #[test]
    fn plural_and_numbered_names_and_sql_passwords_are_secrets_too() {
        let lua = "Config.ApiKeys = \"k1\"
local tokens = 'k2'
TOKEN_2 = \"k3\"
local keybind = 'E'
";
        let sql = "CREATE USER 'fivem'@'localhost' IDENTIFIED BY 'S3cretPass';
GRANT ALL ON db.* TO 'fivem';
";

        let lua = masked(lua, "resources/a/config.lua");
        let sql = masked(sql, "resources/a/install.sql");

        assert!(
            !lua.contains("k1") && !lua.contains("k2") && !lua.contains("k3"),
            "{lua}"
        );
        assert!(lua.contains("keybind = 'E'"), "{lua}");
        assert!(!sql.contains("S3cretPass"), "{sql}");
        assert!(sql.contains("GRANT ALL ON db.*"), "{sql}");
    }

    #[test]
    fn markup_that_never_closes_is_read_quickly() {
        let text = "<a x=\"".repeat(200_000);
        let keys = "-----BEGIN PRIVATE KEY-----".repeat(40_000);
        let started = std::time::Instant::now();

        let brackets = "token = [[
"
        .repeat(100_000);
        let found = secrets(&text, "resources/a/data.xml", false);
        let hidden = secrets(&keys, "resources/a/keys.txt", false);
        let unclosed = secrets(&brackets, "resources/a/config.lua", false);

        assert_eq!(found, []);
        assert_eq!(hidden.len(), 1);
        assert_eq!(unclosed, vec![10..brackets.len()]);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn ordinary_code_and_markup_stay_readable() {
        for (text, path) in [
            ("local count = 5\nlocal max <const> = 10\n", "a.lua"),
            ("const count: number = 5;\nconst label: string = 'Lab';\n", "a.ts"),
            ("headers.Authorization = 'Bearer ' .. token\n", "a.lua"),
            ("if (name === 'Authorization') { return }\n", "a.js"),
            (
                "<Item type=\"CWeaponInfo\">\n  <Name>WEAPON_PISTOL</Name>\n  <param name=\"Theme\" value=\"dark\"/>\n</Item>\n",
                "weapons.meta",
            ),
            ("<auth><user>ada</user></auth>\n", "a.xml"),
            ("function f(): Promise<Token> { return a < b }\n", "a.ts"),
            (
                "# set sv_hostname \"Lab\"\n# rcon_password must be long\n",
                "server.cfg",
            ),
            ("https://discord.com/api/v10/channels/1/messages\n", "a.md"),
            (
                "-----BEGIN CERTIFICATE-----\nMIIBszCC\n-----END CERTIFICATE-----\n",
                "a.lua",
            ),
        ] {
            assert_eq!(masked(text, path), text, "{text}");
        }
    }

    #[test]
    fn the_new_shapes_are_hidden_whole_after_accented_text() {
        let cases = [
            (
                "-- Clé d’accès, été\nlocal token <const> = \"abc\"\n",
                "a.lua",
                "-- Clé d’accès, été\nlocal token <const> = \"[REDACTED]\"\n",
            ),
            (
                "-- Déjà vu\nheaders[\"Authorization\"] = \"Bot é1\"\n",
                "a.lua",
                "-- Déjà vu\nheaders[\"Authorization\"] = \"Bot [REDACTED]\"\n",
            ),
            (
                r#"{"é": "https:\/\/discord.com\/api\/v10\/webhooks\/1\/àb"}"#,
                "a.json",
                r#"{"é": "https:\/\/discord.com\/api\/v10\/webhooks\/[REDACTED]"}"#,
            ),
            (
                "résumé: x\nprivate_key: |\n  clé à\nnext: 1\n",
                "a.yml",
                "résumé: x\nprivate_key: |\n  [REDACTED]\nnext: 1\n",
            ),
            (
                "<Note>café</Note><ApiKey>clé</ApiKey>",
                "a.xml",
                "<Note>café</Note><ApiKey>[REDACTED]</ApiKey>",
            ),
            (
                "« ½ » -----BEGIN PRIVATE KEY-----é-----END PRIVATE KEY-----",
                "a.lua",
                "« ½ » -----BEGIN PRIVATE KEY-----[REDACTED]-----END PRIVATE KEY-----",
            ),
            (
                "# Été\n# set my_token \"clé\"\n",
                "server.cfg",
                "# Été\n# set my_token [REDACTED]\n",
            ),
            (
                "const café = 1; const apiKey: string = 'é';\n",
                "a.ts",
                "const café = 1; const apiKey: string = '[REDACTED]';\n",
            ),
        ];
        for (text, path, expected) in cases {
            assert_eq!(masked(text, path), expected, "{text}");
        }
    }
}
