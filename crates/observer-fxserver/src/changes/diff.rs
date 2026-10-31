//! Line and word diffs of two masked texts, by pages. A hidden secret takes part in the diff by its
//! tag, so that a secret that changed shows on its line, and never with its tag: the panel only
//! learns that it changed.

use std::fmt::Write;
use std::time::{Duration, Instant};

use observer_domain::changes::{DiffLine, DiffLineKind, Hunk, Segment, TextDiff, TextPage};
use similar::{
    Algorithm, ChangeTag, DiffOp, DiffTag, TextDiff as Words, capture_diff_slices_deadline, group_diff_ops,
};

use super::mask::{Masked, REDACTED};

/// Lines in one page of a diff or of a whole text.
pub const PAGE_LINES: usize = 500;
/// How long a line diff may take before it settles for a coarser one.
const DEADLINE: Duration = Duration::from_secs(2);
/// Marks a hidden secret inside a prepared text, with its tag in between.
const OPEN: char = '\u{E000}';
const CLOSE: char = '\u{E001}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffOptions {
    /// Unchanged lines shown around each change.
    pub context: usize,
    pub ignore_whitespace: bool,
    pub page: usize,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            context: 3,
            ignore_whitespace: false,
            page: 0,
        }
    }
}

/// A masked text cut into lines, each secret replaced by its marked tag.
struct Prepared {
    /// Each line without its ending.
    lines: Vec<String>,
    endings: Vec<&'static str>,
}

impl Prepared {
    fn new(masked: Option<&Masked>) -> Self {
        let Some(masked) = masked else {
            return Self {
                lines: Vec::new(),
                endings: Vec::new(),
            };
        };
        let mut text = String::with_capacity(masked.text.len());
        let mut copied = 0;
        for redaction in &masked.redactions {
            let at = usize::try_from(redaction.at).unwrap_or(usize::MAX);
            if at < copied || masked.text.get(at..at + REDACTED.len()) != Some(REDACTED) {
                continue;
            }
            text.push_str(&clean(&masked.text[copied..at]));
            text.push(OPEN);
            let _ = write!(text, "{:04x}", redaction.tag);
            text.push(CLOSE);
            copied = at + REDACTED.len();
        }
        text.push_str(&clean(&masked.text[copied..]));
        let mut lines = Vec::new();
        let mut endings = Vec::new();
        for line in text.split_inclusive('\n') {
            let (content, ending) = if let Some(content) = line.strip_suffix("\r\n") {
                (content, "\r\n")
            } else if let Some(content) = line.strip_suffix('\n') {
                (content, "\n")
            } else {
                (line, "")
            };
            lines.push(content.to_owned());
            endings.push(ending);
        }
        Self { lines, endings }
    }

    fn keys(&self, ignore_whitespace: bool) -> Vec<String> {
        if !ignore_whitespace {
            return self.lines.clone();
        }
        self.lines
            .iter()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }
}

/// A text never holds the markers itself: a character that would pass for one shows as `�`.
fn clean(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains([OPEN, CLOSE]) {
        text.replace([OPEN, CLOSE], "\u{FFFD}").into()
    } else {
        text.into()
    }
}

fn ops(old: &Prepared, new: &Prepared, ignore_whitespace: bool) -> Vec<DiffOp> {
    let (old_keys, new_keys) = (old.keys(ignore_whitespace), new.keys(ignore_whitespace));
    capture_diff_slices_deadline(
        Algorithm::Myers,
        &old_keys,
        &new_keys,
        Some(Instant::now() + DEADLINE),
    )
}

/// How many lines were added and removed, without building the diff.
pub fn counts(old: Option<&Masked>, new: Option<&Masked>) -> (u32, u32) {
    let (old, new) = (Prepared::new(old), Prepared::new(new));
    let (mut added, mut removed) = (0, 0);
    for op in ops(&old, &new, false) {
        let (tag, from, to) = op.as_tag_tuple();
        if tag != DiffTag::Equal {
            added += to.len();
            removed += from.len();
        }
    }
    (number(added), number(removed))
}

/// The diff of two versions of a text, either of which may be missing, a page of it.
pub fn diff(old: Option<&Masked>, new: Option<&Masked>, options: DiffOptions) -> TextDiff {
    let (old, new) = (Prepared::new(old), Prepared::new(new));
    let ops = ops(&old, &new, options.ignore_whitespace);
    let (mut added, mut removed) = (0, 0);
    for op in &ops {
        let (tag, from, to) = op.as_tag_tuple();
        if tag != DiffTag::Equal {
            added += to.len();
            removed += from.len();
        }
    }
    let line_endings_only =
        added == 0 && removed == 0 && old.lines == new.lines && old.endings != new.endings;
    let mut hunks = Vec::new();
    for group in group_diff_ops(ops, options.context) {
        let mut lines = Vec::new();
        for op in &group {
            lines.extend(op_lines(op, &old, &new));
        }
        hunks.extend(split(&lines, PAGE_LINES));
    }
    let pages = paginate(&hunks);
    let page = options.page.min(pages.len().saturating_sub(1));
    let hunks = pages
        .get(page)
        .map(|range| hunks[range.clone()].to_vec())
        .unwrap_or_default();
    TextDiff {
        added: number(added),
        removed: number(removed),
        line_endings_only,
        from_total: number(old.lines.len()),
        to_total: number(new.lines.len()),
        hunks,
        page: number(page),
        pages: number(pages.len().max(1)),
    }
}

/// A page of a whole text.
pub fn page(masked: &Masked, page: usize) -> TextPage {
    let prepared = Prepared::new(Some(masked));
    let total = prepared.lines.len();
    let pages = total.div_ceil(PAGE_LINES).max(1);
    let page = page.min(pages - 1);
    let first = page * PAGE_LINES;
    let lines = prepared
        .lines
        .iter()
        .skip(first)
        .take(PAGE_LINES)
        .map(|line| segments(&[(false, line.as_str())]))
        .collect();
    TextPage {
        first: number(first + 1),
        lines,
        total: number(total),
        page: number(page),
        pages: number(pages),
    }
}

fn op_lines(op: &DiffOp, old: &Prepared, new: &Prepared) -> Vec<DiffLine> {
    let (tag, from, to) = op.as_tag_tuple();
    let line = |kind, from: Option<usize>, to: Option<usize>, segments| DiffLine {
        kind,
        from: from.map(|index| number(index + 1)),
        to: to.map(|index| number(index + 1)),
        segments,
    };
    match tag {
        DiffTag::Equal => from
            .zip(to)
            .map(|(a, b)| {
                line(
                    DiffLineKind::Context,
                    Some(a),
                    Some(b),
                    segments(&[(false, &new.lines[b])]),
                )
            })
            .collect(),
        DiffTag::Delete => from
            .map(|a| {
                line(
                    DiffLineKind::Removed,
                    Some(a),
                    None,
                    segments(&[(false, &old.lines[a])]),
                )
            })
            .collect(),
        DiffTag::Insert => to
            .map(|b| {
                line(
                    DiffLineKind::Added,
                    None,
                    Some(b),
                    segments(&[(false, &new.lines[b])]),
                )
            })
            .collect(),
        DiffTag::Replace => {
            let paired = from.len().min(to.len());
            let mut removed = Vec::with_capacity(from.len());
            let mut inserted = Vec::with_capacity(to.len());
            for (a, b) in from.clone().zip(to.clone()) {
                let (before, after) = words(&old.lines[a], &new.lines[b]);
                removed.push(line(DiffLineKind::Removed, Some(a), None, before));
                inserted.push(line(DiffLineKind::Added, None, Some(b), after));
            }
            for a in from.skip(paired) {
                removed.push(line(
                    DiffLineKind::Removed,
                    Some(a),
                    None,
                    segments(&[(false, &old.lines[a])]),
                ));
            }
            for b in to.skip(paired) {
                inserted.push(line(
                    DiffLineKind::Added,
                    None,
                    Some(b),
                    segments(&[(false, &new.lines[b])]),
                ));
            }
            removed.extend(inserted);
            removed
        }
    }
}

/// The two versions of a changed line, each with the words that changed marked.
fn words(old: &str, new: &str) -> (Vec<Segment>, Vec<Segment>) {
    let diff = Words::configure()
        .deadline(Instant::now() + DEADLINE)
        .diff_words(old, new);
    let mut before: Vec<(bool, &str)> = Vec::new();
    let mut after: Vec<(bool, &str)> = Vec::new();
    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            match change.tag() {
                ChangeTag::Equal => {
                    before.push((false, change.value()));
                    after.push((false, change.value()));
                }
                ChangeTag::Delete => before.push((true, change.value())),
                ChangeTag::Insert => after.push((true, change.value())),
            }
        }
    }
    (segments(&before), segments(&after))
}

/// Pieces of a prepared line as segments: runs of one flag joined, and each marked secret a
/// segment of its own, `[REDACTED]`, its tag dropped.
fn segments(pieces: &[(bool, &str)]) -> Vec<Segment> {
    let mut joined: Vec<(bool, String)> = Vec::new();
    for (changed, text) in pieces {
        match joined.last_mut() {
            Some((last, buffer)) if last == changed => buffer.push_str(text),
            _ => joined.push((*changed, (*text).to_owned())),
        }
    }
    let mut segments = Vec::new();
    for (changed, text) in joined {
        let mut rest = text.as_str();
        while let Some(open) = rest.find(OPEN) {
            push(&mut segments, &rest[..open], changed, false);
            let after = &rest[open + OPEN.len_utf8()..];
            let close = after
                .find(CLOSE)
                .map_or(after.len(), |close| close + CLOSE.len_utf8());
            segments.push(Segment {
                text: REDACTED.to_owned(),
                changed,
                redacted: true,
            });
            rest = &after[close..];
        }
        push(&mut segments, rest, changed, false);
    }
    segments
}

fn push(segments: &mut Vec<Segment>, text: &str, changed: bool, redacted: bool) {
    if text.is_empty() {
        return;
    }
    match segments.last_mut() {
        Some(last) if last.changed == changed && !last.redacted && !redacted => last.text.push_str(text),
        _ => segments.push(Segment {
            text: text.to_owned(),
            changed,
            redacted,
        }),
    }
}

/// Hunks longer than `limit` lines cut into several, each with its own header.
fn split(lines: &[DiffLine], limit: usize) -> Vec<Hunk> {
    lines
        .chunks(limit)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| {
            let from: Vec<u32> = chunk.iter().filter_map(|line| line.from).collect();
            let to: Vec<u32> = chunk.iter().filter_map(|line| line.to).collect();
            let start =
                |numbers: &[u32], fallback: Option<u32>| numbers.first().copied().or(fallback).unwrap_or(0);
            Hunk {
                from_start: start(&from, chunk.iter().find_map(|line| line.to)),
                from_lines: number(from.len()),
                to_start: start(&to, chunk.iter().find_map(|line| line.from)),
                to_lines: number(to.len()),
                lines: chunk.to_vec(),
            }
        })
        .collect()
}

/// Hunks gathered in pages of about `PAGE_LINES` lines, a hunk never split across two.
fn paginate(hunks: &[Hunk]) -> Vec<std::ops::Range<usize>> {
    let mut pages = Vec::new();
    let (mut start, mut lines) = (0, 0);
    for (index, hunk) in hunks.iter().enumerate() {
        if lines > 0 && lines + hunk.lines.len() > PAGE_LINES {
            pages.push(start..index);
            start = index;
            lines = 0;
        }
        lines += hunk.lines.len();
    }
    if start < hunks.len() {
        pages.push(start..hunks.len());
    }
    pages
}

fn number(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::mask::{Redaction, mask};

    const KEY: [u8; 32] = [5; 32];

    fn text(text: &str) -> Masked {
        Masked {
            text: text.to_owned(),
            redactions: Vec::new(),
        }
    }

    /// One line per number, as `line(number)` writes it.
    fn lines(count: usize, line: impl Fn(usize) -> String) -> String {
        (0..count).fold(String::new(), |mut text, number| {
            text.push_str(&line(number));
            text.push('\n');
            text
        })
    }

    fn shown(line: &DiffLine) -> String {
        line.segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect()
    }

    #[test]
    fn a_change_shows_with_its_context_and_numbers() {
        let old = text("a\nb\nc\nd\ne\nf\ng\nh\n");
        let new = text("a\nb\nc\nd\nE\nf\ng\nh\n");

        let diff = diff(Some(&old), Some(&new), DiffOptions::default());

        assert_eq!((diff.added, diff.removed), (1, 1));
        assert_eq!(diff.hunks.len(), 1);
        let hunk = &diff.hunks[0];
        assert_eq!(
            (hunk.from_start, hunk.from_lines, hunk.to_start, hunk.to_lines),
            (2, 7, 2, 7)
        );
        let kinds: Vec<(DiffLineKind, Option<u32>, Option<u32>)> = hunk
            .lines
            .iter()
            .map(|line| (line.kind, line.from, line.to))
            .collect();
        assert_eq!(kinds[3], (DiffLineKind::Removed, Some(5), None));
        assert_eq!(kinds[4], (DiffLineKind::Added, None, Some(5)));
        assert_eq!(shown(&hunk.lines[4]), "E");
    }

    #[test]
    fn the_words_that_changed_are_marked() {
        let diff = diff(
            Some(&text("Config.Locale = 'fr'\n")),
            Some(&text("Config.Locale = 'en'\n")),
            DiffOptions::default(),
        );

        let added = &diff.hunks[0].lines[1];
        let changed: Vec<&str> = added
            .segments
            .iter()
            .filter(|segment| segment.changed)
            .map(|segment| segment.text.as_str())
            .collect();
        assert_eq!(shown(added), "Config.Locale = 'en'");
        assert!(changed.iter().any(|word| word.contains("en")), "{changed:?}");
        assert!(!changed.iter().any(|word| word.contains("Config")));
    }

    #[test]
    fn a_secret_that_changed_shows_on_its_line_and_never_its_tag() {
        let old = mask("token = 'one'\nkept = 1\n", "a.lua", false, &KEY);
        let new = mask("token = 'two'\nkept = 1\n", "a.lua", false, &KEY);
        let same = mask("token = 'one'\nkept = 2\n", "a.lua", false, &KEY);

        let changed = diff(Some(&old), Some(&new), DiffOptions::default());
        let unchanged = diff(Some(&old), Some(&same), DiffOptions::default());

        assert_eq!((changed.added, changed.removed), (1, 1));
        let secret = changed.hunks[0].lines[1]
            .segments
            .iter()
            .find(|segment| segment.redacted)
            .unwrap();
        assert_eq!((secret.text.as_str(), secret.changed), ("[REDACTED]", true));
        let context = &unchanged.hunks[0].lines[0];
        assert_eq!(context.kind, DiffLineKind::Context);
        assert!(
            context
                .segments
                .iter()
                .any(|segment| segment.redacted && !segment.changed)
        );
        let json = serde_json::to_string(&changed).unwrap();
        assert!(!json.contains('\u{E000}') && !json.contains(&format!("{:04x}", old.redactions[0].tag)));
    }

    #[test]
    fn line_endings_alone_are_told_apart_and_whitespace_can_be_ignored() {
        let unix = text("a\nb\n");
        let windows = text("a\r\nb\r\n");
        let spaced = text("a\n  b\n");

        let endings = diff(Some(&unix), Some(&windows), DiffOptions::default());
        let ignored = diff(
            Some(&unix),
            Some(&spaced),
            DiffOptions {
                ignore_whitespace: true,
                ..DiffOptions::default()
            },
        );

        assert!(endings.line_endings_only);
        assert_eq!(endings.hunks, Vec::new());
        assert_eq!((ignored.added, ignored.removed), (0, 0));
    }

    #[test]
    fn a_new_file_is_all_added_and_a_text_marker_cannot_be_forged() {
        let new = text("a\n\u{E000}ffff\u{E001}\n");

        let diff = diff(None, Some(&new), DiffOptions::default());

        assert_eq!((diff.added, diff.removed, diff.from_total), (2, 0, 0));
        assert_eq!(shown(&diff.hunks[0].lines[1]), "\u{FFFD}ffff\u{FFFD}");
        assert!(
            diff.hunks[0].lines[1]
                .segments
                .iter()
                .all(|segment| !segment.redacted)
        );
    }

    #[test]
    fn long_diffs_come_by_pages() {
        let old = lines(2000, |line| format!("line {line}"));
        let new = lines(2000, |line| format!("changed {line}"));

        let first = diff(Some(&text(&old)), Some(&text(&new)), DiffOptions::default());
        let last = diff(
            Some(&text(&old)),
            Some(&text(&new)),
            DiffOptions {
                page: 99,
                ..DiffOptions::default()
            },
        );

        assert_eq!(first.pages, 8);
        assert!(first.hunks.iter().map(|hunk| hunk.lines.len()).sum::<usize>() <= PAGE_LINES);
        assert_eq!(last.page, 7);
        assert_eq!(counts(Some(&text(&old)), Some(&text(&new))), (2000, 2000));
    }

    #[test]
    fn a_50000_line_file_diffs_within_a_few_seconds() {
        let old = lines(50_000, |line| format!("local value{line} = {line}"));
        let new = lines(50_000, |line| {
            format!(
                "local value{line} = {}",
                if line % 7 == 0 { line + 1 } else { line }
            )
        });
        let started = Instant::now();

        let diff = diff(Some(&text(&old)), Some(&text(&new)), DiffOptions::default());

        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert!(diff.added > 7000);
    }

    #[test]
    fn a_whole_text_comes_by_pages_with_its_secrets_hidden() {
        let masked = mask(&"token = 'x'\n".repeat(600), "a.lua", false, &KEY);

        let second = page(&masked, 1);

        assert_eq!(
            (second.first, second.total, second.pages, second.lines.len()),
            (501, 600, 2, 100)
        );
        assert!(second.lines[0].iter().any(|segment| segment.redacted));
        assert_eq!(
            Redaction::decode(&Redaction::encode(&masked.redactions)).len(),
            600
        );
    }
}
