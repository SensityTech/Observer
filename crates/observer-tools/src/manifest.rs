//! A resource's manifest, `fxmanifest.lua` or `__resource.lua`, read the way FiveM turns it into
//! entries, without running any Lua. FiveM runs the manifest as a Lua chunk in which every unknown
//! global is a function (`resource_init.lua`): `key 'v'` adds the entry `key = v`, `key {…}` adds
//! each value of the table under the key less one final `s`, and the function it returns takes a
//! second value, stored JSON-encoded under `key_extra`, which is how `data_file 'TYPE' 'path'`
//! pairs a type with a path. Real manifests are lists of such calls, so reading the calls gives
//! what the client will load; anything else (variables, loops, functions, expressions) becomes a
//! note on the manifest, never a failure, and so does anything that makes FiveM refuse it.

use std::fmt::Write as _;

/// The longest manifest read: real ones hold a few kilobytes.
pub const MAX_BYTES: usize = 1024 * 1024;
/// The notes kept for one manifest; the one that says FiveM refuses it is always kept.
const MAX_NOTES: usize = 64;
/// The longest construct a note quotes.
const MAX_QUOTED_CHARS: usize = 60;
/// How deep tables and parentheses nest before Lua refuses the chunk (`LUAI_MAXCCALLS`).
const MAX_DEPTH: usize = 200;

/// What a manifest gives the tool, in the order FiveM reads each list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The first `fx_version`, the one FXServer checks: an `fxmanifest.lua` whose version is not
    /// one it knows (`adamant`, `bodacious`, `cerulean` in the source read) does not start.
    pub fx_version: Option<Entry>,
    /// `game` entries (`games {…}` included): an `fxmanifest.lua` naming neither `gta5` nor
    /// `common`, or both, does not start on a GTA V server.
    pub games: Vec<Entry>,
    /// `file` entries (`files {…}` included), patterns FXServer resolves against the folder.
    pub files: Vec<Entry>,
    pub client_scripts: Vec<Entry>,
    pub shared_scripts: Vec<Entry>,
    pub maps: Vec<Entry>,
    /// The `data_file` pairs the client loads, in manifest order. Empty when the client refuses
    /// them all (see [`NoteKind::DataFileCountMismatch`] and [`NoteKind::ObsoleteDataFiles`]).
    pub data_files: Vec<DataFile>,
    /// The resources started first: every `dependency`, then every `dependencies {…}` entry,
    /// which FiveM stores as `dependencie` and reads after. Entries starting with `/` are
    /// constraints on the server, not resources.
    pub dependencies: Vec<Entry>,
    /// Names this resource answers to as a dependency.
    pub provides: Vec<Entry>,
    /// Any `server_only` entry, whatever its value: such a resource never reaches clients.
    pub server_only: bool,
    /// Any `this_is_a_map` entry: the client reloads its map store for the resource.
    pub this_is_a_map: bool,
    pub notes: Vec<Note>,
}

/// One value of a manifest entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub value: String,
    /// The 1-based line the value is written on.
    pub line: u32,
}

/// A `data_file` pair: a type, such as `DLC_ITYP_REQUEST`, and a path or pattern in the resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFile {
    /// As written; the client compares types ignoring case.
    pub kind: String,
    pub path: String,
    /// The line of the type.
    pub line: u32,
}

/// Something in the manifest the reader did not follow, or that changes what FiveM does with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub kind: NoteKind,
    pub line: u32,
    /// What was found, in a few words.
    pub detail: String,
}

/// What a [`Note`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NoteKind {
    /// The manifest is encrypted by the asset escrow: FXServer decrypts it, nothing here can.
    Encrypted,
    /// Longer than [`MAX_BYTES`]: not read.
    TooLarge,
    /// Lua refuses to load the file (an unfinished string, a bad escape, a byte order mark…):
    /// FiveM does not load the resource.
    SyntaxError,
    /// FiveM stops with an error at this statement: the resource does not load.
    RuntimeError,
    /// A construct the reader does not follow, which may add entries it misses.
    NotFollowed,
    /// A plural key given a single string (`files 'a.meta'`): only a table drops the `s`, and
    /// FiveM never reads the plural key.
    IgnoredPluralKey,
    /// A `data_file` whose path is not a string, which the client skips.
    DataFilePathNotString,
    /// As many `data_file` types as paths are needed: the client stops with an error when the
    /// resource starts, and loads none of its data files.
    DataFileCountMismatch,
    /// A data file of the obsolete `mpassault_vehicles` resource is named: the client ignores
    /// every data file of the resource.
    ObsoleteDataFiles,
}

impl NoteKind {
    /// Whether FiveM refuses the manifest, so that the resource does not load at all.
    pub const fn refuses(self) -> bool {
        matches!(self, Self::SyntaxError | Self::RuntimeError)
    }

    /// A short stable name, for counts and logs.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Encrypted => "encrypted",
            Self::TooLarge => "too_large",
            Self::SyntaxError => "syntax_error",
            Self::RuntimeError => "runtime_error",
            Self::NotFollowed => "not_followed",
            Self::IgnoredPluralKey => "ignored_plural_key",
            Self::DataFilePathNotString => "data_file_path_not_string",
            Self::DataFileCountMismatch => "data_file_count_mismatch",
            Self::ObsoleteDataFiles => "obsolete_data_files",
        }
    }
}

impl Manifest {
    /// Whether FiveM refuses this manifest: the resource does not load, and the entries above are
    /// only what was read before the error, for the report.
    pub fn refused(&self) -> bool {
        self.notes.iter().any(|note| note.kind.refuses())
    }

    /// The patterns of the files FXServer ships to clients, in its order: `file`, `client_script`,
    /// `shared_script`, then `map` entries.
    pub fn shipped_patterns(&self) -> impl Iterator<Item = &Entry> {
        self.files
            .iter()
            .chain(&self.client_scripts)
            .chain(&self.shared_scripts)
            .chain(&self.maps)
    }
}

/// Reads a manifest's text. Bytes that are not UTF-8 are the caller's to replace.
pub fn read(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    // Escrowed manifests start with the header of escrowed drawables: `FXAP`, then a version byte.
    if text.starts_with("FXAP") && text.as_bytes().get(4).is_some_and(|&byte| byte < 0x20) {
        push_note(
            &mut manifest.notes,
            NoteKind::Encrypted,
            1,
            "encrypted by the asset escrow".to_owned(),
        );
        return manifest;
    }
    if text.len() > MAX_BYTES {
        push_note(
            &mut manifest.notes,
            NoteKind::TooLarge,
            1,
            format!("{} bytes", text.len()),
        );
        return manifest;
    }
    // FiveM hands the bytes to `luaL_loadbuffer`, which, unlike Lua's file loader, keeps a byte
    // order mark, where the lexer stops.
    let bom = text.strip_prefix('\u{feff}');
    let (tokens, lex_error) = lex(bom.unwrap_or(text).as_bytes());
    let mut reader = Reader::new(tokens);
    if bom.is_some() {
        reader.note(
            NoteKind::SyntaxError,
            1,
            "starts with a byte order mark, which Lua does not read".to_owned(),
        );
    }
    reader.run();
    if let Some(error) = lex_error {
        reader.note(NoteKind::SyntaxError, error.line, error.message.to_owned());
    } else if let Some(line) = reader.too_deep {
        reader.note(
            NoteKind::SyntaxError,
            line,
            "nested deeper than Lua accepts".to_owned(),
        );
    } else if let Some((line, what)) = reader.unfinished {
        reader.note(
            NoteKind::SyntaxError,
            line,
            format!("{what} left open at the end of the file"),
        );
    }
    reader.finish()
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Name(String),
    Str(Vec<u8>),
    Num(Number),
    Sym(&'static str),
    /// A byte Lua takes for nothing here.
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Number {
    Integer(i64),
    Float(f64),
}

impl Number {
    /// The number as Lua's `tostring` writes it, which is what `AddMetaData` stores.
    fn text(self) -> String {
        match self {
            Self::Integer(value) => value.to_string(),
            Self::Float(value) => lua_float(value),
        }
    }

    fn negated(self) -> Self {
        match self {
            Self::Integer(value) => Self::Integer(value.wrapping_neg()),
            Self::Float(value) => Self::Float(-value),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: Kind,
    line: u32,
}

struct LexError {
    line: u32,
    message: &'static str,
}

const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// The globals of FiveM's manifest state, which are Lua's own and not entry keys. `dofile`,
/// `load` and `loadfile` are removed before the manifest runs, so they are keys like any other.
const BUILTINS: [&str; 30] = [
    "assert",
    "collectgarbage",
    "error",
    "getmetatable",
    "ipairs",
    "next",
    "pairs",
    "pcall",
    "print",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "select",
    "setmetatable",
    "tonumber",
    "tostring",
    "type",
    "warn",
    "xpcall",
    "_G",
    "_VERSION",
    "coroutine",
    "debug",
    "math",
    "string",
    "table",
    "utf8",
    "json",
    "AddMetaData",
];

/// Plural keys whose table form gives a key the tool keeps.
const PLURALS: [&str; 8] = [
    "files",
    "client_scripts",
    "shared_scripts",
    "maps",
    "data_files",
    "dependencies",
    "provides",
    "games",
];

/// Symbols, longest first so that a prefix never takes a longer one's place.
const SYMBOLS: [&str; 33] = [
    "...", "..", "==", "~=", "<=", ">=", "<<", ">>", "//", "::", "+", "-", "*", "/", "%", "^", "#", "&", "~",
    "|", "<", ">", "=", "(", ")", "{", "}", "[", "]", ";", ":", ",", ".",
];

fn is_keyword(name: &str) -> bool {
    KEYWORDS.contains(&name)
}

/// Lua's lexer, from `llex.c`'s rules: comments and blanks skipped, strings decoded, lines counted
/// with `\r\n` and `\n\r` as one break. Stops at the first error Lua would raise.
fn lex(bytes: &[u8]) -> (Vec<Token>, Option<LexError>) {
    let mut lexer = Lexer {
        bytes,
        at: 0,
        line: 1,
    };
    let mut tokens = Vec::new();
    loop {
        match lexer.next() {
            Ok(Some(token)) => tokens.push(token),
            Ok(None) => return (tokens, None),
            Err(message) => {
                return (
                    tokens,
                    Some(LexError {
                        line: lexer.line,
                        message,
                    }),
                );
            }
        }
    }
}

struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
    line: u32,
}

impl Lexer<'_> {
    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.at + offset).copied()
    }

    /// Steps over one line break, `\r\n` and `\n\r` counting as one.
    fn newline(&mut self) {
        let first = self.peek(0);
        self.at += 1;
        if let (Some(first), Some(second)) = (first, self.peek(0))
            && matches!(second, b'\n' | b'\r')
            && second != first
        {
            self.at += 1;
        }
        self.line = self.line.saturating_add(1);
    }

    fn next(&mut self) -> Result<Option<Token>, &'static str> {
        loop {
            let Some(byte) = self.peek(0) else {
                return Ok(None);
            };
            match byte {
                b'\n' | b'\r' => self.newline(),
                b' ' | b'\t' | 0x0B | 0x0C => self.at += 1,
                b'-' if self.peek(1) == Some(b'-') => {
                    self.at += 2;
                    if let Some(level) = self.long_bracket() {
                        self.long_string(level, false)
                            .map_err(|_| "unfinished long comment")?;
                    } else {
                        while self.peek(0).is_some_and(|byte| byte != b'\n' && byte != b'\r') {
                            self.at += 1;
                        }
                    }
                }
                _ => break,
            }
        }
        let line = self.line;
        let Some(byte) = self.peek(0) else {
            return Ok(None);
        };
        let kind = match byte {
            b'\'' | b'"' => Kind::Str(self.short_string(byte)?),
            b'[' => {
                let equals = self.bytes[self.at + 1..]
                    .iter()
                    .take_while(|&&byte| byte == b'=')
                    .count();
                if self.peek(1 + equals) == Some(b'[') {
                    let level = self.long_bracket().ok_or("invalid long string delimiter")?;
                    Kind::Str(self.long_string(level, true)?)
                } else if equals > 0 {
                    return Err("invalid long string delimiter");
                } else {
                    self.at += 1;
                    Kind::Sym("[")
                }
            }
            b'0'..=b'9' => Kind::Num(self.number()?),
            b'.' if self.peek(1).is_some_and(|byte| byte.is_ascii_digit()) => Kind::Num(self.number()?),
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let start = self.at;
                while self
                    .peek(0)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    self.at += 1;
                }
                Kind::Name(String::from_utf8_lossy(&self.bytes[start..self.at]).into_owned())
            }
            _ => {
                let rest = &self.bytes[self.at..];
                if let Some(symbol) = SYMBOLS.iter().find(|symbol| rest.starts_with(symbol.as_bytes())) {
                    self.at += symbol.len();
                    Kind::Sym(symbol)
                } else {
                    self.at += 1;
                    Kind::Other(byte)
                }
            }
        };
        Ok(Some(Token { kind, line }))
    }

    /// At `[`, `[=…=[`: steps over it and gives its level, or leaves `at` alone when it is not one.
    fn long_bracket(&mut self) -> Option<usize> {
        if self.peek(0) != Some(b'[') {
            return None;
        }
        let equals = self.bytes[self.at + 1..]
            .iter()
            .take_while(|&&byte| byte == b'=')
            .count();
        if self.peek(1 + equals) != Some(b'[') {
            return None;
        }
        self.at += equals + 2;
        Some(equals)
    }

    /// The body of a long string or comment, up to `]=…=]` of the same level. A line break right
    /// after the opening bracket is dropped, and every break reads as `\n`.
    fn long_string(&mut self, level: usize, keep: bool) -> Result<Vec<u8>, &'static str> {
        let mut text = Vec::new();
        if matches!(self.peek(0), Some(b'\n' | b'\r')) {
            self.newline();
        }
        loop {
            match self.peek(0) {
                None => return Err("unfinished long string"),
                Some(b']') => {
                    let equals = self.bytes[self.at + 1..]
                        .iter()
                        .take_while(|&&byte| byte == b'=')
                        .count();
                    if equals == level && self.peek(1 + equals) == Some(b']') {
                        self.at += level + 2;
                        return Ok(text);
                    }
                    if keep {
                        text.push(b']');
                    }
                    self.at += 1;
                }
                Some(b'\n' | b'\r') => {
                    self.newline();
                    if keep {
                        text.push(b'\n');
                    }
                }
                Some(byte) => {
                    if keep {
                        text.push(byte);
                    }
                    self.at += 1;
                }
            }
        }
    }

    /// A string between `'` or `"`, with Lua 5.4's escapes.
    fn short_string(&mut self, quote: u8) -> Result<Vec<u8>, &'static str> {
        self.at += 1;
        let mut text = Vec::new();
        loop {
            let Some(byte) = self.peek(0) else {
                return Err("unfinished string");
            };
            match byte {
                b'\n' | b'\r' => return Err("unfinished string"),
                b'\\' => {
                    self.at += 1;
                    self.escape(&mut text)?;
                }
                _ if byte == quote => {
                    self.at += 1;
                    return Ok(text);
                }
                _ => {
                    text.push(byte);
                    self.at += 1;
                }
            }
        }
    }

    fn escape(&mut self, text: &mut Vec<u8>) -> Result<(), &'static str> {
        let Some(byte) = self.peek(0) else {
            return Err("unfinished string");
        };
        let simple = match byte {
            b'a' => Some(0x07),
            b'b' => Some(0x08),
            b'f' => Some(0x0C),
            b'n' => Some(b'\n'),
            b'r' => Some(b'\r'),
            b't' => Some(b'\t'),
            b'v' => Some(0x0B),
            b'\\' | b'"' | b'\'' => Some(byte),
            _ => None,
        };
        if let Some(simple) = simple {
            text.push(simple);
            self.at += 1;
            return Ok(());
        }
        match byte {
            b'\n' | b'\r' => {
                self.newline();
                text.push(b'\n');
            }
            b'x' => {
                let high = self.peek(1).and_then(hex_value);
                let low = self.peek(2).and_then(hex_value);
                let (Some(high), Some(low)) = (high, low) else {
                    return Err("hexadecimal digit expected");
                };
                text.push(high * 16 + low);
                self.at += 3;
            }
            b'z' => {
                self.at += 1;
                while let Some(byte) = self.peek(0) {
                    match byte {
                        b'\n' | b'\r' => self.newline(),
                        b' ' | b'\t' | 0x0B | 0x0C => self.at += 1,
                        _ => break,
                    }
                }
            }
            b'u' => {
                if self.peek(1) != Some(b'{') {
                    return Err("missing '{' in \\u{xxxx}");
                }
                self.at += 2;
                let mut value: u32 = 0;
                let mut digits = 0;
                while let Some(digit) = self.peek(0).and_then(hex_value) {
                    value = value
                        .checked_mul(16)
                        .and_then(|value| value.checked_add(u32::from(digit)))
                        .filter(|&value| value <= 0x7FFF_FFFF)
                        .ok_or("UTF-8 value too large")?;
                    digits += 1;
                    self.at += 1;
                }
                if digits == 0 {
                    return Err("hexadecimal digit expected");
                }
                if self.peek(0) != Some(b'}') {
                    return Err("missing '}' in \\u{xxxx}");
                }
                self.at += 1;
                utf8_escape(value, text);
            }
            b'0'..=b'9' => {
                let mut value: u32 = 0;
                let mut digits = 0;
                while digits < 3
                    && let Some(digit) = self.peek(0).filter(u8::is_ascii_digit)
                {
                    value = value * 10 + u32::from(digit - b'0');
                    digits += 1;
                    self.at += 1;
                }
                text.push(u8::try_from(value).map_err(|_| "decimal escape too large")?);
            }
            _ => return Err("invalid escape sequence"),
        }
        Ok(())
    }

    /// A numeral, read as `llex.c` reads one: digits, `.`, and an exponent with its sign, then
    /// converted as `lua_stringtonumber` would.
    fn number(&mut self) -> Result<Number, &'static str> {
        let start = self.at;
        let hex = self.peek(0) == Some(b'0') && matches!(self.peek(1), Some(b'x' | b'X'));
        let exponent: &[u8] = if hex { b"Pp" } else { b"Ee" };
        if hex {
            self.at += 2;
        }
        loop {
            match self.peek(0) {
                Some(byte) if exponent.contains(&byte) => {
                    self.at += 1;
                    if matches!(self.peek(0), Some(b'+' | b'-')) {
                        self.at += 1;
                    }
                }
                Some(byte) if byte.is_ascii_hexdigit() || byte == b'.' => self.at += 1,
                _ => break,
            }
        }
        if self
            .peek(0)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        {
            return Err("malformed number");
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| "malformed number")?;
        parse_number(text).ok_or("malformed number")
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

/// Lua's `\u{…}`: up to six bytes, for values a `char` cannot hold too.
fn utf8_escape(value: u32, text: &mut Vec<u8>) {
    if value < 0x80 {
        text.push(u8::try_from(value).unwrap_or(b'?'));
        return;
    }
    let mut continuation = Vec::new();
    let mut rest = value;
    // The largest value the first byte still holds, with one more continuation byte each time.
    let mut first_max = 0x3F;
    while rest > first_max {
        continuation.push(0x80 | u8::try_from(rest & 0x3F).unwrap_or(0));
        rest >>= 6;
        first_max >>= 1;
    }
    let mask = u8::try_from((!first_max << 1) & 0xFF).unwrap_or(0);
    text.push(mask | u8::try_from(rest).unwrap_or(0));
    text.extend(continuation.iter().rev());
}

/// `lua_stringtonumber` on a numeral from the lexer: a decimal integer that fits, a hexadecimal
/// integer (wrapping around, as Lua does), or a float.
fn parse_number(text: &str) -> Option<Number> {
    if let Some(digits) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            let value = digits.bytes().fold(0u64, |value, byte| {
                value
                    .wrapping_mul(16)
                    .wrapping_add(u64::from(hex_value(byte).unwrap_or(0)))
            });
            return Some(Number::Integer(i64::from_ne_bytes(value.to_ne_bytes())));
        }
        return hex_float(digits).map(Number::Float);
    }
    if text.bytes().all(|byte| byte.is_ascii_digit())
        && let Ok(value) = text.parse::<i64>()
    {
        return Some(Number::Integer(value));
    }
    text.parse::<f64>()
        .ok()
        .filter(|value| !value.is_nan())
        .map(Number::Float)
}

/// A hexadecimal float, `1.8p3`, its `0x` already gone.
fn hex_float(text: &str) -> Option<f64> {
    let (mantissa, exponent) = match text.find(['p', 'P']) {
        Some(at) => (&text[..at], Some(&text[at + 1..])),
        None => (text, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let mut value = 0f64;
    for byte in whole.bytes().chain(fraction.bytes()) {
        value = value * 16.0 + f64::from(hex_value(byte)?);
    }
    let mut power = -4 * i32::try_from(fraction.len()).ok()?;
    if let Some(exponent) = exponent {
        power = power.checked_add(exponent.parse::<i32>().ok()?)?;
    }
    Some(value * 2f64.powi(power))
}

/// A float as Lua 5.4 writes one: `%.14g`, then `.0` when that reads like an integer.
fn lua_float(value: f64) -> String {
    if value.is_nan() {
        return if value.is_sign_negative() { "-nan" } else { "nan" }.to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    let scientific = format!("{value:.13e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let trim = |text: &str| -> String {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text.to_owned()
        }
    };
    let text = if (-4..14).contains(&exponent) {
        let decimals = usize::try_from(13 - exponent).unwrap_or(0);
        trim(&format!("{value:.decimals$}"))
    } else {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), exponent.unsigned_abs())
    };
    if text.bytes().all(|byte| byte == b'-' || byte.is_ascii_digit()) {
        text + ".0"
    } else {
        text
    }
}

/// A value the reader could work out, as FiveM's loader sees it.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Str(Vec<u8>),
    Num(Number),
    Nil,
    Bool,
    Table(Table),
    /// Anything that needs running Lua: a variable, a call, an operator.
    Unknown,
}

impl Value {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Str(_) => "string",
            Self::Num(_) => "number",
            Self::Nil => "nil",
            Self::Bool => "boolean",
            Self::Table(_) => "table",
            Self::Unknown => "value",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Table {
    /// The values without a key, in order, with their lines: what `ipairs` walks.
    items: Vec<(Value, u32)>,
    /// A `[number] = value` field, which can change what `ipairs` sees.
    indexed: bool,
}

/// The two lists the client pairs: `data_file` types and `data_file_extra` paths.
#[derive(Default)]
struct DataFiles {
    kinds: Vec<Entry>,
    paths: Vec<(Extra, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Extra {
    Path(String),
    /// A value whose JSON is not a string, which the client skips.
    NotString(&'static str),
    /// Not worked out; already noted.
    Unknown,
    /// The path the client takes for the obsolete `mpassault_vehicles` resource.
    Obsolete,
}

impl Extra {
    fn from_path(path: String) -> Self {
        if path == OBSOLETE_DATA_FILE {
            Self::Obsolete
        } else {
            Self::Path(path)
        }
    }
}

/// The path that makes the client ignore every data file of a resource (`ResourcesTest.cpp`).
const OBSOLETE_DATA_FILE: &str = "data/ai/vehicleweapons_caracara.meta";

struct Reader {
    tokens: Vec<Token>,
    pos: usize,
    manifest: Manifest,
    data: DataFiles,
    /// `dependencies {…}` entries, which FiveM stores as `dependencie` and starts after every
    /// `dependency`.
    later_dependencies: Vec<Entry>,
    /// FiveM stopped with an error: nothing after runs.
    stopped: bool,
    /// A table or call still open at the end of the file, and the line it opened on.
    unfinished: Option<(u32, &'static str)>,
    /// Expressions open around the one being read.
    depth: usize,
    /// Where the nesting went past [`MAX_DEPTH`].
    too_deep: Option<u32>,
}

/// One call of a chain: its first value, and its second for the rare `AddMetaData(key, value)`.
struct Call {
    first: (Value, u32),
    second: Option<(Value, u32)>,
}

impl Reader {
    fn new(tokens: Vec<Token>) -> Self {
        Reader {
            tokens,
            pos: 0,
            manifest: Manifest::default(),
            data: DataFiles::default(),
            later_dependencies: Vec::new(),
            stopped: false,
            unfinished: None,
            depth: 0,
            too_deep: None,
        }
    }

    fn run(&mut self) {
        while !self.stopped && self.pos < self.tokens.len() {
            self.statement();
        }
    }

    fn peek(&self) -> Option<&Kind> {
        self.tokens.get(self.pos).map(|token| &token.kind)
    }

    fn peek_at(&self, offset: usize) -> Option<&Kind> {
        self.tokens.get(self.pos + offset).map(|token| &token.kind)
    }

    fn line(&self) -> u32 {
        self.tokens
            .get(self.pos)
            .or_else(|| self.tokens.last())
            .map_or(1, |token| token.line)
    }

    fn eat(&mut self, symbol: &str) -> bool {
        let found = matches!(self.peek(), Some(Kind::Sym(found)) if *found == symbol);
        if found {
            self.pos += 1;
        }
        found
    }

    fn at_argument(&self) -> bool {
        matches!(self.peek(), Some(Kind::Str(_) | Kind::Sym("{" | "(")))
    }

    fn note(&mut self, kind: NoteKind, line: u32, detail: String) {
        push_note(&mut self.manifest.notes, kind, line, detail);
    }

    /// Notes an error FiveM stops at; nothing after it runs.
    fn stop(&mut self, line: u32, detail: String) {
        self.note(NoteKind::RuntimeError, line, detail);
        self.stopped = true;
    }

    fn open_at_end(&mut self, line: u32, what: &'static str) {
        self.unfinished.get_or_insert((line, what));
    }

    fn statement(&mut self) {
        let line = self.line();
        let Some(kind) = self.peek().cloned() else {
            return;
        };
        match kind {
            Kind::Sym(";") => self.pos += 1,
            Kind::Name(name)
                if !is_keyword(&name)
                    && matches!(self.peek_at(1), Some(Kind::Str(_) | Kind::Sym("{" | "("))) =>
            {
                self.pos += 1;
                self.call(&name, line);
            }
            _ => {
                let quoted = self.skip_statement();
                self.note(NoteKind::NotFollowed, line, quoted);
            }
        }
    }

    /// A call chain, `head a b …`, its head already read.
    fn call(&mut self, head: &str, line: u32) {
        let mut calls = Vec::new();
        while self.at_argument() && self.unfinished.is_none() {
            calls.push(self.arguments());
        }
        let continues = matches!(self.peek(), Some(Kind::Sym("." | ":" | "[" | "=" | ",")));
        self.apply(head, line, calls);
        if continues && !self.stopped {
            let quoted = self.skip_statement();
            self.note(
                NoteKind::NotFollowed,
                line,
                format!("`{head}` call followed by {quoted}"),
            );
        }
    }

    /// The values one call passes: a string, a table, or a parenthesised list.
    fn arguments(&mut self) -> Call {
        let line = self.line();
        match self.peek().cloned() {
            Some(Kind::Str(bytes)) => {
                self.pos += 1;
                Call {
                    first: (Value::Str(bytes), line),
                    second: None,
                }
            }
            Some(Kind::Sym("{")) => {
                self.pos += 1;
                Call {
                    first: (self.table(line), line),
                    second: None,
                }
            }
            _ => {
                self.pos += 1;
                if self.eat(")") {
                    return Call {
                        first: (Value::Nil, line),
                        second: None,
                    };
                }
                let first = self.expression();
                let second = self.eat(",").then(|| self.expression());
                while self.eat(",") {
                    let _ = self.expression();
                }
                if self.eat(")") {
                    Call { first, second }
                } else {
                    self.skip_to_closer(")", line, "call");
                    Call {
                        first: (Value::Unknown, line),
                        second: None,
                    }
                }
            }
        }
    }

    /// One expression, as far as it ends at `,`, `;` or a closing bracket. Literals and tables of
    /// literals are worked out; anything else is skipped and [`Value::Unknown`].
    fn expression(&mut self) -> (Value, u32) {
        let line = self.line();
        if self.depth >= MAX_DEPTH {
            self.too_deep.get_or_insert(line);
            self.pos = self.tokens.len();
            return (Value::Unknown, line);
        }
        self.depth += 1;
        let value = self.primary(line);
        self.depth -= 1;
        if matches!(self.peek(), None | Some(Kind::Sym("," | ";" | ")" | "}" | "]"))) {
            (value, line)
        } else {
            self.skip_expression();
            (Value::Unknown, line)
        }
    }

    /// The value an expression starts with.
    fn primary(&mut self, line: u32) -> Value {
        match self.peek().cloned() {
            Some(Kind::Str(bytes)) => {
                self.pos += 1;
                Value::Str(bytes)
            }
            Some(Kind::Num(number)) => {
                self.pos += 1;
                Value::Num(number)
            }
            Some(Kind::Name(name)) if name == "nil" => {
                self.pos += 1;
                Value::Nil
            }
            Some(Kind::Name(name)) if name == "true" || name == "false" => {
                self.pos += 1;
                Value::Bool
            }
            Some(Kind::Sym("{")) => {
                self.pos += 1;
                self.table(line)
            }
            Some(Kind::Sym("-")) => match self.peek_at(1).cloned() {
                Some(Kind::Num(number)) => {
                    self.pos += 2;
                    Value::Num(number.negated())
                }
                _ => Value::Unknown,
            },
            Some(Kind::Sym("(")) => {
                self.pos += 1;
                let (inner, _) = self.expression();
                if self.eat(")") {
                    inner
                } else {
                    self.skip_to_closer(")", line, "parenthesis");
                    Value::Unknown
                }
            }
            _ => Value::Unknown,
        }
    }

    /// A table constructor, its `{` already read.
    fn table(&mut self, line: u32) -> Value {
        let mut table = Table::default();
        loop {
            match self.peek() {
                None => {
                    self.open_at_end(line, "table");
                    return Value::Unknown;
                }
                Some(Kind::Sym("}")) => {
                    self.pos += 1;
                    return Value::Table(table);
                }
                Some(Kind::Sym("[")) => {
                    self.pos += 1;
                    let (key, _) = self.expression();
                    if !(self.eat("]") && self.eat("=")) {
                        self.skip_to_closer("}", line, "table");
                        return Value::Unknown;
                    }
                    let _ = self.expression();
                    if !matches!(key, Value::Str(_)) {
                        table.indexed = true;
                    }
                }
                Some(Kind::Name(name))
                    if !is_keyword(name) && matches!(self.peek_at(1), Some(Kind::Sym("="))) =>
                {
                    self.pos += 2;
                    let _ = self.expression();
                }
                Some(_) => {
                    let item = self.expression();
                    table.items.push(item);
                }
            }
            match self.peek() {
                Some(Kind::Sym("," | ";")) => self.pos += 1,
                Some(Kind::Sym("}")) => {}
                None => {
                    self.open_at_end(line, "table");
                    return Value::Unknown;
                }
                Some(_) => {
                    self.skip_to_closer("}", line, "table");
                    return Value::Unknown;
                }
            }
        }
    }

    /// Skips the rest of an expression, up to `,`, `;` or a closing bracket at its own depth.
    fn skip_expression(&mut self) {
        let mut depth = 0usize;
        while let Some(kind) = self.peek() {
            if depth == 0 && matches!(kind, Kind::Sym("," | ";" | ")" | "}" | "]")) {
                return;
            }
            if opens(kind) {
                depth += 1;
            } else if closes(kind) {
                depth = depth.saturating_sub(1);
            }
            self.pos += 1;
        }
    }

    /// Skips up to and past the `closer` that closes what is open, nested brackets included.
    fn skip_to_closer(&mut self, closer: &str, line: u32, what: &'static str) {
        let mut depth = 0usize;
        while let Some(kind) = self.peek() {
            if depth == 0 && matches!(kind, Kind::Sym(found) if *found == closer) {
                self.pos += 1;
                return;
            }
            if matches!(kind, Kind::Sym("(" | "{" | "[")) {
                depth += 1;
            } else if matches!(kind, Kind::Sym(")" | "}" | "]")) {
                depth = depth.saturating_sub(1);
            }
            self.pos += 1;
        }
        self.open_at_end(line, what);
    }

    /// Skips a statement the reader does not follow and quotes it. It ends where a new statement
    /// surely starts, outside any bracket or block: at a `;`, or at a name or a statement keyword
    /// after a complete expression.
    fn skip_statement(&mut self) -> String {
        let start = self.pos;
        let mut depth = 0usize;
        let mut complete = false;
        while let Some(kind) = self.peek() {
            if self.pos > start && depth == 0 {
                if matches!(kind, Kind::Sym(";")) {
                    self.pos += 1;
                    break;
                }
                if complete && starts_statement(kind) {
                    break;
                }
            }
            if opens(kind) {
                depth += 1;
            } else if closes(kind) {
                if depth == 0 {
                    // A closer with nothing open is a Lua syntax error: stop past it.
                    self.pos += 1;
                    break;
                }
                depth -= 1;
            }
            complete = ends_expression(kind);
            self.pos += 1;
        }
        quote(&self.tokens[start..self.pos.min(self.tokens.len())])
    }

    /// Runs one call chain the way FiveM's loader does.
    fn apply(&mut self, head: &str, line: u32, calls: Vec<Call>) {
        if head == "AddMetaData" {
            self.add_directly(line, calls);
            return;
        }
        if BUILTINS.contains(&head) {
            self.note(NoteKind::NotFollowed, line, format!("calls Lua's `{head}`"));
            return;
        }
        let mut calls = calls.into_iter();
        let (first, first_line) = calls.next().map_or((Value::Nil, line), |call| call.first);
        let key = match first {
            Value::Table(table) => {
                let key = head.strip_suffix('s').unwrap_or(head);
                for (item, item_line) in table.items {
                    let value = match item {
                        // `ipairs` ends at the first nil.
                        Value::Nil => break,
                        Value::Str(bytes) => lua_string(&bytes),
                        Value::Num(number) => number.text(),
                        Value::Bool | Value::Table(_) => {
                            let what = item.type_name();
                            self.stop(
                                item_line,
                                format!("a {what} in `{head} {{…}}`, where FiveM wants strings"),
                            );
                            return;
                        }
                        Value::Unknown => {
                            let detail = format!("a value of `{head} {{…}}` that needs running Lua");
                            self.note(NoteKind::NotFollowed, item_line, detail);
                            continue;
                        }
                    };
                    self.add(key, value, item_line);
                }
                if table.indexed {
                    self.note(
                        NoteKind::NotFollowed,
                        first_line,
                        format!("`{head} {{…}}` with numbered fields"),
                    );
                }
                key
            }
            Value::Str(bytes) => {
                self.add_one(head, lua_string(&bytes), first_line);
                head
            }
            Value::Num(number) => {
                self.add_one(head, number.text(), first_line);
                head
            }
            Value::Nil | Value::Bool => {
                let what = first.type_name();
                self.stop(
                    first_line,
                    format!("`{head}` given a {what}, where FiveM wants a string"),
                );
                return;
            }
            Value::Unknown => {
                self.note(
                    NoteKind::NotFollowed,
                    first_line,
                    format!("`{head}` given a value that needs running Lua"),
                );
                return;
            }
        };
        if let Some(Call {
            first: (second, second_line),
            ..
        }) = calls.next()
            && key == "data_file"
        {
            let extra = match second {
                Value::Str(bytes) => Extra::from_path(lua_string(&bytes)),
                Value::Unknown => {
                    let detail = "a `data_file` path that needs running Lua".to_owned();
                    self.note(NoteKind::NotFollowed, second_line, detail);
                    Extra::Unknown
                }
                other => Extra::NotString(other.type_name()),
            };
            self.data.paths.push((extra, second_line));
        }
        if calls.next().is_some() {
            self.stop(
                line,
                format!("a third value after `{head}`: the pair's function returns nothing to call"),
            );
        }
    }

    /// `key 'value'`: the key as written, which FiveM never reads for a plural.
    fn add_one(&mut self, head: &str, value: String, line: u32) {
        if let Some(singular) = PLURALS.contains(&head).then(|| head.strip_suffix('s')).flatten() {
            let detail = format!(
                "`{head}` given one string, which FiveM does not read: write `{singular}` or a table"
            );
            self.note(NoteKind::IgnoredPluralKey, line, detail);
        }
        self.add(head, value, line);
    }

    /// `AddMetaData(key, value)` called by the manifest itself: stored as is, no table, no pair.
    fn add_directly(&mut self, line: u32, calls: Vec<Call>) {
        let mut calls = calls.into_iter();
        let Some(Call {
            first: (key, _),
            second,
        }) = calls.next()
        else {
            return;
        };
        let (value, value_line) = second.unwrap_or((Value::Nil, line));
        let text = |value: Value| match value {
            Value::Str(bytes) => Ok(lua_string(&bytes)),
            Value::Num(number) => Ok(number.text()),
            other => Err(other),
        };
        match (text(key), text(value)) {
            (Ok(key), Ok(value)) => {
                if !key.eq_ignore_ascii_case("is_cfxv2") {
                    self.add(&key, value, value_line);
                }
            }
            (Err(Value::Unknown), _) | (_, Err(Value::Unknown)) => {
                let detail = "`AddMetaData` given a value that needs running Lua".to_owned();
                self.note(NoteKind::NotFollowed, line, detail);
                return;
            }
            (Err(other), _) | (_, Err(other)) => {
                let what = other.type_name();
                self.stop(
                    line,
                    format!("`AddMetaData` given a {what}, where it wants a string"),
                );
                return;
            }
        }
        if calls.next().is_some() {
            self.stop(
                line,
                "a call on what `AddMetaData` returns, which is nothing".to_owned(),
            );
        }
    }

    /// One entry, as `AddMetaData` stores it.
    fn add(&mut self, key: &str, value: String, line: u32) {
        let entry = Entry { value, line };
        let manifest = &mut self.manifest;
        match key {
            "file" => manifest.files.push(entry),
            "client_script" => manifest.client_scripts.push(entry),
            "shared_script" => manifest.shared_scripts.push(entry),
            "map" => manifest.maps.push(entry),
            "data_file" => self.data.kinds.push(entry),
            "data_file_extra" => {
                let extra = match json_string(&entry.value) {
                    Some(path) => Extra::from_path(path),
                    None => Extra::NotString("JSON value"),
                };
                self.data.paths.push((extra, line));
            }
            "dependency" => manifest.dependencies.push(entry),
            "dependencie" => self.later_dependencies.push(entry),
            "provide" => manifest.provides.push(entry),
            "server_only" => manifest.server_only = true,
            "fx_version" => {
                manifest.fx_version.get_or_insert(entry);
            }
            "game" => manifest.games.push(entry),
            "this_is_a_map" => manifest.this_is_a_map = true,
            _ => {}
        }
    }

    /// Pairs the data files as the client does, and closes the manifest.
    fn finish(mut self) -> Manifest {
        let later = std::mem::take(&mut self.later_dependencies);
        self.manifest.dependencies.extend(later);
        let DataFiles { kinds, paths } = std::mem::take(&mut self.data);
        if let Some((_, line)) = paths.iter().find(|(extra, _)| *extra == Extra::Obsolete) {
            let detail = format!("`{OBSOLETE_DATA_FILE}` named: the client skips every data file");
            self.note(NoteKind::ObsoleteDataFiles, *line, detail);
        } else if kinds.len() != paths.len() {
            let line = kinds.first().map_or(1, |kind| kind.line);
            let detail = format!("{} data_file types for {} paths", kinds.len(), paths.len());
            self.note(NoteKind::DataFileCountMismatch, line, detail);
        } else {
            for (kind, (extra, path_line)) in kinds.into_iter().zip(paths) {
                match extra {
                    Extra::Path(path) => {
                        self.manifest.data_files.push(DataFile {
                            kind: kind.value,
                            path,
                            line: kind.line,
                        });
                    }
                    Extra::NotString(what) => {
                        let detail = format!("`{}` given a {what} as its path", kind.value);
                        self.note(NoteKind::DataFilePathNotString, path_line, detail);
                    }
                    Extra::Unknown | Extra::Obsolete => {}
                }
            }
        }
        self.manifest
    }
}

/// Tokens that open a bracket or a block, which [`closes`] closes.
fn opens(kind: &Kind) -> bool {
    match kind {
        Kind::Sym(symbol) => matches!(*symbol, "(" | "{" | "["),
        Kind::Name(name) => matches!(name.as_str(), "function" | "if" | "do" | "repeat"),
        _ => false,
    }
}

fn closes(kind: &Kind) -> bool {
    match kind {
        Kind::Sym(symbol) => matches!(*symbol, ")" | "}" | "]"),
        Kind::Name(name) => matches!(name.as_str(), "end" | "until"),
        _ => false,
    }
}

/// Whether a statement starts with this token when an expression just ended: a name, or a
/// keyword that only starts statements (`do` is left out: it ends `for` and `while` headers).
fn starts_statement(kind: &Kind) -> bool {
    match kind {
        Kind::Name(name) => {
            !is_keyword(name)
                || matches!(
                    name.as_str(),
                    "local" | "function" | "if" | "for" | "while" | "repeat" | "return" | "goto" | "break"
                )
        }
        _ => false,
    }
}

/// Whether an expression may end with this token.
fn ends_expression(kind: &Kind) -> bool {
    match kind {
        Kind::Str(_) | Kind::Num(_) => true,
        Kind::Sym(symbol) => matches!(*symbol, ")" | "}" | "]" | "..."),
        Kind::Name(name) => !is_keyword(name) || matches!(name.as_str(), "nil" | "true" | "false" | "end"),
        Kind::Other(_) => false,
    }
}

/// A few tokens, written back for a note.
fn quote(tokens: &[Token]) -> String {
    let mut text = String::new();
    for token in tokens {
        if text.len() > MAX_QUOTED_CHARS * 4 {
            break;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        match &token.kind {
            Kind::Name(name) => text.push_str(name),
            Kind::Str(bytes) => {
                text.push('\'');
                text.push_str(&lua_string(bytes));
                text.push('\'');
            }
            Kind::Num(number) => text.push_str(&number.text()),
            Kind::Sym(symbol) => text.push_str(symbol),
            Kind::Other(byte) => {
                let _ = write!(text, "<{byte:02x}>");
            }
        }
    }
    let mut quoted: String = text.chars().take(MAX_QUOTED_CHARS).collect();
    if quoted.len() < text.len() {
        quoted.push('…');
    }
    format!("`{quoted}`")
}

/// A Lua string's bytes as text; bytes that are not UTF-8 become U+FFFD.
fn lua_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The string a JSON document holds, when it is one string: what the client takes from a
/// `data_file_extra` written by hand.
fn json_string(text: &str) -> Option<String> {
    let text = text.trim_matches([' ', '\t', '\n', '\r']);
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return None,
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let first = hex_unit(&mut chars)?;
                    let code = if (0xD800..0xDC00).contains(&first) {
                        if chars.next()? != '\\' || chars.next()? != 'u' {
                            return None;
                        }
                        let second = hex_unit(&mut chars)?;
                        if !(0xDC00..0xE000).contains(&second) {
                            return None;
                        }
                        0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                    } else {
                        first
                    };
                    out.push(char::from_u32(code)?);
                }
                _ => return None,
            },
            c if u32::from(c) < 0x20 => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

/// The four hexadecimal digits of a JSON `\u` escape.
fn hex_unit(chars: &mut std::str::Chars) -> Option<u32> {
    let mut value = 0;
    for _ in 0..4 {
        value = value * 16 + chars.next()?.to_digit(16)?;
    }
    Some(value)
}

/// Keeps a note; past [`MAX_NOTES`], only the kinds a manifest has at most once or twice.
fn push_note(notes: &mut Vec<Note>, kind: NoteKind, line: u32, detail: String) {
    let repeats = matches!(
        kind,
        NoteKind::NotFollowed | NoteKind::IgnoredPluralKey | NoteKind::DataFilePathNotString
    );
    if notes.len() < MAX_NOTES || !repeats {
        notes.push(Note { kind, line, detail });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    fn values(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.value.as_str()).collect()
    }

    fn pairs(manifest: &Manifest) -> Vec<(&str, &str)> {
        manifest
            .data_files
            .iter()
            .map(|data_file| (data_file.kind.as_str(), data_file.path.as_str()))
            .collect()
    }

    fn kinds(manifest: &Manifest) -> Vec<NoteKind> {
        manifest.notes.iter().map(|note| note.kind).collect()
    }

    #[test]
    fn a_typical_manifest_gives_its_files_scripts_and_data_files() {
        let manifest = read(concat!(
            "server_script \"@other/check.lua\"\n",
            "fx_version 'cerulean'\n",
            "games { 'gta5' };\n",
            "lua54 'yes'\n",
            "this_is_a_map 'yes'\n",
            "files{\n",
            "    \"audio/**.*\",\n",
            "\t'data/carcols.meta',\n",
            "    'water/*',\n",
            "\n",
            "}\n",
            "escrow_ignore { 'stream/**' }\n",
            "data_file 'AUDIO_WAVEPACK' 'audio'\n",
            "data_file \"VEHICLE_METADATA_FILE\" \"data/**/vehicles.meta\"\n",
            "data_file('DLC_ITYP_REQUEST')('props.ytyp')\n",
            "client_scripts {\n    'client.lua'\n}\n",
            "shared_script 'config.lua'\n",
            "server_scripts{ 'server.lua' }\n",
            "dependency '/assetpacks'\n",
        ));
        assert_eq!(
            manifest.fx_version.as_ref().map(|entry| entry.value.as_str()),
            Some("cerulean")
        );
        assert_eq!(values(&manifest.games), ["gta5"]);
        assert!(manifest.this_is_a_map);
        assert!(!manifest.server_only);
        assert_eq!(
            values(&manifest.files),
            ["audio/**.*", "data/carcols.meta", "water/*"]
        );
        assert_eq!(values(&manifest.client_scripts), ["client.lua"]);
        assert_eq!(values(&manifest.shared_scripts), ["config.lua"]);
        assert_eq!(values(&manifest.dependencies), ["/assetpacks"]);
        assert_eq!(
            pairs(&manifest),
            [
                ("AUDIO_WAVEPACK", "audio"),
                ("VEHICLE_METADATA_FILE", "data/**/vehicles.meta"),
                ("DLC_ITYP_REQUEST", "props.ytyp"),
            ]
        );
        assert_eq!(manifest.data_files[2].line, 15);
        assert_eq!(manifest.files[1].line, 8);
        assert_eq!(manifest.notes, Vec::new(), "{:?}", manifest.notes);
        assert_eq!(manifest.shipped_patterns().count(), 5);
    }

    #[test]
    fn comments_of_every_form_hide_what_they_hold() {
        let manifest = read(concat!(
            "-- data_file 'A' 'a'\n",
            "--[[ data_file 'B' 'b'\n data_file 'C' 'c' ]]\n",
            "--[==[ ]] data_file 'D' 'd' ]==]\n",
            "--[= not a long comment: data_file 'E' 'e'\n",
            "data_file 'F' 'f' -- data_file 'G' 'g'\n",
            "---[[ a line comment\n",
            "data_file 'H' 'h'\n",
        ));
        assert_eq!(pairs(&manifest), [("F", "f"), ("H", "h")]);
        assert_eq!(manifest.data_files[1].line, 8);
    }

    #[test]
    fn strings_of_every_form_are_decoded_as_lua_does() {
        let manifest = read(concat!(
            "file 'a\\tb\\\\c\\'d\\\"e'\n",
            "file \"\\x41\\66\\u{48}\\u{e9}\"\n",
            "file 'one\\z\n      two'\n",
            "file [[\nlong\nstring]]\n",
            "file [==[with ]] inside]==]\n",
            "file 'line\\\nbreak'\n",
        ));
        assert_eq!(
            values(&manifest.files),
            [
                "a\tb\\c'd\"e",
                "ABH\u{e9}",
                "onetwo",
                "long\nstring",
                "with ]] inside",
                "line\nbreak"
            ]
        );
        assert_eq!(manifest.files[3].line, 5);
        assert_eq!(manifest.files[4].line, 8);
    }

    #[test]
    fn a_table_drops_one_final_s_from_any_key_and_a_string_keeps_it() {
        let manifest = read(concat!(
            "files { 'a.meta' }\n",
            "file { 'b.meta' }\n",
            "files 'c.meta'\n",
            "dependencies { 'later' }\n",
            "dependency 'first'\n",
            "dependencies 'never'\n",
            "provides { 'other' }\n",
            "data_files { 'T' } 'p'\n",
        ));
        assert_eq!(values(&manifest.files), ["a.meta", "b.meta"]);
        assert_eq!(values(&manifest.dependencies), ["first", "later"]);
        assert_eq!(values(&manifest.provides), ["other"]);
        assert_eq!(pairs(&manifest), [("T", "p")]);
        assert_eq!(
            kinds(&manifest),
            [NoteKind::IgnoredPluralKey, NoteKind::IgnoredPluralKey]
        );
        assert_eq!(manifest.notes[0].line, 3);
    }

    #[test]
    fn a_chain_continues_on_the_next_line_as_in_lua() {
        let manifest = read("data_file 'DLC_ITYP_REQUEST'\n    'stream/a.ytyp'\ndata_file\n('A')\n('b')\n");
        assert_eq!(
            pairs(&manifest),
            [("DLC_ITYP_REQUEST", "stream/a.ytyp"), ("A", "b")]
        );
    }

    #[test]
    fn data_file_types_and_paths_pair_by_position_as_the_client_pairs_them() {
        let lone = read("data_file 'A'\ndata_file 'B' 'b'\n");
        assert_eq!(lone.data_files, Vec::new());
        assert_eq!(kinds(&lone), [NoteKind::DataFileCountMismatch]);

        let table = read("data_file { 'A', 'B' } 'a'\n");
        assert_eq!(table.data_files, Vec::new());
        assert_eq!(kinds(&table), [NoteKind::DataFileCountMismatch]);
    }

    #[test]
    fn a_path_that_is_not_a_string_is_skipped_but_still_paired() {
        let manifest =
            read("data_file 'A' { 'x' }\ndata_file 'B' 'b'\ndata_file('C')(1)\ndata_file('D')()\n");
        assert_eq!(pairs(&manifest), [("B", "b")]);
        assert_eq!(kinds(&manifest), [NoteKind::DataFilePathNotString; 3]);
        assert!(!manifest.refused());
    }

    #[test]
    fn a_third_value_after_a_pair_makes_fivem_refuse_the_manifest() {
        let manifest = read("file 'kept'\ndata_file 'A' 'a' 'extra'\nfile 'never'\n");
        assert!(manifest.refused());
        assert_eq!(kinds(&manifest), [NoteKind::RuntimeError]);
        assert_eq!(manifest.notes[0].line, 2);
        assert_eq!(values(&manifest.files), ["kept"]);
    }

    #[test]
    fn a_value_fivem_cannot_store_makes_it_refuse_the_manifest() {
        for text in [
            "file(nil)",
            "file()",
            "client_script(true)",
            "files { { 'nested' } }",
            "files { 'a', false }",
        ] {
            let manifest = read(text);
            assert!(manifest.refused(), "{text}");
            assert_eq!(kinds(&manifest), [NoteKind::RuntimeError], "{text}");
        }
    }

    #[test]
    fn ipairs_stops_at_the_first_nil_and_named_fields_are_left_out() {
        let manifest = read("files { 'a', name = 'skipped', ['key'] = 'skipped', 'b'; nil, 'c' }\n");
        assert_eq!(values(&manifest.files), ["a", "b"]);
        assert_eq!(manifest.notes, Vec::new());
        let indexed = read("files { [1] = 'a' }\n");
        assert_eq!(kinds(&indexed), [NoteKind::NotFollowed]);
    }

    #[test]
    fn numbers_are_stored_as_lua_writes_them() {
        let manifest =
            read("files { 1, 1.5, 2.0, 0x10, -3, 1e15, 1e100, 0.1, 0x1p4, 9223372036854775808 }\n");
        assert_eq!(
            values(&manifest.files),
            [
                "1",
                "1.5",
                "2.0",
                "16",
                "-3",
                "1e+15",
                "1e+100",
                "0.1",
                "16.0",
                "9.2233720368548e+18"
            ]
        );
    }

    #[test]
    fn constructs_the_reader_cannot_follow_are_notes_and_reading_goes_on() {
        let manifest = read(concat!(
            "local prefix = 'stream/'\n",
            "for _, name in ipairs({ 'a', 'b' }) do\n",
            "    file(prefix .. name)\n",
            "end\n",
            "file(prefix)\n",
            "if true then data_file 'A' 'a' end\n",
            "file('a' .. 'b')\n",
            "print('hello')\n",
            "x.y 'z'\n",
            "file 'after'\n",
        ));
        assert_eq!(values(&manifest.files), ["after"]);
        assert_eq!(manifest.data_files, Vec::new());
        let lines: Vec<u32> = manifest.notes.iter().map(|note| note.line).collect();
        assert_eq!(lines, [1, 2, 5, 6, 7, 8, 9]);
        assert!(
            manifest
                .notes
                .iter()
                .all(|note| note.kind == NoteKind::NotFollowed)
        );
        assert!(!manifest.refused());
    }

    #[test]
    fn what_lua_cannot_load_makes_fivem_refuse_the_manifest() {
        for (text, line) in [
            ("file 'a'\nfile 'unfinished\n", 2),
            ("file 'bad \\q escape'", 1),
            ("--[[ never closed\nfile 'a'", 2),
            ("file [=x", 1),
            ("files {\n 'a',\n", 1),
            ("file(\n'a'", 1),
            ("\u{feff}file 'a'", 1),
            ("file 3x", 1),
            ("file '\\300'", 1),
        ] {
            let manifest = read(text);
            assert!(manifest.refused(), "{text:?}");
            assert!(
                manifest
                    .notes
                    .iter()
                    .any(|note| note.kind == NoteKind::SyntaxError && note.line == line),
                "{text:?}: {:?}",
                manifest.notes
            );
        }
    }

    #[test]
    fn an_escrowed_manifest_is_noted_and_left_unread() {
        let manifest = read("FXAP\u{1}\0\0\0\0\0garbage");
        assert_eq!(kinds(&manifest), [NoteKind::Encrypted]);
        assert!(!manifest.refused());
        assert_eq!(manifest.files, Vec::new());
    }

    #[test]
    fn the_obsolete_assault_vehicles_path_drops_every_data_file() {
        let manifest = read("data_file 'A' 'a'\ndata_file 'B' 'data/ai/vehicleweapons_caracara.meta'\n");
        assert_eq!(manifest.data_files, Vec::new());
        assert_eq!(kinds(&manifest), [NoteKind::ObsoleteDataFiles]);
    }

    #[test]
    fn server_only_and_maps_count_whatever_their_value_and_the_first_version_wins() {
        let manifest = read("server_only 'no'\nfx_version 'adamant'\nfx_version 'cerulean'\nmap 'map.lua'\n");
        assert!(manifest.server_only);
        assert_eq!(
            manifest.fx_version.map(|entry| entry.value),
            Some("adamant".to_owned())
        );
        assert_eq!(values(&manifest.maps), ["map.lua"]);
    }

    #[test]
    fn line_breaks_of_every_kind_count_once() {
        let manifest = read("file 'a'\r\nfile 'b'\n\rfile 'c'\rfile [[x\r\ny]]\nfile 'd'\n");
        let lines: Vec<u32> = manifest.files.iter().map(|entry| entry.line).collect();
        assert_eq!(lines, [1, 2, 3, 4, 6]);
        assert_eq!(manifest.files[3].value, "x\ny");
    }

    #[test]
    fn add_meta_data_called_directly_stores_the_entry_as_is() {
        let manifest = read(
            "AddMetaData('files', 'kept as files')\nAddMetaData('file', 'a.meta')\nAddMetaData('is_cfxv2', 'x')\n",
        );
        assert_eq!(values(&manifest.files), ["a.meta"]);
        assert!(read("AddMetaData('file')").refused());
    }

    #[test]
    fn a_data_file_extra_written_by_hand_is_read_as_json() {
        // The Lua string `"a\/b"` is JSON for `a/b`.
        let manifest =
            read("data_file 'A'\ndata_file_extra '\"a\\\\/b\"'\ndata_file 'B'\ndata_file_extra 'not json'\n");
        assert_eq!(pairs(&manifest), [("A", "a/b")]);
        assert_eq!(kinds(&manifest), [NoteKind::DataFilePathNotString]);
        let unit = |hex: &str| format!("\\u{hex}");
        let pair = format!(" \"{}{}{}\" ", unit("00e9"), unit("d83d"), unit("de00"));
        assert_eq!(json_string(&pair).as_deref(), Some("\u{e9}\u{1f600}"));
        assert_eq!(json_string(&format!("\"{}\"", unit("d83d"))), None);
    }

    #[test]
    fn hostile_input_ends_without_a_panic() {
        let sample = concat!(
            "fx_version 'cerulean'\nfiles { 'a', [[b]], \"c\\x41\" }\n",
            "data_file('A')('b') --[==[ c ]==]\nlocal x = { function() end }\n",
            "data_file 'X' '\\u{7FFFFFFF}' ; ; ) } ] end until\n"
        );
        for end in 0..=sample.len() {
            if sample.is_char_boundary(end) {
                let _ = read(&sample[..end]);
            }
        }
        let _ = read(&"{".repeat(100_000));
        let _ = read(&format!("files {}", "{".repeat(100_000)));
        let deep = read(&format!("files {}'a'{}", "(".repeat(500), ")".repeat(500)));
        assert!(deep.refused());
        let _ = read(&"( ".repeat(50_000));
        let _ = read("file '\\u{80000000}'");
        let _ = read("file 0x");
        let _ = read("file '\\");
        let _ = read(&"x = 1\n".repeat(1_000));
        assert_eq!(read(&"x = 1\n".repeat(1_000)).notes.len(), MAX_NOTES);
        assert_eq!(kinds(&read(&" ".repeat(MAX_BYTES + 1))), [NoteKind::TooLarge]);
    }

    /// Every manifest under the resources folder named by `OBSERVER_MANIFEST_CORPUS`, read as
    /// FXServer finds them, with counts printed for comparison with the folder's survey; then the
    /// model-info metas their data files name, matched by the globs.
    #[test]
    fn a_real_resources_folder_reads_without_a_failure() {
        let Some(root) = std::env::var_os("OBSERVER_MANIFEST_CORPUS") else {
            return;
        };
        let root = PathBuf::from(root);
        let every: Vec<PathBuf> = files_below(&root, true)
            .into_iter()
            .filter(|file| file.ends_with("/fxmanifest.lua") || file.ends_with("/__resource.lua"))
            .map(|file| root.join(file))
            .collect();
        let resources = resource_manifests(&root);
        summarise(&root, "every manifest file", &every);
        summarise(&root, "resources FXServer finds", &resources);
        model_infos(&resources);
    }

    /// The manifests of the resources FXServer finds: `[category]` folders opened, any other folder
    /// a resource, which reads `fxmanifest.lua` when it has one.
    fn resource_manifests(root: &Path) -> Vec<PathBuf> {
        let mut resources = Vec::new();
        let mut folders = std::collections::VecDeque::from([root.to_path_buf()]);
        while let Some(folder) = folders.pop_front() {
            let mut entries: Vec<_> = fs::read_dir(&folder).into_iter().flatten().flatten().collect();
            entries.sort_by_key(fs::DirEntry::file_name);
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path();
                if !path.is_dir() || name.starts_with('.') || name.eq_ignore_ascii_case("txadmin") {
                    continue;
                }
                if name.starts_with('[') && name.ends_with(']') {
                    folders.push_back(path);
                } else if let Some(manifest) = ["fxmanifest.lua", "__resource.lua"]
                    .iter()
                    .map(|file| path.join(file))
                    .find(|file| file.is_file())
                {
                    resources.push(manifest);
                }
            }
        }
        resources
    }

    fn summarise(root: &Path, label: &str, paths: &[PathBuf]) {
        let mut types: BTreeMap<String, usize> = BTreeMap::new();
        let mut notes: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut with_data_files = 0;
        let mut data_files = 0;
        let mut refused = Vec::new();
        for path in paths {
            let manifest = read(&String::from_utf8_lossy(
                &fs::read(path).expect("a manifest can be read"),
            ));
            with_data_files += usize::from(!manifest.data_files.is_empty());
            data_files += manifest.data_files.len();
            for data_file in &manifest.data_files {
                *types.entry(data_file.kind.to_ascii_uppercase()).or_default() += 1;
            }
            for note in &manifest.notes {
                *notes.entry(note.kind.name()).or_default() += 1;
                if note.kind != NoteKind::Encrypted {
                    println!(
                        "  note {}:{} {} {}",
                        relative(root, path),
                        note.line,
                        note.kind.name(),
                        note.detail
                    );
                }
            }
            if manifest.refused() {
                refused.push(relative(root, path));
            }
        }
        println!(
            "{label}: {} manifests, {data_files} data files in {with_data_files} of them, {} refused",
            paths.len(),
            refused.len()
        );
        for kind in MODEL_INFO_TYPES.iter().chain(&["DLC_ITYP_REQUEST"]) {
            println!("  {kind}: {}", types.get(*kind).copied().unwrap_or(0));
        }
        println!("  notes: {notes:?}");
        println!("  refused: {refused:?}");
    }

    const MODEL_INFO_TYPES: [&str; 3] = [
        "VEHICLE_METADATA_FILE",
        "WEAPON_METADATA_FILE",
        "PED_METADATA_FILE",
    ];

    /// The files the model-info data files of each resource name, matched against its folder and
    /// against what FXServer ships of it (`.y*` files of `stream/` other than `.ytyp` left out),
    /// and how many of them hold model infos.
    fn model_infos(resources: &[PathBuf]) {
        let mut in_folder = 0;
        let mut shipped_count = 0;
        let mut with_model_infos = 0;
        for manifest_path in resources {
            let manifest = read(&String::from_utf8_lossy(&fs::read(manifest_path).expect("read")));
            let patterns: Vec<&str> = manifest
                .data_files
                .iter()
                .filter(|data_file| MODEL_INFO_TYPES.contains(&data_file.kind.to_ascii_uppercase().as_str()))
                .map(|data_file| data_file.path.as_str())
                .collect();
            if patterns.is_empty() {
                continue;
            }
            let folder = manifest_path.parent().expect("a manifest has a folder");
            let files = files_below(folder, false);
            let shipped: Vec<String> = manifest
                .shipped_patterns()
                .flat_map(|entry| {
                    crate::glob::expand_with(&entry.value, &files, crate::glob::Folders::SERVER)
                })
                .filter(|file| {
                    // The extension starts at the path's first dot, as FXServer cuts it.
                    let extension = file.find('.').map_or("", |dot| &file[dot..]);
                    !(file.starts_with("stream/")
                        && extension.as_bytes().get(1) == Some(&b'y')
                        && extension != ".ytyp")
                })
                .collect();
            let mut matched = std::collections::BTreeSet::new();
            let mut loaded = std::collections::BTreeSet::new();
            for pattern in patterns {
                matched.extend(crate::glob::expand(pattern, &files));
                for path in crate::glob::data_file_paths(pattern, &shipped) {
                    if let Some(found) = crate::glob::lookup(&path, &shipped) {
                        loaded.insert(found.to_owned());
                    }
                }
            }
            in_folder += matched.len();
            shipped_count += loaded.len();
            with_model_infos += loaded
                .iter()
                .filter(|file| {
                    let text = fs::read(folder.join(file)).unwrap_or_default();
                    let text = String::from_utf8_lossy(&text);
                    text.contains("ModelInfo__InitDataList")
                })
                .count();
        }
        println!(
            "model-info data files: {in_folder} files matched in the folders, {shipped_count} among the \
             shipped files, {with_model_infos} of these holding model infos"
        );
    }

    /// Every file below `folder`, relative to it with `/` between names; folders starting with a
    /// dot left out when `skip_hidden`, as FXServer leaves them out of its walks (but not its globs).
    fn files_below(folder: &Path, skip_hidden: bool) -> Vec<String> {
        let mut files = Vec::new();
        let mut folders = vec![folder.to_path_buf()];
        while let Some(current) = folders.pop() {
            for entry in fs::read_dir(&current).into_iter().flatten().flatten() {
                let name = entry.file_name();
                let path = entry.path();
                if path.is_dir() {
                    if !(skip_hidden && name.to_string_lossy().starts_with('.')) {
                        folders.push(path);
                    }
                } else {
                    files.push(relative(folder, &path).replace('\\', "/"));
                }
            }
        }
        files
    }

    fn relative(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }
}
