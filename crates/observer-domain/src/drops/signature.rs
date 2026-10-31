//! Crash texts to signatures, and drop reasons to templates.
//!
//! A signature groups the crashes that are the same crash, whoever had it. Its forms follow what
//! FiveM's `MiniDump.cpp` writes after the "Game crashed: " prefix: a code name and the blamed
//! module, a symbolised place, an error while loading an asset, an error code, a C++ exception,
//! or a text of its own.

use super::{MAX_RESOURCE_BYTES, cut};

/// The rules' version, stored with each signature: bumped when a rule changes, so signatures
/// made by older rules can be told apart and made again.
pub const NORMALISER: u32 = 1;
/// The longest signature kept, in bytes.
pub const MAX_SIGNATURE_BYTES: usize = 256;
/// The longest template kept, in characters.
pub const MAX_TEMPLATE_CHARS: usize = 160;

/// The game executables FiveM and RedM write with their build: `GTA5_b3095.exe`, `RDR2_b1491.exe`.
const GAMES: [(&str, &str); 2] = [("gta5", "GTA5"), ("rdr2", "RDR2")];
/// What a module blamed without a code name, or symbolised, ends with.
const MODULE_EXTENSIONS: [&str; 2] = ["exe", "dll"];
/// What `classify` turns every translation of "Unhandled exception: " into.
const EXCEPTION_PREFIX: &str = "Unhandled exception: ";
/// Where the game's file system mounts the server's resources.
const RESOURCES: &str = "resources:/";
/// The parts of a signature that are not a template are client text too: they keep only the
/// characters, and the lengths, the names FiveM writes have. Anything else is free text.
const MOST_MODULE: usize = 64;
const MOST_OFFSET: usize = 16;
const MOST_SYMBOL: usize = 128;
const MOST_RESOURCE: usize = 64;
const MOST_ERROR: usize = 64;
const MOST_CODE_WORD: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureForm {
    /// A code name and the blamed module's offset: `kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)`.
    Hash,
    /// A symbolised place: `GTA5_b3095.exe!sub_140A1B2C3 (0x1c)`.
    Symbol,
    /// One of the game's error codes: `ERR_GFX_D3D_INIT`.
    Error,
    /// An exception while the game loaded a resource's asset.
    Load,
    /// A C++ exception's type and message.
    Exception,
    /// Anything else.
    Text,
}

impl SignatureForm {
    pub const ALL: [Self; 6] = [
        Self::Hash,
        Self::Symbol,
        Self::Error,
        Self::Load,
        Self::Exception,
        Self::Text,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hash => "hash",
            Self::Symbol => "symbol",
            Self::Error => "error",
            Self::Load => "load",
            Self::Exception => "exception",
            Self::Text => "text",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|form| form.as_str() == name)
    }

    /// Text a client wrote: shown to statistics viewers only once 2 players produced it.
    pub const fn free_text(self) -> bool {
        matches!(self, Self::Exception | Self::Text)
    }
}

/// What groups a crash with the same crashes, and what it says about them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// The form's name, a colon, and what tells the crash apart: `hash:GTA5_b3095.exe+1A2B3C`.
    pub signature: String,
    pub form: SignatureForm,
    /// The blamed module without the game's build: `GTA5.exe`, `gta-streaming-five.dll`.
    pub module: Option<String>,
    /// The game's build the blamed executable names: 3095 from `GTA5_b3095.exe`.
    pub game_build: Option<u32>,
    /// The crash's code name: `kilo-ohio-lemon`.
    pub label: Option<String>,
    /// For a load crash, the resource whose asset failed.
    pub resource: Option<String>,
}

/// The signature of a crash's text, as `classify` gives it once redacted. The most precise form
/// wins: a symbol, a hash, a load, an error code, an exception, then the text itself.
pub fn signature(crash_text: &str) -> Signature {
    let text = crash_text.trim();
    let named = named_blame(text);
    let label = named
        .and_then(|blame| blame.label)
        .map(|label| cut(label, MAX_SIGNATURE_BYTES).to_owned());
    if let Some(symbol) = symbol(text) {
        let blamed = Blamed::new(symbol.module);
        let offset = symbol.offset.map_or_else(String::new, |offset| {
            format!("+0x{}", offset.to_ascii_lowercase())
        });
        return Signature {
            module: Some(blamed.module),
            game_build: blamed.build,
            label,
            ..plain(
                SignatureForm::Symbol,
                &format!("{}!{}{offset}", blamed.written, symbol.name),
            )
        };
    }
    if let Some(blame) = named.or_else(|| bare_blame(text)) {
        let blamed = Blamed::new(blame.module);
        return Signature {
            module: Some(blamed.module),
            game_build: blamed.build,
            label,
            ..plain(
                SignatureForm::Hash,
                &format!("{}+{}", blamed.written, blame.offset.to_ascii_uppercase()),
            )
        };
    }
    if let Some((resource, code)) = load(text) {
        return Signature {
            resource: Some(resource.to_owned()),
            ..plain(
                SignatureForm::Load,
                &format!("{}:{resource}", code.as_deref().unwrap_or("unknown")),
            )
        };
    }
    if let Some(code) = error_code(text) {
        return plain(SignatureForm::Error, code);
    }
    if let Some(rest) = strip_ascii_prefix(text, EXCEPTION_PREFIX) {
        return free(SignatureForm::Exception, rest);
    }
    free(SignatureForm::Text, text)
}

/// A reason's group: numbers, addresses, quoted strings, GUIDs become placeholders; the dropped
/// player's own name becomes `{player}`; whitespace collapsed; at most `MAX_TEMPLATE_CHARS`
/// characters.
pub fn template(text: &str, player_name: Option<&str>) -> String {
    template_within(text, player_name, MAX_TEMPLATE_CHARS, usize::MAX)
}

/// A signature with nothing more than its form and text, cut to its cap.
fn plain(form: SignatureForm, body: &str) -> Signature {
    let mut signature = format!("{}:{body}", form.as_str());
    signature.truncate(cut(&signature, MAX_SIGNATURE_BYTES).len());
    Signature {
        signature,
        form,
        module: None,
        game_build: None,
        label: None,
        resource: None,
    }
}

/// A signature whose body is a template, cut so that no placeholder is cut in half.
fn free(form: SignatureForm, text: &str) -> Signature {
    let budget = MAX_SIGNATURE_BYTES - form.as_str().len() - 1;
    plain(form, &template_within(text, None, MAX_TEMPLATE_CHARS, budget))
}

/// A blamed module as the signature writes it, and without the game's build.
struct Blamed {
    written: String,
    module: String,
    build: Option<u32>,
}

impl Blamed {
    /// The game's executable is recognised whatever its case and written as FiveM writes it;
    /// any other module keeps its own case.
    fn new(written: &str) -> Self {
        let lower = written.to_ascii_lowercase();
        GAMES
            .iter()
            .find_map(|&(prefix, game)| {
                let build = lower
                    .strip_prefix(prefix)?
                    .strip_prefix("_b")?
                    .strip_suffix(".exe")?;
                if !build.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                let build: u32 = build.parse().ok()?;
                Some(Self {
                    written: format!("{game}_b{build}.exe"),
                    module: format!("{game}.exe"),
                    build: Some(build),
                })
            })
            .unwrap_or_else(|| Self {
                written: written.to_owned(),
                module: cut(written, MAX_SIGNATURE_BYTES).to_owned(),
                build: None,
            })
    }
}

/// A module and an offset in it, with the crash's code name when FiveM gave one.
#[derive(Clone, Copy)]
struct Blame<'a> {
    label: Option<&'a str>,
    module: &'a str,
    offset: &'a str,
}

/// `kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)`: FiveM's code name and the blamed module, the whole
/// text as FiveM writes it. Words around them are a client's, and make the text free.
fn named_blame(text: &str) -> Option<Blame<'_>> {
    text.match_indices(" (").find_map(|(at, _)| {
        let label = &text[..at];
        if !is_code_name(label) {
            return None;
        }
        let inside = text[at + 2..].strip_suffix(')')?;
        if inside.len() > MOST_MODULE + 1 + MOST_OFFSET + 2 {
            return None;
        }
        let (module, offset) = inside.rsplit_once('+')?;
        let offset = without_hex_prefix(offset);
        let blamed = module_shaped(module)
            && (1..=MOST_OFFSET).contains(&offset.len())
            && offset.bytes().all(|byte| byte.is_ascii_hexdigit());
        blamed.then_some(Blame {
            label: Some(label),
            module,
            offset,
        })
    })
}

/// A module and its offset with no code name before them, the whole text:
/// `gta-streaming-five.dll+4F2A1`.
fn bare_blame(text: &str) -> Option<Blame<'_>> {
    text.match_indices('+').find_map(|(at, _)| {
        let module = module_before(text, at).filter(|module| module.len() == at)?;
        let after = without_hex_prefix(&text[at + 1..]);
        let offset = hex_run(after);
        let ends = offset.len() == after.len();
        ((1..=MOST_OFFSET).contains(&offset.len()) && ends).then_some(Blame {
            label: None,
            module,
            offset,
        })
    })
}

/// Three lowercase words joined by hyphens, as FiveM names a crash: `kilo-ohio-lemon`.
fn is_code_name(word: &str) -> bool {
    word.split('-').count() == 3
        && word.split('-').all(|part| {
            (1..=MOST_CODE_WORD).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_lowercase())
        })
}

/// A module as FiveM names one: letters, digits, `_`, `.` and `-`, an `.exe` or `.dll` at the end.
fn module_shaped(module: &str) -> bool {
    let Some((stem, extension)) = module.rsplit_once('.') else {
        return false;
    };
    module.len() <= MOST_MODULE
        && !stem.is_empty()
        && MODULE_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
        && few_digits(module)
        && module
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// No run of more than five digits: a game build has four, a phone number ten.
fn few_digits(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_digit())
        .all(|run| run.len() <= 5)
}

/// A symbolised place: `GTA5_b3095.exe!sub_140A1B2C3 (0x1c)`, or `+0x1c` right after the name.
struct Symbol<'a> {
    module: &'a str,
    name: &'a str,
    offset: Option<&'a str>,
}

/// The whole text: `GTA5_b3095.exe!sub_140A1B2C3 (0x1c)`.
fn symbol(text: &str) -> Option<Symbol<'_>> {
    text.match_indices('!').find_map(|(at, _)| {
        let module = module_before(text, at).filter(|module| module.len() == at)?;
        let rest = &text[at + 1..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '+'))
            .unwrap_or(rest.len());
        let name = &rest[..end];
        let shaped = (1..=MOST_SYMBOL).contains(&name.len())
            && few_digits(name)
            && name.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'_' | b':' | b'<' | b'>' | b'~' | b'$' | b'@' | b'?' | b'.')
            });
        let tail = &rest[end..];
        let offset = symbol_offset(tail);
        // Nothing after the name and its offset.
        let whole = match tail.trim_start().strip_prefix('(') {
            Some(inside) => inside.ends_with(')') && offset.is_some(),
            None => {
                tail.is_empty()
                    || (offset.is_some()
                        && tail
                            .trim_start_matches(['+', '0', 'x', 'X'])
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit()))
            }
        };
        (shaped && whole).then_some(Symbol { module, name, offset })
    })
}

fn symbol_offset(after: &str) -> Option<&str> {
    if let Some(offset) = after.strip_prefix('+') {
        let digits = hex_run(without_hex_prefix(offset));
        return (!digits.is_empty()).then_some(digits);
    }
    let inside = without_hex_prefix(after.trim_start().strip_prefix('(')?);
    let digits = hex_run(inside);
    (!digits.is_empty() && inside[digits.len()..].starts_with(')')).then_some(digits)
}

/// The module name that ends at `end`, when it has a module's extension: `GTA5_b3095.exe`.
fn module_before(text: &str, end: usize) -> Option<&str> {
    let start = text[..end]
        .bytes()
        .rposition(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')))
        .map_or(0, |at| at + 1);
    let module = &text[start..end];
    module_shaped(module).then_some(module)
}

/// An exception while the game loaded a file of a resource: `An exception occurred (c0000005 at
/// 0x7ff6bb17f1c9) during loading of resources:/cars/data/...`. The resource, and the exception's
/// code when the text has one.
fn load(text: &str) -> Option<(&str, Option<String>)> {
    // ASCII lowercase keeps every byte where it was.
    let lower = text.to_ascii_lowercase();
    lower.match_indices(RESOURCES).find_map(|(at, _)| {
        let path = &text[at + RESOURCES.len()..];
        let end = path
            .find(|c: char| matches!(c, '/' | '\\') || c.is_whitespace())
            .unwrap_or(path.len());
        let resource = &path[..end];
        let shaped = resource.len() <= MOST_RESOURCE.min(MAX_RESOURCE_BYTES)
            && few_digits(resource)
            && resource.bytes().any(|byte| byte != b'.')
            && resource
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'));
        shaped.then(|| (resource, exception_code(text)))
    })
}

/// The 8 hex digits of `(c0000005 at 0x7ff6bb17f1c9)`, lowercased.
fn exception_code(text: &str) -> Option<String> {
    text.match_indices('(').find_map(|(at, _)| {
        let inside = &text[at + 1..];
        let code = inside.get(..8)?;
        (code.bytes().all(|byte| byte.is_ascii_hexdigit()) && inside[8..].starts_with(" at"))
            .then(|| code.to_ascii_lowercase())
    })
}

/// The first of the game's error codes: `ERR_` and at least 3 capitals, digits or underscores,
/// not inside a longer word (`D3DERR_INVALIDCALL` is Direct3D's).
fn error_code(text: &str) -> Option<&str> {
    text.match_indices("ERR_").find_map(|(at, _)| {
        if text[..at].chars().next_back().is_some_and(is_word) {
            return None;
        }
        let length = text[at + 4..]
            .bytes()
            .take_while(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
            .count();
        let code = &text[at..at + 4 + length];
        ((3..=MOST_ERROR - 4).contains(&length) && few_digits(code)).then_some(code)
    })
}

/// `text` without a prefix, compared without ASCII case.
fn strip_ascii_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len())
        .filter(|start| start.eq_ignore_ascii_case(prefix))
        .map(|_| &text[prefix.len()..])
}

fn without_hex_prefix(text: &str) -> &str {
    text.strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text)
}

fn hex_run(text: &str) -> &str {
    &text[..text.bytes().take_while(u8::is_ascii_hexdigit).count()]
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A template being written: whitespace collapsed, within its caps, each placeholder whole or
/// not at all.
struct Template {
    text: String,
    chars: usize,
    max_chars: usize,
    max_bytes: usize,
    /// Whitespace came since the last piece: one space goes before the next.
    space: bool,
    /// A piece did not fit: the rest is cut.
    full: bool,
}

impl Template {
    fn push(&mut self, piece: &str) {
        let space = usize::from(self.space && !self.text.is_empty());
        let chars = space + piece.chars().count();
        if self.chars + chars > self.max_chars || self.text.len() + space + piece.len() > self.max_bytes {
            self.full = true;
            return;
        }
        if space == 1 {
            self.text.push(' ');
        }
        self.text.push_str(piece);
        self.chars += chars;
        self.space = false;
    }

    fn finish(self) -> String {
        if self.text.is_empty() {
            "{empty}".to_owned()
        } else {
            self.text
        }
    }
}

fn template_within(text: &str, player_name: Option<&str>, max_chars: usize, max_bytes: usize) -> String {
    let player = player_name
        .map(str::trim)
        .filter(|name| name.chars().count() >= 2);
    let mut template = Template {
        text: String::new(),
        chars: 0,
        max_chars,
        max_bytes,
        space: false,
        full: false,
    };
    let mut previous: Option<char> = None;
    let mut at = 0;
    while at < text.len() && !template.full {
        let rest = &text[at..];
        let word_start = previous.is_none_or(|c| !is_word(c));
        let length = if let Some((placeholder, length)) = placeholder_at(rest, word_start, player) {
            template.push(placeholder);
            length
        } else {
            let Some(c) = rest.chars().next() else { break };
            if c.is_whitespace() {
                template.space = true;
            } else {
                template.push(c.encode_utf8(&mut [0; 4]));
            }
            c.len_utf8()
        };
        previous = rest[..length].chars().next_back();
        at += length;
    }
    template.finish()
}

/// The placeholder `rest` starts with, and the length it stands for. The player's name comes
/// first, so a name with digits or quotes is still the player.
fn placeholder_at(rest: &str, word_start: bool, player: Option<&str>) -> Option<(&'static str, usize)> {
    if let Some(length) = player.and_then(|name| name_at(rest, name, word_start)) {
        return Some(("{player}", length));
    }
    if word_start {
        if let Some(length) = guid_at(rest) {
            return Some(("{guid}", length));
        }
        if let Some(length) = address_at(rest) {
            return Some(("{addr}", length));
        }
        if let Some(length) = number_at(rest) {
            return Some(("{n}", length));
        }
    }
    match rest.as_bytes().first() {
        Some(b'"') => rest[1..].find('"').map(|end| ("{text}", end + 2)),
        // An apostrophe inside a word ("don't", "players'") opens nothing, nor closes.
        Some(b'\'') if word_start => rest[1..]
            .match_indices('\'')
            .map(|(end, _)| end + 2)
            .find(|&length| rest[length..].chars().next().is_none_or(|c| !is_word(c)))
            .map(|length| ("{text}", length)),
        _ => None,
    }
}

/// The length of the player's name at the start of `rest`, compared without case, when it is
/// a whole occurrence: no letter or digit glued to either end.
fn name_at(rest: &str, name: &str, word_start: bool) -> Option<usize> {
    if !word_start && name.chars().next().is_some_and(is_word) {
        return None;
    }
    let mut chars = rest.char_indices();
    for wanted in name.chars() {
        let (_, found) = chars.next()?;
        if found != wanted && !found.to_lowercase().eq(wanted.to_lowercase()) {
            return None;
        }
    }
    let length = chars.next().map_or(rest.len(), |(at, _)| at);
    let glued_after =
        name.chars().next_back().is_some_and(is_word) && rest[length..].chars().next().is_some_and(is_word);
    (!glued_after).then_some(length)
}

/// A GUID, bare or in braces: `6F9619FF-8B86-D011-B42D-00C04FC964FF`.
fn guid_at(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let braced = bytes.first() == Some(&b'{');
    let mut at = usize::from(braced);
    for (group, length) in [8, 4, 4, 4, 12].into_iter().enumerate() {
        if group > 0 {
            if bytes.get(at) != Some(&b'-') {
                return None;
            }
            at += 1;
        }
        if !bytes.get(at..at + length)?.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        at += length;
    }
    if braced {
        return (bytes.get(at) == Some(&b'}')).then_some(at + 1);
    }
    let glued = rest[at..].chars().next().is_some_and(|c| is_word(c) || c == '-');
    (!glued).then_some(at)
}

/// An address: `0x` and hex digits, or a bare hex word with digits and letters, longer than an
/// exception code's 8 digits so that `c0000005` stays: `7ff6bb17f1c9`.
fn address_at(rest: &str) -> Option<usize> {
    if let Some(digits) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        let length = hex_run(digits).len();
        return (length > 0).then_some(2 + length);
    }
    let word = &rest[..rest.find(|c: char| !is_word(c)).unwrap_or(rest.len())];
    let address = word.len() > 8
        && word.bytes().all(|byte| byte.is_ascii_hexdigit())
        && word.bytes().any(|byte| byte.is_ascii_digit())
        && word.bytes().any(|byte| byte.is_ascii_alphabetic());
    address.then_some(word.len())
}

/// A number: digits, with the `,` or `.` between their groups: `1,234`, `12.5`.
fn number_at(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    if !bytes.first()?.is_ascii_digit() {
        return None;
    }
    let mut at = 0;
    loop {
        at += bytes[at..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        match (bytes.get(at), bytes.get(at + 1)) {
            (Some(b',' | b'.'), Some(next)) if next.is_ascii_digit() => at += 1,
            _ => return Some(at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drops::{ReportedDrop, classify};

    /// A crash text, its signature, and the module, game build, code name and resource it names.
    type Case = (
        &'static str,
        &'static str,
        Option<&'static str>,
        Option<u32>,
        Option<&'static str>,
        Option<&'static str>,
    );

    const CORPUS: &[Case] = &[
        // A code name and the blamed module.
        (
            "kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)",
            "hash:GTA5_b3095.exe+1A2B3C",
            Some("GTA5.exe"),
            Some(3095),
            Some("kilo-ohio-lemon"),
            None,
        ),
        (
            "kilo-ohio-lemon (GTA5_b3095.exe+1a2b3c)",
            "hash:GTA5_b3095.exe+1A2B3C",
            Some("GTA5.exe"),
            Some(3095),
            Some("kilo-ohio-lemon"),
            None,
        ),
        (
            "kilo-ohio-lemon (gta5_B3095.EXE+1A2B3C)",
            "hash:GTA5_b3095.exe+1A2B3C",
            Some("GTA5.exe"),
            Some(3095),
            Some("kilo-ohio-lemon"),
            None,
        ),
        (
            "nevada-bacon-salami (GTA5_b2802.exe+0x16B7A3F)",
            "hash:GTA5_b2802.exe+16B7A3F",
            Some("GTA5.exe"),
            Some(2802),
            Some("nevada-bacon-salami"),
            None,
        ),
        (
            "papa-delta-nine (RDR2_b1491.exe+5C8D2E)",
            "hash:RDR2_b1491.exe+5C8D2E",
            Some("RDR2.exe"),
            Some(1491),
            Some("papa-delta-nine"),
            None,
        ),
        (
            "whiskey-mars-ten (gta-streaming-five.dll+4F2A1)",
            "hash:gta-streaming-five.dll+4F2A1",
            Some("gta-streaming-five.dll"),
            None,
            Some("whiskey-mars-ten"),
            None,
        ),
        (
            "oscar-hotel-zulu (citizen-scripting-lua.dll+2b1c)",
            "hash:citizen-scripting-lua.dll+2B1C",
            Some("citizen-scripting-lua.dll"),
            None,
            Some("oscar-hotel-zulu"),
            None,
        ),
        (
            "alpha-bravo-six (GTA5.exe+1A2B3C)",
            "hash:GTA5.exe+1A2B3C",
            Some("GTA5.exe"),
            None,
            Some("alpha-bravo-six"),
            None,
        ),
        (
            "gta-streaming-five.dll+4A2B1",
            "hash:gta-streaming-five.dll+4A2B1",
            Some("gta-streaming-five.dll"),
            None,
            None,
            None,
        ),
        (
            "GTA5_b3095.exe+1a2b3c",
            "hash:GTA5_b3095.exe+1A2B3C",
            Some("GTA5.exe"),
            Some(3095),
            None,
            None,
        ),
        (
            // A blame is the whole text as FiveM writes it: with more after it, the error code.
            "kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C) ERR_GFX_D3D_INIT",
            "error:ERR_GFX_D3D_INIT",
            None,
            None,
            None,
            None,
        ),
        (
            // Eleven digits are no build FiveM writes.
            "GTA5_b99999999999.exe+1A",
            "text:GTA5_b99999999999.exe+{n}A",
            None,
            None,
            None,
            None,
        ),
        // A symbolised place.
        (
            "GTA5_b3095.exe!sub_140A1B2C3 (0x1c)",
            "symbol:GTA5_b3095.exe!sub_140A1B2C3+0x1c",
            Some("GTA5.exe"),
            Some(3095),
            None,
            None,
        ),
        (
            "GTA5_b3095.exe!sub_140A1B2C3 (0x1C)",
            "symbol:GTA5_b3095.exe!sub_140A1B2C3+0x1c",
            Some("GTA5.exe"),
            Some(3095),
            None,
            None,
        ),
        (
            "RDR2_b1491.exe!sub_142D0E1F0 (0x3a)",
            "symbol:RDR2_b1491.exe!sub_142D0E1F0+0x3a",
            Some("RDR2.exe"),
            Some(1491),
            None,
            None,
        ),
        (
            "citizen-scripting-v8client.dll!v8::internal::Heap::CollectGarbage (0x5d)",
            "symbol:citizen-scripting-v8client.dll!v8::internal::Heap::CollectGarbage+0x5d",
            Some("citizen-scripting-v8client.dll"),
            None,
            None,
            None,
        ),
        (
            "ntdll.dll!RtlpWaitOnCriticalSection+0xFB",
            "symbol:ntdll.dll!RtlpWaitOnCriticalSection+0xfb",
            Some("ntdll.dll"),
            None,
            None,
            None,
        ),
        (
            "GTA5_b3095.exe!sub_140A1B2C3",
            "symbol:GTA5_b3095.exe!sub_140A1B2C3",
            Some("GTA5.exe"),
            Some(3095),
            None,
            None,
        ),
        (
            // FiveM writes a symbol instead of the code name, never both: a client wrote this.
            "kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C) GTA5_b3095.exe!sub_140A1B2C3 (0x1c)",
            "text:kilo-ohio-lemon (GTA5_b3095.exe+{n}A2B3C) GTA5_b3095.exe!sub_140A1B2C3 ({addr})",
            None,
            None,
            None,
            None,
        ),
        // An error while loading an asset.
        (
            "Recursive error: An exception occurred (c0000005 at 0x7ff6bb17f1c9) during loading of \
             resources:/cars/data/[limiteds]/xmas 4/carvariations.meta in data file mounter \
             0x141a22350. This may be a broken resource.",
            "load:c0000005:cars",
            None,
            None,
            None,
            Some("cars"),
        ),
        (
            "Recursive error: An exception occurred (C0000409 at 0x7FF6BB17F1C9) during loading of \
             resources:/my-maps/stream/[mlo] legion/v_int_49.ytyp in data file mounter 0x141a22350.",
            "load:c0000409:my-maps",
            None,
            None,
            None,
            Some("my-maps"),
        ),
        (
            "Error during loading of resources:/eup-stream/stream/[peds] police/mp_m_freemode_01.ytd",
            "load:unknown:eup-stream",
            None,
            None,
            None,
            Some("eup-stream"),
        ),
        (
            "ERR_STR_FAILURE: An exception occurred (c0000005 at 0x7ff6bb17f1c9) during loading of \
             resources:/cars/data/vehicles.meta",
            "load:c0000005:cars",
            None,
            None,
            None,
            Some("cars"),
        ),
        // An error code.
        (
            "ERR_GFX_D3D_INIT",
            "error:ERR_GFX_D3D_INIT",
            None,
            None,
            None,
            None,
        ),
        (
            "ERR_MEM_EMBEDDEDALLOC_ALLOC: failed to allocate 1048576 bytes",
            "error:ERR_MEM_EMBEDDEDALLOC_ALLOC",
            None,
            None,
            None,
            None,
        ),
        (
            "The game ran into ERR_GEN_ZLIB_2 while streaming",
            "error:ERR_GEN_ZLIB_2",
            None,
            None,
            None,
            None,
        ),
        (
            "Unhandled exception: ERR_SYS_PURE_CALL",
            "error:ERR_SYS_PURE_CALL",
            None,
            None,
            None,
            None,
        ),
        // A C++ exception.
        (
            "Unhandled exception: std::bad_alloc",
            "exception:std::bad_alloc",
            None,
            None,
            None,
            None,
        ),
        (
            "Unhandled exception: Access violation reading location 0x00007FF6BB17F1C9",
            "exception:Access violation reading location {addr}",
            None,
            None,
            None,
            None,
        ),
        (
            "Unhandled exception: std::runtime_error: invalid key 'weapon_pistol'",
            "exception:std::runtime_error: invalid key {text}",
            None,
            None,
            None,
            None,
        ),
        (
            "unhandled exception: std::length_error",
            "exception:std::length_error",
            None,
            None,
            None,
            None,
        ),
        // Anything else.
        (
            "Out of memory after 3 attempts (1,048,576 bytes)",
            "text:Out of memory after {n} attempts ({n} bytes)",
            None,
            None,
            None,
            None,
        ),
        (
            r"Failed to read C:\Users\[user]\AppData\Local\FiveM\FiveM.app\data\cache\[REDACTED]",
            r"text:Failed to read C:\Users\[user]\AppData\Local\FiveM\FiveM.app\data\cache\[REDACTED]",
            None,
            None,
            None,
            None,
        ),
        (
            "Session [token] of [license] from [ip] lost",
            "text:Session [token] of [license] from [ip] lost",
            None,
            None,
            None,
            None,
        ),
        (
            "Device {6F9619FF-8B86-D011-B42D-00C04FC964FF} was removed",
            "text:Device {guid} was removed",
            None,
            None,
            None,
            None,
        ),
        (
            "Corrupted heap at 7ff6bb17f1c9",
            "text:Corrupted heap at {addr}",
            None,
            None,
            None,
            None,
        ),
        (
            "  The   game\tstopped\n responding  ",
            "text:The game stopped responding",
            None,
            None,
            None,
            None,
        ),
        (
            "D3DERR_INVALIDCALL",
            "text:D3DERR_INVALIDCALL",
            None,
            None,
            None,
            None,
        ),
        (
            "kilo-ohio-lemon (GTA5_b3095.exe+)",
            "text:kilo-ohio-lemon (GTA5_b3095.exe+)",
            None,
            None,
            None,
            None,
        ),
        (
            "Left (see you soon+1)",
            "text:Left (see you soon+{n})",
            None,
            None,
            None,
            None,
        ),
        ("", "text:{empty}", None, None, None, None),
    ];

    /// Texts no crash has, made to break a parser.
    const HOSTILE: [&str; 30] = [
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        "\"",
        "'",
        "+",
        "!",
        "0x",
        " (",
        "((((",
        "a-b-c (",
        "a-b-c (x+",
        "a-b-c (+1)",
        "a-b-c ( a-b-c ( a-b-c (GTA5_b3095.exe+1A",
        "GTA5_b.exe+1A",
        "GTA5_b3095.exe+",
        "GTA5_b3095.exe+zz",
        "GTA5_b3095.exe!",
        ".exe!x",
        ".dll+1",
        "resources:/",
        "resources://cars",
        "(c0000005 at",
        "ERR_é",
        "{6F9619FF-8B86-D011-B42D-00C04FC964F",
        "🦀1🦀2🦀0x🦀'🦀\"🦀",
        "İİİ ẞ ﬃ \u{0} \u{feff} تعطلت",
    ];

    #[test]
    fn every_crash_of_the_corpus_gets_its_signature() {
        for &(text, expected, module, build, label, resource) in CORPUS {
            let found = signature(text);

            let form = expected
                .split_once(':')
                .and_then(|(form, _)| SignatureForm::from_name(form));
            assert_eq!(found.signature, expected, "{text}");
            assert_eq!(Some(found.form), form, "{text}");
            assert_eq!(found.module.as_deref(), module, "{text}");
            assert_eq!(found.game_build, build, "{text}");
            assert_eq!(found.label.as_deref(), label, "{text}");
            assert_eq!(found.resource.as_deref(), resource, "{text}");
        }
    }

    #[test]
    fn the_corpus_covers_every_form() {
        for form in SignatureForm::ALL {
            let prefix = format!("{}:", form.as_str());

            assert!(CORPUS.iter().any(|case| case.1.starts_with(&prefix)), "{prefix}");
        }
    }

    #[test]
    fn hex_offsets_compare_without_their_case() {
        assert_eq!(
            signature("kilo-ohio-lemon (GTA5_b3095.exe+1a2b3c)"),
            signature("kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)")
        );
        assert_eq!(
            signature("GTA5_b3095.exe!sub_140A1B2C3 (0x1C)"),
            signature("GTA5_b3095.exe!sub_140A1B2C3 (0x1c)")
        );
    }

    #[test]
    fn exceptions_differing_only_by_an_address_share_a_signature() {
        let first = signature("Unhandled exception: Access violation at 0x7ff6bb17f1c9 writing 0x0");
        let second = signature("Unhandled exception: Access violation at 0x7FF6BB27A001 writing 0x10");

        assert_eq!(first, second);
        assert_eq!(
            first.signature,
            "exception:Access violation at {addr} writing {addr}"
        );
    }

    #[test]
    fn texts_differing_only_by_numbers_share_a_signature() {
        let first = signature("Streaming stalled on 12 requests after 3.5 seconds");
        let second = signature("Streaming stalled on 1,024 requests after 10 seconds");

        assert_eq!(first, second);
        assert_eq!(
            first.signature,
            "text:Streaming stalled on {n} requests after {n} seconds"
        );
    }

    #[test]
    fn a_translated_exception_and_its_english_share_a_signature() {
        let crash = |reason| {
            let classified = classify(ReportedDrop {
                reason,
                resource: None,
                category: Some(2),
            });
            signature(&classified.crash_text.unwrap())
        };

        assert_eq!(
            crash("Le jeu a cessé de fonctionner : Exception non-gérée : std::bad_alloc"),
            crash("Game crashed: Unhandled exception: std::bad_alloc")
        );
    }

    #[test]
    fn a_signature_never_exceeds_its_cap_and_ends_on_a_character_boundary() {
        let symbol = signature(&format!("GTA5_b3095.exe!{}", "🦀".repeat(100)));
        let text = signature(&"é".repeat(300));
        let module = signature(&format!("{}.dll+1A", "a".repeat(400)));

        // Neither crabs in a symbol nor a 400-letter module is what FiveM writes: free text.
        assert_eq!(symbol.form, SignatureForm::Text);
        assert!(symbol.signature.len() <= MAX_SIGNATURE_BYTES);
        assert_eq!(text.signature, format!("text:{}", "é".repeat(125)));
        assert_eq!(module.form, SignatureForm::Text);
        assert!(module.signature.len() <= MAX_SIGNATURE_BYTES);
    }

    #[test]
    fn a_free_text_signature_cuts_no_placeholder_in_half() {
        let found = signature(&format!("{} 0x1C", "é".repeat(124)));

        assert_eq!(found.signature, format!("text:{}", "é".repeat(124)));
    }

    #[test]
    fn no_text_makes_it_panic() {
        let mut texts: Vec<String> = CORPUS
            .iter()
            .map(|case| case.0)
            .chain(HOSTILE)
            .map(String::from)
            .collect();
        texts.extend([
            "9".repeat(600),
            "1,".repeat(300),
            "🦀".repeat(200),
            "é".repeat(512),
            "'a ".repeat(200),
            format!("kilo-ohio-lemon ({}+1A2B3C)", "x".repeat(400)),
        ]);

        for text in &texts {
            for at in (0..=text.len()).filter(|&at| text.is_char_boundary(at)) {
                for part in [&text[..at], &text[at..]] {
                    assert!(signature(part).signature.len() <= MAX_SIGNATURE_BYTES, "{part}");
                    for name in [None, Some("Bob"), Some("🦀🦀"), Some("İi"), Some("((")] {
                        assert!(
                            template(part, name).chars().count() <= MAX_TEMPLATE_CHARS,
                            "{part}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn forms_round_trip_by_name() {
        for form in SignatureForm::ALL {
            assert_eq!(SignatureForm::from_name(form.as_str()), Some(form));
        }
        assert_eq!(SignatureForm::from_name("Hash"), None);
    }

    #[test]
    fn only_exceptions_and_texts_are_free_text() {
        let free: Vec<SignatureForm> = SignatureForm::ALL
            .into_iter()
            .filter(|form| form.free_text())
            .collect();

        assert_eq!(free, [SignatureForm::Exception, SignatureForm::Text]);
    }

    #[test]
    fn afk_reasons_share_a_template() {
        assert_eq!(template("AFK for 31 minutes", None), "AFK for {n} minutes");
        assert_eq!(template("AFK for 45 minutes", None), "AFK for {n} minutes");
    }

    #[test]
    fn the_players_own_name_becomes_a_placeholder() {
        assert_eq!(
            template("Bob was kicked: bob spammed BOB", Some("Bob")),
            "{player} was kicked: {player} spammed {player}"
        );
        assert_eq!(
            template("xX_Sniper42 was banned for 7 days", Some("xX_Sniper42")),
            "{player} was banned for {n} days"
        );
        assert_eq!(
            template("Kicked [TAG] Bob for 5 warnings", Some(" [TAG] Bob ")),
            "Kicked {player} for {n} warnings"
        );
        assert_eq!(template("ÉLODIE left", Some("élodie")), "{player} left");
    }

    #[test]
    fn a_name_inside_a_longer_word_is_left_alone() {
        assert_eq!(
            template("Bobby left, Bob_ too", Some("Bob")),
            "Bobby left, Bob_ too"
        );
    }

    #[test]
    fn a_name_shorter_than_two_characters_is_left_alone() {
        assert_eq!(template("A left", Some("A")), "A left");
        assert_eq!(template("A left", Some(" ")), "A left");
    }

    #[test]
    fn quoted_strings_become_a_placeholder() {
        assert_eq!(
            template(r#"Banned for using "ESX menu" and 'noclip 2'"#, None),
            "Banned for using {text} and {text}"
        );
    }

    #[test]
    fn an_apostrophe_inside_a_word_opens_no_quote() {
        assert_eq!(
            template("You don't have the players' 3 cars", None),
            "You don't have the players' {n} cars"
        );
        assert_eq!(template("an unclosed \"quote 5", None), "an unclosed \"quote {n}");
    }

    #[test]
    fn guids_become_a_placeholder() {
        assert_eq!(
            template("Session 6f9619ff-8b86-d011-b42d-00c04fc964ff expired", None),
            "Session {guid} expired"
        );
        assert_eq!(
            template("Device {6F9619FF-8B86-D011-B42D-00C04FC964FF} gone", None),
            "Device {guid} gone"
        );
    }

    #[test]
    fn numbers_and_addresses_become_placeholders() {
        assert_eq!(
            template("Lost 1,234 packets in 12.5 seconds at 0x7FF6BB17F1C9.", None),
            "Lost {n} packets in {n} seconds at {addr}."
        );
    }

    #[test]
    fn digits_inside_a_word_stay() {
        assert_eq!(
            template("GTA5 sub_140A1B2C3 x64 c0000005", None),
            "GTA5 sub_140A1B2C3 x64 c0000005"
        );
    }

    #[test]
    fn redaction_markers_stay_as_they_are() {
        let text = r"Banned [license] [discord] [steam] [ip] [token] [REDACTED] in C:\Users\[user]\x";

        assert_eq!(template(text, None), text);
    }

    #[test]
    fn whitespace_collapses_to_one_space() {
        assert_eq!(template("\t Too   many\n\nspaces  ", None), "Too many spaces");
    }

    #[test]
    fn an_empty_reason_has_a_template_too() {
        assert_eq!(template("", None), "{empty}");
        assert_eq!(template(" \n ", None), "{empty}");
    }

    #[test]
    fn a_template_keeps_at_most_its_cap_without_cutting_a_placeholder() {
        let cut_before_a_number = format!("{} 12345", "a".repeat(157));

        assert_eq!(template(&"x".repeat(300), None), "x".repeat(MAX_TEMPLATE_CHARS));
        assert!(template(&"word ".repeat(100), None).ends_with("word"));
        assert_eq!(template(&cut_before_a_number, None), "a".repeat(157));
        assert_eq!(template(&"é".repeat(200), None), "é".repeat(MAX_TEMPLATE_CHARS));
    }

    #[test]
    fn a_structured_form_takes_only_what_fivem_writes_and_anything_else_is_free_text() {
        // A sentence in the module's place, a name in the symbol's, a path segment in the
        // resource's: a modified client's text, so a template that two players must share.
        for text in [
            "kilo-ohio-lemon (Jean.Dupont,0612345678.dll+1)",
            "GTA5_b3095.exe!call_Jean_on_0612345678_he_is_a_",
            "An exception occurred (c0000005 at 0x1) during loading of resources:/Jean,Dupont/x.meta",
            &format!("ERR_{}", "A".repeat(80)),
            &format!("{}-ohio-lemon (GTA5_b3095.exe+1A)", "k".repeat(40)),
        ] {
            assert_eq!(signature(text).form, SignatureForm::Text, "{text}");
        }
        // Words around a real blame are a client's too.
        assert_eq!(
            signature("kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C) call Jean").form,
            SignatureForm::Text
        );
        assert_eq!(signature("ERR_0612345678").form, SignatureForm::Text);
    }
}
