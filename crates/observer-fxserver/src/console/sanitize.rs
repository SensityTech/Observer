use std::fmt::Write as _;

use vte::{Params, Parser, Perform};

/// Keeps printable characters, tabs and colour sequences (`ESC [ … m`), and drops every other
/// control: cursor moves, screen clears, window titles, OSC links, device strings. What is left is
/// safe to store and to render in the panel.
pub fn sanitize(text: &str) -> String {
    let mut filter = Filter {
        out: String::with_capacity(text.len()),
    };
    Parser::new().advance(&mut filter, text.as_bytes());
    filter.out
}

/// The text without its colour sequences, for the log files. Only works on sanitised text, where
/// every `ESC` starts a sequence that ends in `m`.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('\x1b') {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find('m') {
            Some(end) => &rest[start + end + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

struct Filter {
    out: String,
}

impl Perform for Filter {
    fn print(&mut self, c: char) {
        if !c.is_control() {
            self.out.push(c);
        }
    }

    fn execute(&mut self, byte: u8) {
        if byte == b'\t' {
            self.out.push('\t');
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if action != 'm' || !intermediates.is_empty() || ignore {
            return;
        }
        self.out.push_str("\x1b[");
        for (index, param) in params.iter().enumerate() {
            if index > 0 {
                self.out.push(';');
            }
            for (sub, value) in param.iter().enumerate() {
                if sub > 0 {
                    self.out.push(':');
                }
                let _ = write!(self.out, "{value}");
            }
        }
        self.out.push('m');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_sequences_and_text_pass_through() {
        let line =
            "\x1b[38;5;83m[           resources] \x1b[0m\x1b[0mStarted\tresource \x1b[91mobserver\x1b[0m";

        assert_eq!(sanitize(line), line);
    }

    #[test]
    fn colon_separated_colours_keep_their_form() {
        assert_eq!(sanitize("\x1b[38:2::255:0:0mred"), "\x1b[38:2:0:255:0:0mred");
    }

    #[test]
    fn other_csi_sequences_are_dropped() {
        assert_eq!(
            sanitize("a\x1b[2Jb\x1b[1;1Hc\x1b[?25ld\x1b[>4;1me\x1b[Kf"),
            "abcdef"
        );
    }

    #[test]
    fn osc_titles_and_links_are_dropped_but_the_link_text_stays() {
        assert_eq!(
            sanitize("\x1b]0;window title\x07x\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\y"),
            "xlinky"
        );
    }

    #[test]
    fn device_strings_and_lone_controls_are_dropped() {
        assert_eq!(sanitize("a\x1bP1$r0m\x1b\\b\x07c\x08d\re\x00f\u{9b}g"), "abcdefg");
    }

    #[test]
    fn a_sequence_cut_at_the_end_of_the_line_leaves_nothing_behind() {
        assert_eq!(sanitize("text\x1b[38;5"), "text");
    }

    #[test]
    fn plain_text_has_no_colour_left() {
        assert_eq!(
            plain("\x1b[38;5;83m[           resources] \x1b[0m\x1b[0mStarted \x1b[91mobserver\x1b[0m"),
            "[           resources] Started observer"
        );
    }
}
