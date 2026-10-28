//! FXServer's console grammar (`Console.cpp`): how a buffer is cut into commands and a command into
//! words. Observer reads commands the way FXServer will before it lets one through or warns about
//! a cfg.

use std::ops::Range;

/// Cuts a console buffer into the commands FXServer runs: at `;` outside quotes and at every CR or
/// LF. Quotes toggle without regard to `\`, so after an odd number of `\"` a `;` separates again.
/// Blank commands are left out.
pub fn split(buffer: &str) -> Vec<&str> {
    spans(buffer).into_iter().map(|span| &buffer[span]).collect()
}

/// Where each command of [`split`] lies in the buffer.
pub fn spans(buffer: &str) -> Vec<Range<usize>> {
    let mut commands = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, byte) in buffer.bytes().enumerate() {
        let ends = match byte {
            b'"' => {
                quoted = !quoted;
                false
            }
            b';' => !quoted,
            b'\r' | b'\n' => true,
            _ => false,
        };
        if ends {
            commands.push(start..index);
            start = index + 1;
            // FXServer scans each command afresh, so a quote left open does not carry over a line end.
            quoted = false;
        }
    }
    commands.push(start..buffer.len());
    commands.retain(|command| !words(&buffer[command.clone()]).is_empty());
    commands
}

/// The words of one command. Bytes up to the space separate words, `"` quotes a word in which `\`
/// takes the next character literally, and `//`, `#` and `/* … */` start comments.
///
/// Bytes above 0x7F count as separators too. FXServer compares `char`s, which are signed on its
/// compilers, so it may do the same; reading them as separators errs toward seeing a command
/// where FXServer might not, never the other way.
pub fn words(command: &str) -> Vec<String> {
    let bytes = command.as_bytes();
    let blank = |byte: u8| byte <= b' ' || byte > 0x7f;
    let mut words = Vec::new();
    let mut i = 0;
    loop {
        while i < bytes.len() && blank(bytes[i]) {
            i += 1;
        }
        let rest = &bytes[i..];
        if rest.is_empty() || rest.starts_with(b"//") || rest[0] == b'#' {
            return words;
        }
        if rest.starts_with(b"/*") {
            i = match memchr::memmem::find(&rest[2..], b"*/") {
                Some(end) => i + 2 + end + 2,
                None => return words,
            };
            continue;
        }

        let mut word = Vec::new();
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                word.push(bytes[i]);
                i += 1;
            }
            // Past the closing quote, if there is one.
            i = (i + 1).min(bytes.len());
        } else {
            while i < bytes.len() && !blank(bytes[i]) && bytes[i] != b'"' {
                if bytes[i..].starts_with(b"//") || bytes[i..].starts_with(b"/*") {
                    break;
                }
                word.push(bytes[i]);
                i += 1;
            }
        }
        words.push(String::from_utf8_lossy(&word).into_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_splits_at_semicolons_outside_quotes_and_at_line_ends() {
        assert_eq!(
            split("say \"a; b\"; quit\r\nstatus\nrefresh"),
            ["say \"a; b\"", " quit", "status", "refresh"]
        );
    }

    #[test]
    fn an_escaped_quote_still_toggles_the_quoting_so_a_later_semicolon_separates() {
        assert_eq!(split(r#"say "a \" ; quit"#), [r#"say "a \" "#, " quit"]);
    }

    #[test]
    fn blank_commands_and_comments_are_left_out() {
        assert_eq!(split(" ; ;// nothing\n"), Vec::<&str>::new());
    }

    #[test]
    fn words_are_separated_by_blanks_and_quotes_group_them() {
        assert_eq!(
            words("  set  sv_hostname \"Observer lab\" tail"),
            ["set", "sv_hostname", "Observer lab", "tail"]
        );
    }

    #[test]
    fn a_backslash_inside_quotes_takes_the_next_character() {
        assert_eq!(words(r#"say "a \"b\" c""#), ["say", r#"a "b" c"#]);
    }

    #[test]
    fn comments_end_or_interrupt_the_words() {
        assert_eq!(words("ensure foo // and bar"), ["ensure", "foo"]);
        assert_eq!(words("# start monitor"), Vec::<String>::new());
        assert_eq!(words("start /* monitor */ chat"), ["start", "chat"]);
        assert_eq!(words("start /* never closed"), ["start"]);
    }

    #[test]
    fn bytes_above_ascii_separate_words() {
        assert_eq!(words("\u{e9}quit now"), ["quit", "now"]);
        assert_eq!(words("say \"h\u{e9}llo\""), ["say", "h\u{e9}llo"]);
    }
}
