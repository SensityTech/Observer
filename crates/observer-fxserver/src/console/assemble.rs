use std::time::{Duration, Instant};

/// A longer line is cut: nothing FXServer prints is that long, and a runaway resource must not
/// fill the ring with one line.
pub const MAX_LINE: usize = 16 * 1024;
/// A fragment left without its newline this long is shown as it is.
pub const FLUSH_AFTER: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    pub text: String,
    /// The line was longer than [`MAX_LINE`]; the rest of it was dropped.
    pub truncated: bool,
    /// No newline came in time; what follows arrives as a line of its own.
    pub no_eol: bool,
}

/// Cuts one stream's bytes into lines. FXServer on Windows writes a line in many small pieces,
/// and UTF-8 is decoded only once a line is whole, so a character split across two reads stays
/// intact.
#[derive(Debug, Default)]
pub struct LineAssembler {
    pending: Vec<u8>,
    /// When the pending fragment started.
    since: Option<Instant>,
    /// The current line was cut: drop everything up to its newline.
    dropping: bool,
    /// The last line went out without its newline; an empty remainder is not a line.
    continued: bool,
}

impl LineAssembler {
    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Vec<Assembled> {
        let mut lines = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let (piece, newline) = match memchr::memchr(b'\n', rest) {
                Some(end) => (&rest[..end], true),
                None => (rest, false),
            };
            rest = if newline { &rest[piece.len() + 1..] } else { &[] };

            if !self.dropping {
                if self.pending.is_empty() && !piece.is_empty() {
                    self.since = Some(now);
                }
                self.pending.extend_from_slice(piece);
                if self.pending.len() > MAX_LINE {
                    lines.push(self.cut());
                }
            }
            if newline {
                if self.dropping {
                    self.dropping = false;
                } else if let Some(line) = self.complete() {
                    lines.push(line);
                }
            }
        }
        lines
    }

    /// When the pending fragment is due to go out without its newline.
    pub fn deadline(&self) -> Option<Instant> {
        self.since.map(|since| since + FLUSH_AFTER)
    }

    /// The pending fragment, if it has waited [`FLUSH_AFTER`] for its newline.
    pub fn flush(&mut self, now: Instant) -> Option<Assembled> {
        if self.deadline().is_some_and(|deadline| now >= deadline) {
            self.finish()
        } else {
            None
        }
    }

    /// Whatever is left once the stream has ended.
    pub fn finish(&mut self) -> Option<Assembled> {
        if self.pending.is_empty() {
            return None;
        }
        let text = decode(&std::mem::take(&mut self.pending));
        self.since = None;
        self.continued = true;
        Some(Assembled {
            text,
            truncated: false,
            no_eol: true,
        })
    }

    fn complete(&mut self) -> Option<Assembled> {
        let mut line = std::mem::take(&mut self.pending);
        self.since = None;
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let continued = std::mem::replace(&mut self.continued, false);
        if line.is_empty() && continued {
            return None;
        }
        Some(Assembled {
            text: decode(&line),
            truncated: false,
            no_eol: false,
        })
    }

    /// Keeps the first [`MAX_LINE`] bytes, backing off to a character boundary.
    fn cut(&mut self) -> Assembled {
        let mut end = MAX_LINE;
        while end > 0 && self.pending[end] & 0xc0 == 0x80 {
            end -= 1;
        }
        let text = decode(&self.pending[..end]);
        self.pending.clear();
        self.since = None;
        self.dropping = true;
        self.continued = false;
        Assembled {
            text,
            truncated: true,
            no_eol: false,
        }
    }
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Assembled {
        Assembled {
            text: text.to_owned(),
            truncated: false,
            no_eol: false,
        }
    }

    #[test]
    fn lines_split_at_newlines_and_lose_their_carriage_return() {
        let mut lines = LineAssembler::default();

        let out = lines.push(b"first\r\nsecond\nthi", Instant::now());

        assert_eq!(out, [line("first"), line("second")]);
        assert_eq!(lines.push(b"rd\r\n", Instant::now()), [line("third")]);
    }

    #[test]
    fn a_line_written_in_pieces_comes_out_whole() {
        let mut lines = LineAssembler::default();
        let now = Instant::now();
        let mut out = Vec::new();

        for piece in [
            &b"\x1b[38;5;83m[           resources] "[..],
            b"\x1b[0m",
            b"Started",
            b" resource observer",
            b"\r",
            b"\n",
        ] {
            out.extend(lines.push(piece, now));
        }

        assert_eq!(
            out,
            [line(
                "\x1b[38;5;83m[           resources] \x1b[0mStarted resource observer"
            )]
        );
    }

    #[test]
    fn a_character_split_across_reads_is_decoded_once_the_line_is_whole() {
        let mut lines = LineAssembler::default();
        let bytes = "caf\u{e9} \u{1f600}\n".as_bytes();

        let mut out = Vec::new();
        for byte in bytes {
            out.extend(lines.push(std::slice::from_ref(byte), Instant::now()));
        }

        assert_eq!(out, [line("caf\u{e9} \u{1f600}")]);
    }

    #[test]
    fn invalid_utf8_is_replaced() {
        let mut lines = LineAssembler::default();

        assert_eq!(
            lines.push(b"bad \xff byte\n", Instant::now()),
            [line("bad \u{fffd} byte")]
        );
    }

    #[test]
    fn a_line_over_the_limit_is_cut_on_a_character_boundary_and_the_rest_dropped() {
        let mut lines = LineAssembler::default();
        let mut long = vec![b'a'; MAX_LINE - 1];
        long.extend("\u{e9}".as_bytes());
        long.extend([b'z'; 100]);
        let now = Instant::now();

        let mut out = lines.push(&long[..MAX_LINE / 2], now);
        out.extend(lines.push(&long[MAX_LINE / 2..], now));
        out.extend(lines.push(b"tail of the long line\nnext\n", now));

        assert_eq!(out.len(), 2);
        assert!(out[0].truncated);
        assert_eq!(out[0].text, "a".repeat(MAX_LINE - 1));
        assert_eq!(out[1], line("next"));
    }

    #[test]
    fn a_fragment_without_newline_is_flushed_after_a_while() {
        let mut lines = LineAssembler::default();
        let start = Instant::now();

        assert_eq!(lines.push(b"> prompt", start), []);
        assert_eq!(lines.deadline(), Some(start + FLUSH_AFTER));
        assert_eq!(lines.flush(start + FLUSH_AFTER / 2), None);

        let flushed = lines.flush(start + FLUSH_AFTER).unwrap();

        assert_eq!(flushed.text, "> prompt");
        assert!(flushed.no_eol);
        assert_eq!(lines.deadline(), None);
        assert_eq!(
            lines.push(b"\r\n", start + FLUSH_AFTER * 2),
            [],
            "the newline only ends the flushed line"
        );
        assert_eq!(
            lines.push(b"\n", start),
            [line("")],
            "a real empty line still counts"
        );
    }

    #[test]
    fn the_rest_of_a_flushed_line_comes_as_a_line_of_its_own() {
        let mut lines = LineAssembler::default();
        let start = Instant::now();
        lines.push(b"half", start);
        lines.flush(start + FLUSH_AFTER).unwrap();

        assert_eq!(
            lines.push(b" and the rest\n", start + FLUSH_AFTER),
            [line(" and the rest")]
        );
    }

    #[test]
    fn what_is_left_at_the_end_of_the_stream_comes_out() {
        let mut lines = LineAssembler::default();
        lines.push(b"no newline at the end", Instant::now());

        let last = lines.finish().unwrap();

        assert_eq!(last.text, "no newline at the end");
        assert!(last.no_eol);
        assert_eq!(lines.finish(), None);
    }
}
