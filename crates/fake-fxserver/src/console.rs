use std::io::{self, Write};

/// A console channel and the 256-colour code FXServer prints its name in.
pub type Channel<'a> = (&'a str, u8);

pub const CMD: Channel = ("cmd", 171);
pub const RESOURCES: Channel = ("resources", 83);
pub const LICENSE: Channel = ("svadhesive", 66);
pub const SCRIPTING: Channel = ("c-scripting-core", 161);
pub const SERVER: Channel = ("citizen-server-impl", 73);
pub const MAIN: Channel = ("citizen-server-main", 186);
pub const NET: Channel = ("net-tcp-server", 75);
pub const SCRIPT_COLOUR: u8 = 129;

#[cfg(windows)]
pub const EOL: &str = "\r\n";
#[cfg(not(windows))]
pub const EOL: &str = "\n";

/// Prints a line the way FXServer does: a coloured, right-aligned channel name, then the text,
/// written in separate pieces so that readers have to assemble lines themselves.
pub fn line(channel: Channel, text: &str) {
    let (name, colour) = channel;
    let mut out = io::stdout().lock();
    let _ = write!(out, "\x1b[38;5;{colour}m[{name:>20}] \x1b[0m\x1b[0m");
    let _ = out.flush();
    let _ = write!(out, "{text}{EOL}");
    let _ = out.flush();
}

pub fn script(resource: &str, text: &str) {
    line((&format!("script:{resource}"), SCRIPT_COLOUR), &colours(text));
}

pub fn fatal(text: &str) {
    let mut err = io::stderr().lock();
    let _ = write!(err, "Fatal Error: {text}{EOL}");
    let _ = err.flush();
}

/// Turns the `^0`-`^9` codes scripts print into the SGR sequences FXServer writes for them.
fn colours(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let code = match chars.peek() {
            Some(&digit) if c == '^' => match digit {
                '0' => "97",
                '1' => "91",
                '2' => "32",
                '3' => "93",
                '4' => "94",
                '5' => "96",
                '6' => "95",
                '7' => "0",
                '8' => "31",
                '9' => "34",
                _ => "",
            },
            _ => "",
        };
        if code.is_empty() {
            out.push(c);
        } else {
            chars.next();
            out.push_str("\x1b[");
            out.push_str(code);
            out.push('m');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_codes_become_the_sequences_fxserver_prints() {
        assert_eq!(
            colours("^1red^7 plain ^2green^0 reset ^x"),
            "\x1b[91mred\x1b[0m plain \x1b[32mgreen\x1b[97m reset ^x"
        );
    }
}
