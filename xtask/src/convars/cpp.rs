//! What the generator reads in one C++ file of FXServer: the convars it declares, those it only
//! reads, the console types that name their values, and the `#if` blocks around them. The reading
//! is textual and as narrow as the forms the sources use: a form it does not know is reported and
//! left out, never guessed.

use std::collections::HashMap;

use observer_domain::convars::{ConvarFlag, ConvarPlatform, ConvarRange};

/// A declaration as written, before the console types are known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub cpp_type: String,
    /// The default as written, blanks collapsed; `None` when the declaration gives none.
    pub default: Option<String>,
    /// `None` when the flags are not a plain `ConVar_A | ConVar_B`.
    pub flags: Option<Vec<ConvarFlag>>,
    pub range: Option<ConvarRange>,
    pub description: Option<String>,
    pub line: u32,
    pub platform: Option<ConvarPlatform>,
}

/// A convar read by name: `GetVariable("x")`, `FindEntryRaw("x")`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub name: String,
    pub line: u32,
    pub platform: Option<ConvarPlatform>,
}

/// A `ConsoleArgumentType<T>` specialization: how FXServer prints and reads a type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsoleType {
    /// Each enumerator and how `Unparse` prints it: `On` → `on`.
    pub printed: HashMap<String, String>,
    /// Each value `Parse` gives, in order, with the strings it reads as that value.
    pub parsed: Vec<(String, Vec<String>)>,
    /// `Parse` reads numbers too (`std::stoi`).
    pub numbers: bool,
}

#[derive(Debug, Default)]
pub struct Findings {
    pub declarations: Vec<Declaration>,
    pub reads: Vec<Read>,
    /// By the type's last path segment: `fx::OneSyncState` is `OneSyncState`.
    pub types: HashMap<String, ConsoleType>,
    /// Declarations whose name is not a literal, as `line: text`.
    pub skipped: Vec<String>,
    /// Lines of declarations inside an `#if` the generator cannot evaluate, kept.
    pub unknown_conditions: Vec<u32>,
}

/// A rate limiter `GetRateLimiter("key", RateLimiterDefaults{ rate, burst })` asks for, which
/// declares `rateLimiter_<key>_rate` and `rateLimiter_<key>_burst`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimiter {
    pub key: String,
    pub rate: String,
    pub burst: String,
    pub line: u32,
    pub platform: Option<ConvarPlatform>,
}

/// Reads `text`, the file at `path` from the repository's root.
pub fn read(path: &str, text: &str) -> (Findings, Vec<RateLimiter>) {
    let file = File::new(text);
    let mut findings = Findings::default();
    let mut limiters = Vec::new();
    file.types(&mut findings);
    file.declarations(&mut findings);
    file.reads(&mut findings);
    if path.ends_with("citizen-server-main/src/ServerInstance.cpp") {
        file.set_list(&mut findings);
    }
    file.rate_limiters(&mut findings, &mut limiters);
    findings.reads.sort_by_key(|read| read.line);
    (findings, limiters)
}

struct File<'a> {
    text: &'a str,
    /// `text` with every comment blanked, newlines kept: offsets are the same in both.
    code: String,
    line_starts: Vec<usize>,
    lines: Vec<LineState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Truth {
    True,
    False,
    Windows,
    Linux,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineState {
    truth: Truth,
}

impl<'a> File<'a> {
    fn new(text: &'a str) -> Self {
        let code = blank_comments(text);
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        let lines = conditions(&code);
        Self {
            text,
            code,
            line_starts,
            lines,
        }
    }

    /// 1-based.
    fn line_of(&self, offset: usize) -> u32 {
        let index = self.line_starts.partition_point(|start| *start <= offset);
        u32::try_from(index).unwrap_or(u32::MAX)
    }

    fn state(&self, line: u32) -> Truth {
        let index = usize::try_from(line).unwrap_or(usize::MAX).saturating_sub(1);
        self.lines.get(index).map_or(Truth::True, |state| state.truth)
    }

    /// Whether a line counts, its platform, and whether its condition was left unknown.
    fn kept(&self, line: u32) -> Option<(Option<ConvarPlatform>, bool)> {
        match self.state(line) {
            Truth::False => None,
            Truth::True => Some((None, false)),
            Truth::Windows => Some((Some(ConvarPlatform::Windows), false)),
            Truth::Linux => Some((Some(ConvarPlatform::Linux), false)),
            Truth::Unknown => Some((None, true)),
        }
    }

    fn declarations(&self, findings: &mut Findings) {
        let code = self.code.as_str();
        for (at, _) in code.match_indices("AddVariable") {
            if ident_before(code, at) {
                continue;
            }
            let after = skip_blanks(code, at + "AddVariable".len());
            let Some((cpp_type, end)) = angle(code, after) else {
                continue;
            };
            let open = skip_blanks(code, end);
            if code[open..].starts_with('(') {
                self.declaration(findings, at, cpp_type, open, None);
            }
        }
        for (at, _) in code.match_indices("ConVar") {
            if ident_before(code, at) {
                continue;
            }
            let after = skip_blanks(code, at + "ConVar".len());
            let Some((cpp_type, end)) = angle(code, after) else {
                continue;
            };
            let before = code[..at].trim_end();
            if before.ends_with("make_shared<") {
                let close = skip_blanks(code, end);
                if !code[close..].starts_with('>') {
                    continue;
                }
                let open = skip_blanks(code, close + 1);
                if code[open..].starts_with('(') {
                    self.declaration(findings, at, cpp_type, open, None);
                }
            } else if before.ends_with("new") && !ident_before(code, before.len() - "new".len()) {
                let open = skip_blanks(code, end);
                if code[open..].starts_with('(') {
                    self.declaration(findings, at, cpp_type, open, None);
                }
            } else {
                let start = skip_blanks(code, end);
                let name_end = start + code[start..].bytes().take_while(|b| is_ident(*b)).count();
                if name_end == start || code.as_bytes()[start].is_ascii_digit() {
                    continue;
                }
                let open = skip_blanks(code, name_end);
                if code[open..].starts_with(['(', '{']) {
                    self.declaration(findings, at, cpp_type, open, Some(&code[start..name_end]));
                }
            }
        }
        findings.declarations.sort_by_key(|declaration| declaration.line);
    }

    /// The declaration whose call or constructor starts at `at` and whose arguments open at
    /// `open`. `variable` is the variable a `ConVar<T> name(...)` declares.
    fn declaration(
        &self,
        findings: &mut Findings,
        at: usize,
        cpp_type: &str,
        open: usize,
        variable: Option<&str>,
    ) {
        let code = self.code.as_str();
        let line = self.line_of(at);
        let Some((inside, close)) = enclosed(code, open) else {
            return;
        };
        let args = split_args(inside);
        let literal = |index: usize| args.get(index).and_then(|arg| string_literal(arg));
        let (name, rest) = match (literal(0), literal(1)) {
            (Some(name), _) => (name, &args[1..]),
            (None, Some(name)) => (name, &args[2..]),
            _ => {
                // The forwarding helper itself (`make_shared<ConVar<TVar>>(ctx, args...)`) names
                // nothing.
                if !inside.contains("args...") {
                    findings
                        .skipped
                        .push(format!("{line}: {}", collapse(&code[at..=close])));
                }
                return;
            }
        };
        let Some((platform, unknown)) = self.kept(line) else {
            return;
        };
        if unknown {
            findings.unknown_conditions.push(line);
        }
        let flags = rest.first().and_then(|flags| parse_flags(flags));
        let default = rest.get(1).map(|default| collapse(default));
        let variable = variable
            .map(str::to_owned)
            .or_else(|| assigned_variable(&code[statement_start(code, at)..at]));
        let range = variable.and_then(|variable| self.constraints(&variable));
        let start = statement_start(code, at);
        let first = start + code[start..].len() - code[start..].trim_start().len();
        let description = self
            .comment_above(self.line_of(first))
            .or_else(|| self.comment_beside(close));
        findings.declarations.push(Declaration {
            name,
            cpp_type: collapse(cpp_type),
            default,
            flags,
            range,
            description,
            line,
            platform,
        });
    }

    /// `variable->GetHelper()->SetConstraints(min, max)` with whole numbers, anywhere in the file.
    fn constraints(&self, variable: &str) -> Option<ConvarRange> {
        let code = self.code.as_str();
        for call in [
            format!("{variable}->GetHelper()->SetConstraints"),
            format!("{variable}.GetHelper()->SetConstraints"),
        ] {
            for (at, _) in code.match_indices(&call) {
                if ident_before(code, at) {
                    continue;
                }
                let open = skip_blanks(code, at + call.len());
                let Some((inside, _)) = enclosed(code, open) else {
                    continue;
                };
                if let [min, max] = split_args(inside).as_slice()
                    && let (Ok(min), Ok(max)) = (min.trim().parse(), max.trim().parse())
                {
                    return Some(ConvarRange { min, max });
                }
            }
        }
        None
    }

    /// The `//` lines right above `line`, without those that read as code left commented out.
    fn comment_above(&self, line: u32) -> Option<String> {
        let lines: Vec<&str> = self.text.lines().collect();
        let mut index = usize::try_from(line).ok()?.checked_sub(1)?;
        let mut comment = Vec::new();
        while index > 0 {
            index -= 1;
            let Some(text) = lines[index].trim().strip_prefix("//") else {
                break;
            };
            comment.push(text.trim_start_matches('/').trim());
        }
        comment.reverse();
        let kept: Vec<&str> = comment
            .into_iter()
            .filter(|text| !text.is_empty() && !text.ends_with([';', '{', '}', ')']))
            .collect();
        (!kept.is_empty()).then(|| kept.join("\n"))
    }

    /// A `// comment` after the declaration, on the line it ends on.
    fn comment_beside(&self, close: usize) -> Option<String> {
        let end = self.text[close..]
            .find('\n')
            .map_or(self.text.len(), |at| close + at);
        let rest = &self.text[close + 1..end];
        let comment = rest.split_once("//")?.1.trim_start_matches('/').trim();
        (!comment.is_empty()).then(|| comment.to_owned())
    }

    fn reads(&self, findings: &mut Findings) {
        let code = self.code.as_str();
        for needle in ["GetVariable(", "FindEntryRaw("] {
            for (at, _) in code.match_indices(needle) {
                if ident_before(code, at) {
                    continue;
                }
                let Some((inside, _)) = enclosed(code, at + needle.len() - 1) else {
                    continue;
                };
                self.read_name(findings, at, inside);
            }
        }
    }

    fn read_name(&self, findings: &mut Findings, at: usize, inside: &str) {
        let Some(name) = string_literal(inside) else {
            return;
        };
        let line = self.line_of(at);
        if let Some((platform, _)) = self.kept(line) {
            findings.reads.push(Read { name, line, platform });
        }
    }

    /// `setList` in `ServerInstance.cpp`: the convars FXServer takes from the cfg before it
    /// starts, `sv_licenseKey` among them, which closed components read.
    fn set_list(&self, findings: &mut Findings) {
        let code = self.code.as_str();
        let Some(at) = code.find("setList =") else {
            return;
        };
        let open = skip_blanks(code, at + "setList =".len());
        let Some((inside, _)) = enclosed(code, open) else {
            return;
        };
        let mut offset = open + 1;
        for arg in split_args(inside) {
            let start = offset + arg.len() - arg.trim_start().len();
            offset += arg.len() + 1;
            if let Some(name) = string_literal(arg) {
                let line = self.line_of(start);
                if let Some((platform, _)) = self.kept(line) {
                    findings.reads.push(Read { name, line, platform });
                }
            }
        }
    }

    fn rate_limiters(&self, findings: &mut Findings, limiters: &mut Vec<RateLimiter>) {
        let code = self.code.as_str();
        for (at, _) in code.match_indices("GetRateLimiter(") {
            if ident_before(code, at) {
                continue;
            }
            let line = self.line_of(at);
            let Some((inside, close)) = enclosed(code, at + "GetRateLimiter".len()) else {
                continue;
            };
            let args = split_args(inside);
            let Some(key) = args.first().and_then(|key| string_literal(key)) else {
                if args.first().is_some_and(|key| key.contains('"')) {
                    findings
                        .skipped
                        .push(format!("{line}: {}", collapse(&code[at..=close])));
                }
                continue;
            };
            let Some(defaults) = args.get(1).and_then(|defaults| braced(defaults)) else {
                continue;
            };
            let defaults = split_args(defaults);
            let [rate, burst] = defaults.as_slice() else {
                continue;
            };
            let (Some(rate), Some(burst)) = (float_literal(rate), float_literal(burst)) else {
                continue;
            };
            let Some((platform, _)) = self.kept(line) else {
                continue;
            };
            limiters.push(RateLimiter {
                key,
                rate: print_float(rate),
                burst: print_float(burst),
                line,
                platform,
            });
        }
    }

    fn types(&self, findings: &mut Findings) {
        let code = self.code.as_str();
        let needle = "struct ConsoleArgumentType";
        for (at, _) in code.match_indices(needle) {
            let after = skip_blanks(code, at + needle.len());
            let Some((cpp_type, end)) = angle(code, after) else {
                continue;
            };
            // The generic templates (`TArgument, std::enable_if…`) are the built-in types.
            if cpp_type.contains(',') || cpp_type.trim() == "TArgument" {
                continue;
            }
            let Some(open) = code[end..].find('{').map(|offset| end + offset) else {
                continue;
            };
            let Some((body, _)) = enclosed(code, open) else {
                continue;
            };
            let key = last_segment(cpp_type).to_owned();
            findings.types.insert(key, console_type(body));
        }
    }
}

/// How `Unparse` prints each enumerator and what `Parse` reads.
fn console_type(body: &str) -> ConsoleType {
    let mut found = ConsoleType::default();
    if let Some(unparse) = function_body(body, "Unparse") {
        for (at, _) in unparse.match_indices("case ") {
            let rest = &unparse[at + "case ".len()..];
            let Some(colon) = label_end(rest) else {
                continue;
            };
            let label = rest[..colon].trim();
            let after = &rest[colon + 1..];
            let next_case = after.find("case ").unwrap_or(after.len());
            let Some(returned) = after[..next_case].find("return") else {
                continue;
            };
            let value = &after[returned + "return".len()..];
            if let Some(end) = value.find(';')
                && let Some(printed) = string_literal(&value[..end])
            {
                found.printed.insert(last_segment(label).to_owned(), printed);
            }
        }
    }
    if let Some(parse) = function_body(body, "Parse") {
        found.numbers = ["std::stoi", "std::stoul", "strtol", "atoi"]
            .iter()
            .any(|call| parse.contains(call));
        let mut at = 0;
        while let Some(offset) = parse[at..].find("if") {
            let start = at + offset;
            at = start + 2;
            if ident_before(parse, start) || parse.as_bytes().get(start + 2).is_some_and(|b| is_ident(*b)) {
                continue;
            }
            let open = skip_blanks(parse, start + 2);
            let Some((condition, close)) = enclosed(parse, open) else {
                continue;
            };
            let read: Vec<String> = string_literals(condition);
            if read.is_empty() {
                continue;
            }
            // Only what the branch itself assigns: `if (… "xm" …) { inputStr = …; }` gives nothing.
            let open = skip_blanks(parse, close + 1);
            let branch = if parse[open..].starts_with('{') {
                enclosed(parse, open).map(|(block, _)| block)
            } else {
                parse[open..].find(';').map(|end| &parse[open..=open + end])
            };
            let Some(branch) = branch else {
                continue;
            };
            let Some(assign) = branch.find("*out =") else {
                continue;
            };
            let value = &branch[assign + "*out =".len()..];
            let Some(end) = value.find(';') else {
                continue;
            };
            let target = value[..end].trim();
            let value = string_literal(target)
                .or_else(|| found.printed.get(last_segment(target)).cloned())
                .unwrap_or_else(|| last_segment(target).to_owned());
            match found.parsed.iter_mut().find(|(known, _)| *known == value) {
                Some((_, strings)) => strings.extend(read),
                None => found.parsed.push((value, read)),
            }
        }
    }
    found
}

/// Where a `case` label ends: its first `:` that is not part of `::`.
fn label_end(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b':' if bytes.get(index + 1) == Some(&b':') => index += 2,
            b':' => return Some(index),
            b';' | b'{' | b'}' => return None,
            _ => index += 1,
        }
    }
    None
}

/// The body of the first function named `name` in `code`.
fn function_body<'c>(code: &'c str, name: &str) -> Option<&'c str> {
    for (at, _) in code.match_indices(name) {
        if ident_before(code, at) || code.as_bytes().get(at + name.len()).is_some_and(|b| is_ident(*b)) {
            continue;
        }
        let open = skip_blanks(code, at + name.len());
        let (_, close) = enclosed(code, open)?;
        let brace = skip_blanks(code, close + 1);
        if code[brace..].starts_with('{') {
            return enclosed(code, brace).map(|(body, _)| body);
        }
    }
    None
}

/// `text` with comments turned to blanks, newlines and every byte offset kept. String, character
/// and raw string literals stay as written.
fn blank_comments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        if rest.starts_with(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                out[index] = b' ';
                index += 1;
            }
        } else if rest.starts_with(b"/*") {
            let end = find(bytes, index + 2, b"*/").map_or(bytes.len(), |end| end + 2);
            for byte in &mut out[index..end] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
            index = end;
        } else if rest.starts_with(b"R\"") && (index == 0 || !is_ident(bytes[index - 1])) {
            index = raw_string_end(bytes, index).unwrap_or(bytes.len());
        } else if rest[0] == b'"'
            || (rest[0] == b'\'' && (index == 0 || !bytes[index - 1].is_ascii_hexdigit()))
        {
            index = quoted_end(bytes, index);
        } else {
            index += 1;
        }
    }
    // Only whole comments were blanked, and a comment's bytes are all replaced: still UTF-8.
    String::from_utf8(out).unwrap_or_default()
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| from + at)
}

/// The end of the `"…"` or `'…'` starting at `start`, past its closing quote.
fn quoted_end(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'\n' => return index,
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

/// The end of the raw string `R"delim(…)delim"` starting at `start`.
fn raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let open = find(bytes, start + 2, b"(")?;
    let mut closing = vec![b')'];
    closing.extend_from_slice(&bytes[start + 2..open]);
    closing.push(b'"');
    find(bytes, open + 1, &closing).map(|end| end + closing.len())
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn ident_before(code: &str, at: usize) -> bool {
    at > 0 && is_ident(code.as_bytes()[at - 1])
}

fn skip_blanks(code: &str, from: usize) -> usize {
    from + code[from..].len() - code[from..].trim_start().len()
}

/// The template argument of the `<…>` opening at `open`, and the offset past its `>`.
fn angle(code: &str, open: usize) -> Option<(&str, usize)> {
    let bytes = code.as_bytes();
    if bytes.get(open) != Some(&b'<') {
        return None;
    }
    let mut depth = 0;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&code[open + 1..index], index + 1));
                }
            }
            b';' | b'{' | b'}' | b'(' | b')' | b'"' => return None,
            _ => {}
        }
    }
    None
}

/// What the bracket opening at `open` holds, and the offset of its closing bracket. Strings and
/// characters are skipped whole.
fn enclosed(code: &str, open: usize) -> Option<(&str, usize)> {
    let bytes = code.as_bytes();
    let mut stack = Vec::new();
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'{' | b'[' => stack.push(bytes[index]),
            b')' | b'}' | b']' => {
                stack.pop();
                if stack.is_empty() {
                    return Some((&code[open + 1..index], index));
                }
            }
            b'R' if bytes.get(index + 1) == Some(&b'"') && !ident_before(code, index) => {
                index = raw_string_end(bytes, index)?;
                continue;
            }
            b'"' => {
                index = quoted_end(bytes, index);
                continue;
            }
            b'\'' if index == 0 || !bytes[index - 1].is_ascii_hexdigit() => {
                index = quoted_end(bytes, index);
                continue;
            }
            _ => {}
        }
        if stack.is_empty() {
            return None;
        }
        index += 1;
    }
    None
}

/// The arguments of a call, split at the commas outside brackets and strings.
fn split_args(inside: &str) -> Vec<&str> {
    let bytes = inside.as_bytes();
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth = depth.saturating_sub(1),
            b'"' => {
                index = quoted_end(bytes, index);
                continue;
            }
            b',' if depth == 0 => {
                args.push(&inside[start..index]);
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    if !inside[start..].trim().is_empty() || !args.is_empty() {
        args.push(&inside[start..]);
    }
    args
}

/// The value of an expression made only of adjacent string literals: `"a" "b"` is `ab`.
pub fn string_literal(expression: &str) -> Option<String> {
    let mut rest = expression.trim();
    if rest.is_empty() {
        return None;
    }
    let mut value = String::new();
    while !rest.is_empty() {
        let bytes = rest.as_bytes();
        let (piece, end) = if rest.starts_with("R\"") {
            let end = raw_string_end(bytes, 0)?;
            let open = rest.find('(')?;
            let delim = open - 2;
            (rest[open + 1..end - delim - 2].to_owned(), end)
        } else if bytes[0] == b'"' {
            let end = quoted_end(bytes, 0);
            if end < 2 || bytes[end - 1] != b'"' {
                return None;
            }
            (unescape(&rest[1..end - 1]), end)
        } else {
            return None;
        };
        value.push_str(&piece);
        rest = rest[end..].trim_start();
    }
    Some(value)
}

/// Every string literal in `code`, in order.
fn string_literals(code: &str) -> Vec<String> {
    let bytes = code.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            let end = quoted_end(bytes, index);
            if end >= index + 2 && bytes[end - 1] == b'"' {
                found.push(unescape(&code[index + 1..end - 1]));
            }
            index = end;
        } else {
            index += 1;
        }
    }
    found
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// `Type{ a, b }` or `{ a, b }`: what the braces hold.
fn braced(expression: &str) -> Option<&str> {
    let expression = expression.trim();
    let open = expression.find('{')?;
    let head = expression[..open].trim();
    if !head
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
    {
        return None;
    }
    let (inside, close) = enclosed(expression, open)?;
    (close == expression.len() - 1).then_some(inside)
}

/// `ConVar_ServerInfo | ConVar_ReadOnly`; `None` for anything else.
fn parse_flags(expression: &str) -> Option<Vec<ConvarFlag>> {
    let mut flags = Vec::new();
    for part in expression.split('|') {
        let flag = match part.trim() {
            "ConVar_None" => continue,
            "ConVar_Archive" => ConvarFlag::Archive,
            "ConVar_ServerInfo" => ConvarFlag::ServerInfo,
            "ConVar_Replicated" => ConvarFlag::Replicated,
            "ConVar_ReadOnly" => ConvarFlag::ReadOnly,
            "ConVar_UserPref" => ConvarFlag::UserPref,
            "ConVar_Internal" => ConvarFlag::Internal,
            "ConVar_ScriptRestricted" => ConvarFlag::ScriptRestricted,
            _ => return None,
        };
        flags.push(flag);
    }
    flags.sort();
    flags.dedup();
    Some(flags)
}

/// A whole number written as a decimal literal.
pub fn integer_literal(expression: &str) -> Option<i64> {
    let text = expression.trim().trim_end_matches(['u', 'U', 'l', 'L']);
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A number written as an integer or floating literal: `50.f`, `1.0`, `4`.
pub fn float_literal(expression: &str) -> Option<f64> {
    let text = expression.trim();
    let text = text.strip_suffix(['f', 'F']).unwrap_or(text);
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        || digits.bytes().filter(|b| *b == b'.').count() > 1
        || digits == "."
    {
        return None;
    }
    text.parse().ok()
}

/// A float as FXServer prints it: `std::to_string`, six decimals.
pub fn print_float(value: f64) -> String {
    format!("{value:.6}")
}

/// `fx::OneSyncState::On` is `On`.
pub fn last_segment(path: &str) -> &str {
    path.trim().rsplit("::").next().unwrap_or_default().trim()
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the statement holding `at` starts: past the last `;`, `{`, `}` or preprocessor line
/// before it.
fn statement_start(code: &str, at: usize) -> usize {
    let start = code[..at].rfind([';', '{', '}']).map_or(0, |end| end + 1);
    let mut line_end = code[..at].rfind('\n').unwrap_or(0);
    while line_end > start {
        let line_start = code[..line_end].rfind('\n').map_or(0, |newline| newline + 1);
        if code[line_start..line_end].trim_start().starts_with('#') {
            return line_end + 1;
        }
        line_end = line_start.saturating_sub(1);
        if line_start == 0 {
            break;
        }
    }
    start
}

/// `auto var = …` or `m_var[0] = …`: the variable the statement assigns.
fn assigned_variable(statement: &str) -> Option<String> {
    let equals = statement.rfind('=')?;
    if statement[equals + 1..].trim_start().starts_with('=')
        || statement[..equals].ends_with(['=', '!', '<', '>'])
    {
        return None;
    }
    let target = statement[..equals].trim_end();
    let start = target
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || "_[]".contains(c)))
        .map_or(0, |at| at + 1);
    let name = &target[start..];
    (!name.is_empty() && !name.as_bytes()[0].is_ascii_digit()).then(|| name.to_owned())
}

/// The truth of each line under the `#if` blocks around it, as FXServer is built: `IS_FXSERVER`
/// defined, the client games' macros not, `_WIN32` on Windows only.
fn conditions(code: &str) -> Vec<LineState> {
    struct Block {
        branch: Truth,
        before: Vec<Truth>,
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut lines = Vec::new();
    for line in code.lines() {
        let enclosing = blocks
            .iter()
            .fold(Truth::True, |truth, block| and(truth, block.branch));
        lines.push(LineState { truth: enclosing });
        let Some(directive) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let directive = directive.trim_start();
        let (word, rest) = directive
            .split_once(|c: char| c.is_whitespace())
            .unwrap_or((directive, ""));
        match word {
            "if" | "ifdef" | "ifndef" => {
                let truth = match word {
                    "ifdef" => macro_truth(rest.trim()),
                    "ifndef" => not(macro_truth(rest.trim())),
                    _ => condition(rest),
                };
                blocks.push(Block {
                    branch: truth,
                    before: vec![truth],
                });
            }
            "elif" => {
                if let Some(block) = blocks.last_mut() {
                    let truth = condition(rest);
                    block.branch = and(not(any(&block.before)), truth);
                    block.before.push(truth);
                }
            }
            "else" => {
                if let Some(block) = blocks.last_mut() {
                    block.branch = not(any(&block.before));
                }
            }
            "endif" => {
                blocks.pop();
            }
            _ => {}
        }
    }
    lines
}

fn macro_truth(name: &str) -> Truth {
    match name {
        "IS_FXSERVER" => Truth::True,
        "GTA_FIVE" | "IS_RDR3" | "GTA_NY" => Truth::False,
        "_WIN32" => Truth::Windows,
        _ => Truth::Unknown,
    }
}

/// `X`, `!X`, `defined(X)`, `defined X`, `!defined(X)`, `0` and `1`; anything else is unknown.
fn condition(text: &str) -> Truth {
    let text = text.trim();
    if let Some(inner) = text.strip_prefix('(').and_then(|inner| inner.strip_suffix(')'))
        && !inner.contains(['(', ')'])
    {
        return condition(inner);
    }
    if let Some(rest) = text.strip_prefix('!') {
        return not(condition(rest));
    }
    if let Some(rest) = text.strip_prefix("defined") {
        let name = rest.trim().trim_start_matches('(').trim_end_matches(')').trim();
        return if name.bytes().all(is_ident) && !name.is_empty() {
            macro_truth(name)
        } else {
            Truth::Unknown
        };
    }
    match text {
        "0" => Truth::False,
        "1" => Truth::True,
        name if !name.is_empty() && name.bytes().all(is_ident) => macro_truth(name),
        _ => Truth::Unknown,
    }
}

fn not(truth: Truth) -> Truth {
    match truth {
        Truth::True => Truth::False,
        Truth::False => Truth::True,
        Truth::Windows => Truth::Linux,
        Truth::Linux => Truth::Windows,
        Truth::Unknown => Truth::Unknown,
    }
}

fn and(a: Truth, b: Truth) -> Truth {
    match (a, b) {
        (Truth::False, _)
        | (_, Truth::False)
        | (Truth::Windows, Truth::Linux)
        | (Truth::Linux, Truth::Windows) => Truth::False,
        (Truth::True, other) | (other, Truth::True) => other,
        (Truth::Unknown, _) | (_, Truth::Unknown) => Truth::Unknown,
        (Truth::Windows, Truth::Windows) => Truth::Windows,
        (Truth::Linux, Truth::Linux) => Truth::Linux,
    }
}

/// Whether one of the branches taken before holds.
fn any(before: &[Truth]) -> Truth {
    let has = |wanted| before.contains(&wanted);
    if has(Truth::True) || (has(Truth::Windows) && has(Truth::Linux)) {
        Truth::True
    } else if has(Truth::Unknown) {
        Truth::Unknown
    } else if has(Truth::Windows) {
        Truth::Windows
    } else if has(Truth::Linux) {
        Truth::Linux
    } else {
        Truth::False
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declarations(code: &str) -> Vec<Declaration> {
        read("code/components/x/src/X.cpp", code).0.declarations
    }

    #[test]
    fn each_form_of_declaration_is_read_with_its_flags_and_default() {
        let code = r#"
static ConVar<int> developerVariable(GetDefaultContext(), "developer", ConVar_Archive | ConVar_UserPref, 0, &gConsole->useDeveloper);
		m_hostname = instance->AddVariable<std::string>("sv_hostname", ConVar_ServerInfo, "default FXServer");
		m_returnClientsListInGetStatus = server->GetInstance()->template AddVariable<bool>("sv_returnClientsListInGetStatus", ConVar_None, true);
	static auto gamename = std::make_shared<ConVar<GameName>>("gamename", ConVar_ServerInfo, GameName::GTA5);
	mumble_logLevel = std::make_shared<ConVar<int>>("mumble_logLevel", ConVar_None, 0);
	auto conVar = new ConVar<bool>("svgui_toggle", ConVar_None, false, &module->toggleVar);
		auto enforceGameBuildVar = instance->AddVariable<fx::GameBuild>("sv_enforceGameBuild", ConVar_ReadOnly | ConVar_ServerInfo, xbr::GetMandatedDefaultGTA5BuildString(), &g_enforcedGameBuild);
"#;

        let found = declarations(code);

        let summary: Vec<_> = found
            .iter()
            .map(|d| {
                (
                    d.name.as_str(),
                    d.cpp_type.as_str(),
                    d.flags.clone(),
                    d.default.as_deref(),
                    d.line,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "developer",
                    "int",
                    Some(vec![ConvarFlag::Archive, ConvarFlag::UserPref]),
                    Some("0"),
                    2
                ),
                (
                    "sv_hostname",
                    "std::string",
                    Some(vec![ConvarFlag::ServerInfo]),
                    Some("\"default FXServer\""),
                    3
                ),
                (
                    "sv_returnClientsListInGetStatus",
                    "bool",
                    Some(vec![]),
                    Some("true"),
                    4
                ),
                (
                    "gamename",
                    "GameName",
                    Some(vec![ConvarFlag::ServerInfo]),
                    Some("GameName::GTA5"),
                    5
                ),
                ("mumble_logLevel", "int", Some(vec![]), Some("0"), 6),
                ("svgui_toggle", "bool", Some(vec![]), Some("false"), 7),
                (
                    "sv_enforceGameBuild",
                    "fx::GameBuild",
                    Some(vec![ConvarFlag::ServerInfo, ConvarFlag::ReadOnly]),
                    Some("xbr::GetMandatedDefaultGTA5BuildString()"),
                    8
                ),
            ]
        );
    }

    #[test]
    fn members_pointers_helpers_and_dynamic_names_declare_nothing() {
        let code = r#"
		std::shared_ptr<ConVar<std::string>> m_listingIpOverride;
	ConVar<int>* ivVar;
	fx::ScriptEngine::RegisterNativeHandler("GET_CONVAR_INT", GetConVar<int>);
		inline std::shared_ptr<ConVar<TVar>> AddVariable(const TArgs&... args)
		{
			return std::make_shared<ConVar<TVar>>(context.GetRef(), args...);
		}
	auto conVar = new ConVar<bool>(module->toggleName, ConVar_None, false, &module->toggleVar);
		: rateVar(console, fmt::sprintf("rateLimiter_%s_rate", key), ConVar_None, defaults.rate),
"#;

        let (found, _) = read("code/components/x/src/X.cpp", code);

        assert_eq!(found.declarations, []);
        assert_eq!(found.skipped.len(), 1, "{:?}", found.skipped);
        assert!(
            found.skipped[0].starts_with("9: ConVar<bool>(module->toggleName"),
            "{:?}",
            found.skipped
        );
    }

    #[test]
    fn a_declaration_split_over_lines_and_commented_out_ones_are_told_apart() {
        let code = "\
// auto off = instance->AddVariable<bool>(\"sv_commentedOut\", ConVar_None, false);
/* instance->AddVariable<int>(\"sv_blockComment\", ConVar_None, 1); */
		static ConVar<uint16_t> svHttpHandlerConnectionTimeout(console::GetDefaultContext(),
			\"sv_httpHandlerConnectionTimeoutSeconds\", ConVar_None, 300,
			httpServerManager->GetHandlerConnectionTimeoutSeconds());
";

        let found = declarations(code);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "sv_httpHandlerConnectionTimeoutSeconds");
        assert_eq!(found[0].cpp_type, "uint16_t");
        assert_eq!(found[0].default.as_deref(), Some("300"));
        assert_eq!(found[0].line, 3);
    }

    #[test]
    fn constraints_on_the_declared_variable_give_its_range() {
        let code = "\
		auto maxVarianceVar = instance->AddVariable<int>(\"sv_authMaxVariance\", ConVar_None, 5);
		maxVarianceVar->GetHelper()->SetConstraints(1, 5);
		auto other = instance->AddVariable<int>(\"sv_authMinTrust\", ConVar_None, 1);
";

        let found = declarations(code);

        assert_eq!(found[0].range, Some(ConvarRange { min: 1, max: 5 }));
        assert_eq!(found[1].range, None);
    }

    #[test]
    fn the_comment_above_or_beside_a_declaration_is_its_description() {
        let code = "\
		// list of space-separated endpoints that can but don't have to include a port
		// for example: sv_endpoints \"123.123.123.123 124.124.124.124\"
		auto srvEndpoints = instance->AddVariable<std::string>(\"sv_endpoints\", ConVar_None, \"\");
		auto lanVar = instance->AddVariable<bool>(\"sv_lan\", ConVar_ServerInfo, false);

		// auto x = instance->AddVariable<bool>(\"y\", ConVar_None, false);
		auto pure = instance->AddVariable<int>(\"sv_pureLevel\", ConVar_ServerInfo, 0); // the client's pure mode
";

        let found = declarations(code);

        assert_eq!(
            found[0].description.as_deref(),
            Some(
                "list of space-separated endpoints that can but don't have to include a port\n\
                 for example: sv_endpoints \"123.123.123.123 124.124.124.124\""
            )
        );
        assert_eq!(found[1].description, None);
        assert_eq!(found[2].description.as_deref(), Some("the client's pure mode"));
    }

    #[test]
    fn flags_that_are_not_plain_convar_flags_are_unknown() {
        let found = declarations("auto v = instance->AddVariable<bool>(\"sv_x\", someFlags, false);");

        assert_eq!(found[0].flags, None);
    }

    #[test]
    fn if_blocks_keep_the_server_side_and_mark_each_platform() {
        let code = "\
#ifdef IS_FXSERVER
static ConVar<bool> a(\"sv_serverSide\", ConVar_None, false);
#else
static ConVar<bool> b(\"cl_clientSide\", ConVar_None, false);
#endif
#ifdef _WIN32
static ConVar<bool> c(\"sv_windowsSide\", ConVar_None, false);
#elif defined(GTA_FIVE)
static ConVar<bool> d(\"cl_five\", ConVar_None, false);
#else
static ConVar<bool> e(\"sv_linuxSide\", ConVar_None, false);
#endif
#if !defined(IS_RDR3) && defined(SOMETHING)
static ConVar<bool> f(\"sv_unknownSide\", ConVar_None, false);
#endif
";

        let (found, _) = read("code/components/x/src/X.cpp", code);

        let names: Vec<_> = found
            .declarations
            .iter()
            .map(|d| (d.name.as_str(), d.platform))
            .collect();
        assert_eq!(
            names,
            [
                ("sv_serverSide", None),
                ("sv_windowsSide", Some(ConvarPlatform::Windows)),
                ("sv_linuxSide", Some(ConvarPlatform::Linux)),
                ("sv_unknownSide", None),
            ]
        );
        assert_eq!(found.unknown_conditions, [14]);
    }

    #[test]
    fn console_types_name_their_values_and_the_other_spellings_they_read() {
        let code = r#"
template<>
struct ConsoleArgumentType<fx::OneSyncState>
{
	static std::string Unparse(const fx::OneSyncState& input)
	{
		switch (input)
		{
			case fx::OneSyncState::Off:
				return "off";
			case fx::OneSyncState::On:
				return "on";
			case fx::OneSyncState::Legacy:
				return "legacy";
			default:
				return "unk";
		}
	}

	static bool Parse(const std::string& input, fx::OneSyncState* out)
	{
		if (_stricmp(input.c_str(), "on") == 0 || _stricmp(input.c_str(), "true") == 0)
		{
			*out = fx::OneSyncState::On;
			return true;
		}
		else if (_stricmp(input.c_str(), "legacy") == 0)
		{
			*out = fx::OneSyncState::Legacy;
			return true;
		}
		else if (_stricmp(input.c_str(), "off") == 0 || _stricmp(input.c_str(), "false") == 0)
		{
			*out = fx::OneSyncState::Off;
			return true;
		}

		return false;
	}
};
"#;

        let (found, _) = read("code/components/x/include/OneSyncVars.h", code);

        let onesync = &found.types["OneSyncState"];
        assert_eq!(onesync.printed["On"], "on");
        assert_eq!(onesync.printed.len(), 3);
        assert_eq!(
            onesync.parsed,
            [
                ("on".to_owned(), vec!["on".to_owned(), "true".to_owned()]),
                ("legacy".to_owned(), vec!["legacy".to_owned()]),
                ("off".to_owned(), vec!["off".to_owned(), "false".to_owned()]),
            ]
        );
        assert!(!onesync.numbers);
    }

    #[test]
    fn a_type_that_reads_numbers_and_names_says_so() {
        let code = r#"
struct ConsoleArgumentType<fx::GameBuild>
{
	static std::string Unparse(const fx::GameBuild& input)
	{
		return input;
	}

	static bool Parse(const std::string& input, fx::GameBuild* out)
	{
		std::string inputStr = input;
		try
		{
			if (auto intString = std::to_string(std::stoi(inputStr)); intString == inputStr)
			{
				*out = intString;
				return true;
			}
		}
		catch (std::invalid_argument&)
		{
		}
		if (inputStr.find("mp") != 0)
		{
			inputStr = "mp" + inputStr;
		}
		if (_stricmp(inputStr.c_str(), "mpchristmas2018") == 0)
		{
			*out = "1604";
			return true;
		}
		else if (_stricmp(inputStr.c_str(), "mpsum") == 0)
		{
			*out = "2060";
			return true;
		}
		return false;
	}
};
"#;

        let (found, _) = read("code/components/x/include/GameBuilds.h", code);

        let builds = &found.types["GameBuild"];
        assert!(builds.numbers);
        assert_eq!(
            builds.parsed,
            [
                ("1604".to_owned(), vec!["mpchristmas2018".to_owned()]),
                ("2060".to_owned(), vec!["mpsum".to_owned()]),
            ]
        );
    }

    #[test]
    fn reads_by_name_and_the_list_fxserver_takes_before_it_starts() {
        let code = r#"
static std::set<std::string, console::IgnoreCaseLess> setList =
{
	"onesync",
	"sv_licenseKey",
};
	auto var = consoleCtx->GetVariableManager()->FindEntryRaw("sv_licenseKeyToken");
	R"(\gametype\)" + server->GetVariable("gametype") + R"(\mapname\)";
	auto dynamic = manager->FindEntryRaw(name);
"#;

        let (found, _) = read("code/components/citizen-server-main/src/ServerInstance.cpp", code);

        let reads: Vec<_> = found.reads.iter().map(|r| (r.name.as_str(), r.line)).collect();
        assert_eq!(
            reads,
            [
                ("onesync", 4),
                ("sv_licenseKey", 5),
                ("sv_licenseKeyToken", 7),
                ("gametype", 8)
            ]
        );
    }

    #[test]
    fn rate_limiters_give_their_key_and_defaults() {
        let code = r#"
		static auto limiter = instance->GetComponent<fx::PeerAddressRateLimiterStore>()->GetRateLimiter("http_info", fx::RateLimiterDefaults{ 4.0, 10.0 });
	auto eventLimiter = store->GetRateLimiter("netEvent", fx::RateLimiterDefaults{ 50.f, 200.f });
	auto byResource = store->GetRateLimiter("http_" + m_resource->GetName(), fx::RateLimiterDefaults{ 10.0, 25.0 });
	TRateLimiter* GetRateLimiter(const std::string& key, RateLimiterDefaults defaults)
"#;

        let (found, limiters) = read("code/components/x/src/X.cpp", code);

        let summary: Vec<_> = limiters
            .iter()
            .map(|l| (l.key.as_str(), l.rate.as_str(), l.burst.as_str(), l.line))
            .collect();
        assert_eq!(
            summary,
            [
                ("http_info", "4.000000", "10.000000", 2),
                ("netEvent", "50.000000", "200.000000", 3)
            ]
        );
        assert_eq!(found.skipped.len(), 1);
        assert!(found.skipped[0].starts_with("4: GetRateLimiter(\"http_\" +"));
    }

    #[test]
    fn raw_strings_and_quotes_in_comments_do_not_derail_the_reading() {
        let code = "\
	auto s = R\"(it's \"quoted\" // not a comment)\"; // it's a comment with a \" quote
	static ConVar<bool> v(\"sv_afterRaw\", ConVar_None, true);
";

        let found = declarations(code);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "sv_afterRaw");
        assert_eq!(found[0].line, 2);
    }

    #[test]
    fn literals_and_floats_read_as_fxserver_prints_them() {
        assert_eq!(string_literal(r#""a" "b\"c""#).as_deref(), Some("ab\"c"));
        assert_eq!(string_literal(r#""FXServer-" GIT_DESCRIPTION"#), None);
        assert_eq!(string_literal("kDefaultServerList"), None);
        assert_eq!(integer_literal("1024 * 100"), None);
        assert_eq!(integer_literal("-1"), Some(-1));
        assert_eq!(float_literal("50.f"), Some(50.0));
        assert_eq!(float_literal("defaults.rate"), None);
        assert_eq!(print_float(1.5), "1.500000");
        assert_eq!(
            braced(r#"NetworkList{ "10.0.0.0/8 127.0.0.0/8" }"#),
            Some(r#" "10.0.0.0/8 127.0.0.0/8" "#)
        );
    }
}
