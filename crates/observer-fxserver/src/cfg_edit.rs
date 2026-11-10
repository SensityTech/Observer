//! The launch's cfg as the panel edits it: its text with each secret replaced by a mark, what its
//! lines set and start, and the bytes a save writes. Nothing here opens a file but the scan.

use std::ops::Range;
use std::path::Path;

use observer_domain::cfg::{
    CfgEndpointLine, CfgOutline, CfgResourceLine, CfgReview, CfgSetting, MAX_TEXT_BYTES, SECRET_CLOSE,
    SECRET_OPEN,
};
use observer_domain::server_data::EndpointProtocol;

use crate::cfg::{self, Context};
use crate::changes::mask;
use crate::command;
use crate::server_data::Resources;

/// Lines of each kind an outline lists.
const MAX_LISTED: usize = 5000;
const BOM: char = '\u{feff}';
const SET_COMMANDS: [&str; 4] = ["set", "sets", "setr", "seta"];
/// The convars the panel has a field for: a cfg may set them by their name alone, which FXServer
/// reads as a command of any other name.
const FIELDS: [&str; 9] = [
    "sv_hostname",
    "sv_projectName",
    "sv_projectDesc",
    "tags",
    "locale",
    "sv_maxclients",
    "sv_enforceGameBuild",
    "sv_licenseKey",
    "steam_webApiKey",
];

/// Why a file is not edited from the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreadable {
    /// Saving it would rewrite every byte that is not UTF-8.
    NotUtf8,
    /// It holds text written like a secret's mark, which a save could not tell from one.
    HoldsMarks,
}

/// Why an edited text is not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A mark that stands for no secret of the file.
    UnknownMark,
    /// A NUL, which no cfg holds.
    Control,
    TooLarge,
}

/// A cfg read for editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editable {
    /// What the panel shows and edits: every line ended by `\n`, each secret as its mark.
    pub text: String,
    /// What the marks stand for, in the order of the file.
    secrets: Vec<String>,
    crlf: bool,
    bom: bool,
}

impl Editable {
    /// Reads the bytes of `file`, a cfg of the exec chain named from the server folder.
    pub fn read(bytes: &[u8], file: &str) -> Result<Self, Unreadable> {
        let text = std::str::from_utf8(bytes).map_err(|_| Unreadable::NotUtf8)?;
        if text.contains(SECRET_OPEN) {
            return Err(Unreadable::HoldsMarks);
        }
        let bom = text.starts_with(BOM);
        let text = text.strip_prefix(BOM).unwrap_or(text);
        let crlf = text.matches("\r\n").count() * 2 > text.matches('\n').count();
        let text = unix_lines(text);
        let mut shown = String::with_capacity(text.len());
        let mut secrets = Vec::new();
        let mut copied = 0;
        for secret in line_bound(&text, merged(mask::secrets(&text, file, true))) {
            shown.push_str(&text[copied..secret.start]);
            secrets.push(text[secret.clone()].to_owned());
            shown.push_str(&mark(secrets.len()));
            copied = secret.end;
        }
        shown.push_str(&text[copied..]);
        Ok(Self {
            text: shown,
            secrets,
            crlf,
            bom,
        })
    }

    pub fn secrets(&self) -> usize {
        self.secrets.len()
    }

    /// `edited` with each mark of the file back to its secret, lines ended by `\n`. A mark that
    /// stands for nothing stays as written: this is for reading the text, not for writing it.
    pub fn revealed(&self, edited: &str) -> String {
        self.reveal(&unix_lines(edited)).0
    }

    /// The bytes to write for `edited`: each mark back to its secret, the file's line ends and
    /// byte order mark as they were.
    pub fn write(&self, edited: &str) -> Result<Vec<u8>, Refused> {
        if edited.len() > MAX_TEXT_BYTES {
            return Err(Refused::TooLarge);
        }
        if edited.contains('\0') {
            return Err(Refused::Control);
        }
        let (text, unknown) = self.reveal(&unix_lines(edited));
        if unknown {
            return Err(Refused::UnknownMark);
        }
        let text = if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text
        };
        let mut bytes = Vec::with_capacity(text.len() + BOM.len_utf8());
        if self.bom {
            bytes.extend_from_slice(BOM.encode_utf8(&mut [0; 4]).as_bytes());
        }
        bytes.extend_from_slice(text.as_bytes());
        if bytes.len() > MAX_TEXT_BYTES {
            return Err(Refused::TooLarge);
        }
        Ok(bytes)
    }

    /// `text` with its marks replaced, and whether one of them stands for no secret.
    fn reveal(&self, text: &str) -> (String, bool) {
        let mut revealed = String::with_capacity(text.len());
        let mut unknown = false;
        let mut rest = text;
        while let Some(at) = rest.find(SECRET_OPEN) {
            let after = &rest[at + SECRET_OPEN.len()..];
            let secret = after.split_once(SECRET_CLOSE).and_then(|(number, _)| {
                let number: usize = number.parse().ok()?;
                Some((
                    self.secrets.get(number.checked_sub(1)?)?,
                    number.to_string().len(),
                ))
            });
            revealed.push_str(&rest[..at]);
            if let Some((secret, digits)) = secret {
                revealed.push_str(secret);
                rest = &after[digits + SECRET_CLOSE.len()..];
            } else {
                unknown = true;
                revealed.push_str(SECRET_OPEN);
                rest = after;
            }
        }
        revealed.push_str(rest);
        (revealed, unknown)
    }
}

fn mark(number: usize) -> String {
    format!("{SECRET_OPEN}{number}{SECRET_CLOSE}")
}

/// `text` with every line ended by `\n`, as a browser's text field holds it.
fn unix_lines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The secrets in order, those that touch or overlap made one.
pub(crate) fn merged(mut secrets: Vec<Range<usize>>) -> Vec<Range<usize>> {
    secrets.sort_by_key(|secret| secret.start);
    let mut out: Vec<Range<usize>> = Vec::with_capacity(secrets.len());
    for secret in secrets {
        match out.last_mut() {
            Some(last) if secret.start <= last.end => last.end = last.end.max(secret.end),
            _ => out.push(secret),
        }
    }
    out
}

/// Each secret cut at the line ends it runs over, so that a mark never takes a line away.
fn line_bound(text: &str, secrets: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut out = Vec::with_capacity(secrets.len());
    for secret in secrets {
        let mut start = secret.start;
        for (offset, _) in text[secret.clone()].match_indices('\n') {
            out.push(start..secret.start + offset);
            start = secret.start + offset + 1;
        }
        out.push(start..secret.end);
    }
    out.retain(|secret| !secret.is_empty());
    out
}

/// What the cfg `cfg_file` of `data` sets and starts as `shown`, the text the panel holds, and
/// what the scan finds in `real`, the same text with its secrets, and in the files it runs.
pub fn check(
    data: &Path,
    cfg_file: &str,
    shown: &str,
    real: &str,
    resources: &Resources,
    context: &Context,
) -> CfgReview {
    let scan = cfg::scan(data, cfg_file, real, resources, context);
    let system = context
        .system_resources
        .as_deref()
        .map(cfg::system_resources)
        .unwrap_or_default();
    let holds = |name: &str| resources.holds(name) || system.contains(&name.to_ascii_lowercase());
    CfgReview {
        outline: outline(shown, holds),
        findings: scan.findings,
        added: 0,
        removed: 0,
    }
}

/// The lines of `text` the panel edits by field, read the way FXServer reads them. `holds` says
/// whether the server has a resource or a category.
pub fn outline(text: &str, holds: impl Fn(&str) -> bool) -> CfgOutline {
    let mut outline = CfgOutline::default();
    for (index, line) in text.split('\n').enumerate() {
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let commands = command::spans(line);
        if commands.is_empty() {
            if let Some(name) = commented_resource(line).filter(|name| holds(name)) {
                outline.resources.push(CfgResourceLine {
                    line: number,
                    name,
                    enabled: false,
                    alone: true,
                    missing: false,
                });
            }
            continue;
        }
        let alone = commands.len() == 1;
        for span in commands {
            let words = command::word_spans(&line[span.clone()]);
            let Some((_, first)) = words.first() else {
                continue;
            };
            let lower = first.to_ascii_lowercase();
            let valued = |values: &[(Range<usize>, String)]| value(line, span.start, values);
            match lower.as_str() {
                "ensure" | "start" if words.len() == 2 => outline.resources.push(CfgResourceLine {
                    line: number,
                    missing: !holds(&words[1].1),
                    name: words[1].1.clone(),
                    enabled: true,
                    alone,
                }),
                "endpoint_add_tcp" | "endpoint_add_udp" if words.len() >= 2 => {
                    let (start, end, _) = valued(&words[1..2]);
                    outline.endpoints.push(CfgEndpointLine {
                        line: number,
                        protocol: if lower.ends_with("tcp") {
                            EndpointProtocol::Tcp
                        } else {
                            EndpointProtocol::Udp
                        },
                        address: words[1].1.clone(),
                        start,
                        end,
                        alone,
                    });
                }
                command if SET_COMMANDS.contains(&command) && words.len() >= 3 => {
                    let (start, end, value) = valued(&words[2..]);
                    outline.settings.push(CfgSetting {
                        line: number,
                        command: Some(lower.clone()),
                        name: words[1].1.clone(),
                        value,
                        start,
                        end,
                        alone,
                    });
                }
                _ if words.len() >= 2 && FIELDS.iter().any(|field| field.eq_ignore_ascii_case(first)) => {
                    let (start, end, value) = valued(&words[1..]);
                    outline.settings.push(CfgSetting {
                        line: number,
                        command: None,
                        name: first.clone(),
                        value,
                        start,
                        end,
                        alone,
                    });
                }
                _ => {}
            }
        }
    }
    for listed in [
        outline.settings.len(),
        outline.endpoints.len(),
        outline.resources.len(),
    ] {
        outline.truncated |= listed > MAX_LISTED;
    }
    outline.settings.truncate(MAX_LISTED);
    outline.endpoints.truncate(MAX_LISTED);
    outline.resources.truncate(MAX_LISTED);
    outline
}

/// Where the words `values` of the command at `offset` of `line` lie, in UTF-16 code units, and
/// what they give: the one word as FXServer reads it, or several as they are written. Nothing for
/// a value that holds a secret's mark.
fn value(line: &str, offset: usize, values: &[(Range<usize>, String)]) -> (u32, u32, Option<String>) {
    let (Some((first, word)), Some((last, _))) = (values.first(), values.last()) else {
        return (0, 0, None);
    };
    let span = offset + first.start..offset + last.end;
    let written = line.get(span.clone()).unwrap_or_default();
    let units = |byte: usize| {
        let before = line.get(..byte).unwrap_or(line);
        u32::try_from(before.encode_utf16().count()).unwrap_or(u32::MAX)
    };
    let value = if written.contains(SECRET_OPEN) {
        None
    } else if values.len() == 1 {
        Some(word.clone())
    } else {
        Some(written.to_owned())
    };
    (units(span.start), units(span.end), value)
}

/// The resource a line commented out as a whole would start: `#ensure maps`, `// start maps`.
fn commented_resource(line: &str) -> Option<String> {
    let line = line.trim_start();
    let rest = line.strip_prefix('#').or_else(|| line.strip_prefix("//"))?;
    let [command] = command::spans(rest).try_into().ok()?;
    let words = command::words(&rest[command]);
    match words.as_slice() {
        [verb, name] if verb.eq_ignore_ascii_case("ensure") || verb.eq_ignore_ascii_case("start") => {
            Some(name.clone())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "endpoint_add_tcp \"0.0.0.0:30120\"\r\nendpoint_add_udp \"0.0.0.0:30120\"\r\n\
        sv_hostname \"Caf\u{e9} \u{1f600} RP\"\r\nsets tags \"roleplay, fr\"\r\n\
        sv_licenseKey cfxk_1a2b3c4d5e6f7g8h9i0j_AbC123\r\n\
        set mysql_connection_string \"mysql://observer:hunter2@localhost/lab\"\r\n\
        ensure chat\r\n#ensure [xmas]\r\n";

    fn editable(text: &str) -> Editable {
        Editable::read(text.as_bytes(), "server.cfg").unwrap()
    }

    #[test]
    fn a_cfg_is_shown_with_a_mark_in_place_of_each_secret() {
        let cfg = editable(CFG);

        assert_eq!(cfg.secrets(), 2);
        assert!(!cfg.text.contains('\r'));
        assert!(
            !cfg.text.contains("hunter2") && !cfg.text.contains("1a2b3c"),
            "{}",
            cfg.text
        );
        assert_eq!(cfg.text.lines().count(), CFG.lines().count());
        let marks: Vec<&str> = cfg
            .text
            .lines()
            .filter(|line| line.contains(SECRET_OPEN))
            .collect();
        assert_eq!(marks.len(), 2, "{}", cfg.text);
        assert!(marks[0].starts_with("sv_licenseKey ") && marks[0].contains("<<secret-1>>"));
        assert!(marks[1].starts_with("set mysql_connection_string ") && marks[1].contains("<<secret-2>>"));
    }

    #[test]
    fn a_text_left_as_shown_is_written_as_the_file_was_byte_for_byte() {
        for text in [
            CFG.to_owned(),
            CFG.replace("\r\n", "\n"),
            format!("\u{feff}{CFG}"),
            "sv_hostname \"no line end\"".to_owned(),
            String::new(),
        ] {
            let cfg = editable(&text);
            assert_eq!(cfg.write(&cfg.text).unwrap(), text.as_bytes(), "{text:?}");
        }
    }

    #[test]
    fn a_save_keeps_the_secrets_whose_marks_stay_and_takes_what_is_typed_over_one() {
        let cfg = editable(CFG);
        let edited = cfg
            .text
            .replace("roleplay, fr", "roleplay, fr, serious")
            .replace("<<secret-2>>", "\"mysql://observer:another@localhost/lab\"");

        let written = String::from_utf8(cfg.write(&edited).unwrap()).unwrap();

        assert!(written.contains("sv_licenseKey cfxk_1a2b3c4d5e6f7g8h9i0j_AbC123\r\n"));
        assert!(written.contains("\"mysql://observer:another@localhost/lab\"\r\n"));
        assert!(written.contains("sets tags \"roleplay, fr, serious\"\r\n"));
        assert!(!written.contains("hunter2") && !written.contains(SECRET_OPEN));
        assert_eq!(cfg.revealed(&edited), written.replace("\r\n", "\n"));
    }

    #[test]
    fn a_mark_that_stands_for_no_secret_is_refused_rather_than_written() {
        let cfg = editable(CFG);
        for mark in ["<<secret-3>>", "<<secret-0>>", "<<secret-x>>", "<<secret-1"] {
            let edited = format!("{}set rcon_password {mark}\n", cfg.text);
            assert_eq!(cfg.write(&edited), Err(Refused::UnknownMark), "{mark}");
            assert!(cfg.revealed(&edited).contains(mark), "read as written");
        }
        assert_eq!(cfg.write("sv_hostname \"a\0b\""), Err(Refused::Control));
        assert_eq!(cfg.write(&"#".repeat(MAX_TEXT_BYTES + 1)), Err(Refused::TooLarge));
    }

    #[test]
    fn a_file_that_is_not_utf8_or_holds_a_mark_of_its_own_is_not_edited() {
        assert_eq!(
            Editable::read(b"sv_hostname \"Caf\xe9\"\n", "server.cfg"),
            Err(Unreadable::NotUtf8)
        );
        assert_eq!(
            Editable::read(b"sv_hostname \"<<secret-1>>\"\n", "server.cfg"),
            Err(Unreadable::HoldsMarks)
        );
    }

    #[test]
    fn the_line_ends_of_a_file_are_those_most_of_its_lines_have() {
        let mostly_unix = editable("a 1\nb 2\nc 3\r\n");
        let mostly_windows = editable("a 1\r\nb 2\r\nc 3\n");
        let old_mac = editable("a 1\rb 2\r");

        assert_eq!(mostly_unix.write("a 1\nb 2\n").unwrap(), b"a 1\nb 2\n");
        assert_eq!(mostly_windows.write("a 1\nb 2\n").unwrap(), b"a 1\r\nb 2\r\n");
        assert_eq!(old_mac.text, "a 1\nb 2\n");
    }

    #[test]
    fn the_outline_places_each_value_in_its_line_as_a_browser_counts() {
        let cfg = editable(CFG);
        let outline = outline(&cfg.text, |name| name == "chat" || name == "[xmas]");
        let line = |number: u32| cfg.text.lines().nth(number as usize - 1).unwrap();
        let written = |number: u32, start: u32, end: u32| {
            let units: Vec<u16> = line(number).encode_utf16().collect();
            String::from_utf16(&units[start as usize..end as usize]).unwrap()
        };

        let settings: Vec<(u32, Option<&str>, &str, Option<&str>)> = outline
            .settings
            .iter()
            .map(|setting| {
                (
                    setting.line,
                    setting.command.as_deref(),
                    setting.name.as_str(),
                    setting.value.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            settings,
            [
                (3, None, "sv_hostname", Some("Caf\u{e9} \u{1f600} RP")),
                (4, Some("sets"), "tags", Some("roleplay, fr")),
                (5, None, "sv_licenseKey", None),
                (6, Some("set"), "mysql_connection_string", None),
            ]
        );
        let hostname = &outline.settings[0];
        assert_eq!(
            written(3, hostname.start, hostname.end),
            "\"Caf\u{e9} \u{1f600} RP\"",
            "an emoji counts for two"
        );
        let endpoints: Vec<(u32, EndpointProtocol, &str)> = outline
            .endpoints
            .iter()
            .map(|endpoint| (endpoint.line, endpoint.protocol, endpoint.address.as_str()))
            .collect();
        assert_eq!(
            endpoints,
            [
                (1, EndpointProtocol::Tcp, "0.0.0.0:30120"),
                (2, EndpointProtocol::Udp, "0.0.0.0:30120")
            ]
        );
        assert_eq!(
            written(1, outline.endpoints[0].start, outline.endpoints[0].end),
            "\"0.0.0.0:30120\""
        );
        assert!(!outline.truncated);
    }

    #[test]
    fn the_outline_lists_what_a_cfg_starts_and_what_it_would_were_a_line_not_commented_out() {
        let text = "ensure chat\nstart [Maps] # the island\nensure gone; ensure chat\n\
            #ensure [xmas]\n  // start chat\n# ensure you have set the key\n# start here\n\
            ensure\nensure a b\nrestart chat\n";

        let outline = outline(text, |name| {
            ["chat", "[maps]", "[xmas]"].contains(&name.to_ascii_lowercase().as_str())
        });

        let resources: Vec<(u32, &str, bool, bool, bool)> = outline
            .resources
            .iter()
            .map(|resource| {
                (
                    resource.line,
                    resource.name.as_str(),
                    resource.enabled,
                    resource.alone,
                    resource.missing,
                )
            })
            .collect();
        assert_eq!(
            resources,
            [
                (1, "chat", true, true, false),
                (2, "[Maps]", true, true, false),
                (3, "gone", true, false, true),
                (3, "chat", true, false, false),
                (4, "[xmas]", false, true, false),
                (5, "chat", false, true, false),
            ],
            "a comment that only reads like a command names nothing the folder holds"
        );
    }

    #[test]
    fn a_value_of_several_words_is_given_as_written_and_a_bare_name_only_for_the_fields() {
        let outline = outline(
            "sv_hostname My Server // public\nset sv_lan 1\nsv_lan 1\nSV_MAXCLIENTS 64\nsets tags\n",
            |_| false,
        );

        let settings: Vec<(&str, Option<&str>)> = outline
            .settings
            .iter()
            .map(|setting| (setting.name.as_str(), setting.value.as_deref()))
            .collect();
        assert_eq!(
            settings,
            [
                ("sv_hostname", Some("My Server")),
                ("sv_lan", Some("1")),
                ("SV_MAXCLIENTS", Some("64"))
            ]
        );
    }
}
