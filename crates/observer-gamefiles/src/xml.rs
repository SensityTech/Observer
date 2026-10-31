//! A small XML reader for the `.meta` files resources ship. It reads what those files are written
//! with (the declaration, comments, CDATA sections, processing instructions, attributes, the five
//! named entities and character references, a UTF-8 BOM) and keeps where each element lies in the
//! text, so that a caller can cut an element out and leave every other byte as it was.
//!
//! It is not a general XML parser. A DOCTYPE is refused, so no file declares entities of its own,
//! and a namespace prefix is part of a name like any other character. Malformed text is an error
//! with the line it is on, never a panic, and the depth, the elements, the nodes and the attributes
//! of one element are capped, so a hostile file costs memory in proportion to its size at most.

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;

/// How deep elements may nest, the root being at depth 1.
pub const MAX_DEPTH: usize = 256;
/// The most elements one document holds.
pub const MAX_ELEMENTS: usize = 1 << 18;
/// The most nodes inside the root: elements, text runs, CDATA sections, comments and processing
/// instructions, the root included.
pub const MAX_NODES: usize = 1 << 20;
/// The most attributes one element carries.
pub const MAX_ATTRIBUTES: usize = 256;
/// The longest reference read between `&` and `;`: `#x10FFFF` and `quot` fit.
const MAX_REFERENCE: usize = 10;
/// The most characters of a name an error quotes.
const QUOTED_CHARS: usize = 64;

/// What is wrong with a text and where.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {problem}")]
pub struct Error {
    /// The line the problem is on, from 1.
    pub line: usize,
    pub problem: Problem,
}

/// What makes a text unreadable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    #[error("no root element")]
    NoRoot,
    #[error("text outside the root element")]
    TextOutsideRoot,
    #[error("a second root element `<{0}>`")]
    SecondRoot(String),
    #[error("`<{0}>` is never closed")]
    Unclosed(String),
    #[error("`</{found}>` closes `<{open}>`")]
    Mismatched { open: String, found: String },
    #[error("`</{0}>` closes no element")]
    StrayEnd(String),
    #[error("unterminated {0}")]
    Unterminated(Construct),
    #[error("malformed tag: {0}")]
    MalformedTag(&'static str),
    #[error("attribute `{0}` given twice")]
    DuplicateAttribute(String),
    #[error("`<` inside an attribute value")]
    LessThanInValue,
    #[error("`&{0}` is not a reference XML defines")]
    BadReference(String),
    #[error("a DOCTYPE, which is not read")]
    Doctype,
    #[error("`<!` that starts no comment or CDATA section")]
    UnknownMarkup,
    #[error("elements nested deeper than {}", MAX_DEPTH)]
    TooDeep,
    #[error("more than {} elements", MAX_ELEMENTS)]
    TooManyElements,
    #[error("more than {} nodes", MAX_NODES)]
    TooManyNodes,
    #[error("more than {} attributes on one element", MAX_ATTRIBUTES)]
    TooManyAttributes,
}

/// The kinds of markup that can be left open at the end of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Construct {
    Comment,
    CData,
    Instruction,
    Tag,
    Value,
}

impl fmt::Display for Construct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Comment => "comment",
            Self::CData => "CDATA section",
            Self::Instruction => "processing instruction",
            Self::Tag => "tag",
            Self::Value => "attribute value",
        })
    }
}

/// A parsed text: its elements in the order their start tags come, the root first.
#[derive(Debug, Clone)]
pub struct Document<'a> {
    text: &'a str,
    elements: Vec<Data>,
}

#[derive(Debug, Clone)]
struct Data {
    name: Range<usize>,
    /// Name and value of each attribute, the value without its quotes and undecoded.
    attributes: Vec<(Range<usize>, Range<usize>)>,
    span: Range<usize>,
    content: Range<usize>,
    line: usize,
    parent: Option<usize>,
    nodes: Vec<Slot>,
}

#[derive(Debug, Clone)]
enum Slot {
    Element(usize),
    Text(Range<usize>),
    CData(Range<usize>),
    Comment(Range<usize>),
    Instruction(Range<usize>),
}

/// One element of a [`Document`].
#[derive(Clone, Copy)]
pub struct Element<'d, 'a> {
    document: &'d Document<'a>,
    index: usize,
}

/// What an element holds, in the order it is written.
#[derive(Debug, Clone, Copy)]
pub enum Node<'d, 'a> {
    Element(Element<'d, 'a>),
    /// Character data as written: [`decode`] gives its value.
    Text(&'a str),
    /// The inside of a CDATA section, which is its value as it is.
    CData(&'a str),
    /// The inside of a comment.
    Comment(&'a str),
    /// The inside of a processing instruction, its target first.
    Instruction(&'a str),
}

/// An attribute as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribute<'a> {
    pub name: &'a str,
    /// The value between the quotes, references undecoded.
    pub raw: &'a str,
}

impl<'a> Attribute<'a> {
    /// The value with its references decoded.
    pub fn value(&self) -> Cow<'a, str> {
        decode(self.raw)
    }
}

impl<'a> Document<'a> {
    /// Reads `text`, which may start with a BOM. Spans count bytes of `text`, the BOM included.
    pub fn parse(text: &'a str) -> Result<Self, Error> {
        Parser::new(text).document()
    }

    /// The text the document was read from.
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The root element: every document has one.
    pub fn root(&self) -> Element<'_, 'a> {
        Element {
            document: self,
            index: 0,
        }
    }

    /// Every element, in the order their start tags come.
    pub fn elements(&self) -> impl Iterator<Item = Element<'_, 'a>> {
        (0..self.elements.len()).map(|index| Element {
            document: self,
            index,
        })
    }

    /// How many elements the document holds, the root included.
    pub fn element_count(&self) -> usize {
        self.elements.len()
    }
}

impl<'d, 'a> Element<'d, 'a> {
    fn data(&self) -> &'d Data {
        let document: &'d Document<'a> = self.document;
        &document.elements[self.index]
    }

    fn slice(&self, range: &Range<usize>) -> &'a str {
        let text: &'a str = self.document.text;
        &text[range.clone()]
    }

    /// The name as written in the start tag.
    pub fn name(&self) -> &'a str {
        self.slice(&self.data().name)
    }

    /// From the `<` of the start tag to past the `>` of the end tag, or of the start tag when it
    /// closes itself.
    pub fn span(&self) -> Range<usize> {
        self.data().span.clone()
    }

    /// Between the start tag and the end tag; empty, at the end of the span, when the element
    /// closes itself.
    pub fn content_span(&self) -> Range<usize> {
        self.data().content.clone()
    }

    /// The element as written, tags included.
    pub fn source(&self) -> &'a str {
        self.slice(&self.data().span)
    }

    /// The line its start tag begins on, from 1.
    pub fn line(&self) -> usize {
        self.data().line
    }

    /// The element this one is inside, `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        self.data().parent.map(|index| Self {
            document: self.document,
            index,
        })
    }

    /// The attributes, in the order they are written.
    pub fn attributes(&self) -> impl Iterator<Item = Attribute<'a>> + use<'d, 'a> {
        let element = *self;
        self.data().attributes.iter().map(move |(name, raw)| Attribute {
            name: element.slice(name),
            raw: element.slice(raw),
        })
    }

    /// The decoded value of the attribute `name`.
    pub fn attribute(&self, name: &str) -> Option<Cow<'a, str>> {
        self.attributes()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value())
    }

    /// What the element holds, in order.
    pub fn nodes(&self) -> impl Iterator<Item = Node<'d, 'a>> + use<'d, 'a> {
        let element = *self;
        self.data().nodes.iter().map(move |slot| match slot {
            Slot::Element(index) => Node::Element(Self {
                document: element.document,
                index: *index,
            }),
            Slot::Text(range) => Node::Text(element.slice(range)),
            Slot::CData(range) => Node::CData(element.slice(range)),
            Slot::Comment(range) => Node::Comment(element.slice(range)),
            Slot::Instruction(range) => Node::Instruction(element.slice(range)),
        })
    }

    /// The child elements, in order.
    pub fn children(&self) -> impl Iterator<Item = Self> + use<'d, 'a> {
        self.nodes().filter_map(|node| match node {
            Node::Element(child) => Some(child),
            _ => None,
        })
    }

    /// The first child element called `name`.
    pub fn child(&self, name: &str) -> Option<Self> {
        self.children().find(|child| child.name() == name)
    }

    /// The text directly inside the element, decoded, CDATA sections included, comments and child
    /// elements left out.
    pub fn text(&self) -> Cow<'a, str> {
        let mut pieces = self.nodes().filter_map(|node| match node {
            Node::Text(raw) => Some(decode(raw)),
            Node::CData(inside) => Some(Cow::Borrowed(inside)),
            _ => None,
        });
        let Some(first) = pieces.next() else {
            return Cow::Borrowed("");
        };
        let Some(second) = pieces.next() else {
            return first;
        };
        let mut text = first.into_owned();
        text.push_str(&second);
        for piece in pieces {
            text.push_str(&piece);
        }
        Cow::Owned(text)
    }
}

impl fmt::Debug for Element<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Element")
            .field("name", &self.name())
            .field("span", &self.span())
            .field("line", &self.line())
            .finish_non_exhaustive()
    }
}

/// The name of the root element, read from the start of `text` alone: what precedes it is checked,
/// nothing after it is. `None` when the text ends before the name does, so a cut text can be asked.
pub fn root_name(text: &str) -> Result<Option<&str>, Error> {
    let mut parser = Parser::new(text);
    let bytes = text.as_bytes();
    while parser.pos < bytes.len() {
        if bytes[parser.pos] != b'<' {
            parser.outside_text()?;
            continue;
        }
        let rest = &bytes[parser.pos..];
        if rest.starts_with(b"<!--") {
            parser.until(parser.pos + 4, b"-->", Construct::Comment)?;
        } else if rest.starts_with(b"<?") {
            parser.until(parser.pos + 2, b"?>", Construct::Instruction)?;
        } else if rest.starts_with(b"<!DOCTYPE") {
            return Err(parser.error_at(parser.pos, Problem::Doctype));
        } else if rest.starts_with(b"<![CDATA[") {
            return Err(parser.error_at(parser.pos, Problem::TextOutsideRoot));
        } else if rest.starts_with(b"<!") {
            return Err(parser.error_at(parser.pos, Problem::UnknownMarkup));
        } else if rest.starts_with(b"</") {
            let name = parser
                .name(parser.pos + 2)
                .unwrap_or(parser.pos + 2..parser.pos + 2);
            return Err(parser.error_at(parser.pos, Problem::StrayEnd(quote(&text[name]))));
        } else {
            let Some(name) = parser.name(parser.pos + 1) else {
                return Err(
                    parser.error_at(parser.pos, Problem::MalformedTag("`<` is not followed by a name"))
                );
            };
            return Ok((name.end < bytes.len()).then(|| &text[name]));
        }
    }
    Ok(None)
}

/// `raw` with its references decoded. A `&` that starts no reference XML defines is kept as it is;
/// [`Document::parse`] refuses those, so text it read never holds one.
pub fn decode(raw: &str) -> Cow<'_, str> {
    if !raw.contains('&') {
        return Cow::Borrowed(raw);
    }
    let mut decoded = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(ampersand) = rest.find('&') {
        decoded.push_str(&rest[..ampersand]);
        let after = &rest[ampersand + 1..];
        if let Some((length, character)) = reference(after) {
            decoded.push(character);
            rest = &after[length + 1..];
        } else {
            decoded.push('&');
            rest = after;
        }
    }
    decoded.push_str(rest);
    Cow::Owned(decoded)
}

/// The line, from 1, that byte `offset` of `bytes` is on.
pub fn line_of(bytes: &[u8], offset: usize) -> usize {
    1 + line_feeds(&bytes[..offset.min(bytes.len())])
}

fn line_feeds(bytes: &[u8]) -> usize {
    bytes.iter().map(|byte| usize::from(*byte == b'\n')).sum()
}

/// The reference at the start of `after`, the text that follows a `&`: the length of its name and
/// the character it stands for.
fn reference(after: &str) -> Option<(usize, char)> {
    let length = after
        .bytes()
        .take(MAX_REFERENCE + 1)
        .position(|byte| byte == b';')?;
    Some((length, character(&after[..length])?))
}

/// The character the reference `name` (between `&` and `;`) stands for.
fn character(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = if let Some(hex) = number.strip_prefix('x') {
                if hex.is_empty() || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return None;
                }
                u32::from_str_radix(hex, 16).ok()?
            } else {
                if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                number.parse().ok()?
            };
            // XML's `Char`: no other control character, no surrogate, no U+FFFE or U+FFFF.
            match code {
                0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x1_0000..=0x10_FFFF => {
                    char::from_u32(code)
                }
                _ => None,
            }
        }
    }
}

/// `name` cut to what an error quotes.
fn quote(name: &str) -> String {
    name.chars().take(QUOTED_CHARS).collect()
}

const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// Bytes above ASCII belong to characters that can be part of a name: a name ends at an ASCII
/// byte, so it is always cut on a character boundary.
const fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':') || byte >= 0x80
}

const fn is_name_byte(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    elements: Vec<Data>,
    /// The elements whose end tag is still to come, innermost last.
    open: Vec<usize>,
    nodes: usize,
    /// A position and its line, moved forward as elements are found.
    counted: (usize, usize),
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        let bom = if text.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        Self {
            text,
            bytes: text.as_bytes(),
            pos: bom,
            elements: Vec::new(),
            open: Vec::new(),
            nodes: 0,
            counted: (0, 1),
        }
    }

    fn document(mut self) -> Result<Document<'a>, Error> {
        while self.pos < self.bytes.len() {
            if self.bytes[self.pos] == b'<' {
                self.markup()?;
            } else if self.open.is_empty() {
                self.outside_text()?;
            } else {
                self.text_run()?;
            }
        }
        if let Some(&open) = self.open.last() {
            let element = &self.elements[open];
            let name = quote(&self.text[element.name.clone()]);
            return Err(self.error_at(element.span.start, Problem::Unclosed(name)));
        }
        if self.elements.is_empty() {
            return Err(self.error_at(self.bytes.len(), Problem::NoRoot));
        }
        Ok(Document {
            text: self.text,
            elements: self.elements,
        })
    }

    fn error_at(&self, offset: usize, problem: Problem) -> Error {
        Error {
            line: line_of(self.bytes, offset),
            problem,
        }
    }

    /// The line of `offset`, counted from where the last call stopped when it lies past it.
    fn line(&mut self, offset: usize) -> usize {
        let (at, line) = self.counted;
        let line = if offset >= at {
            line + line_feeds(&self.bytes[at..offset])
        } else {
            line_of(self.bytes, offset)
        };
        self.counted = (offset, line);
        line
    }

    fn skip_space(&self, mut at: usize) -> usize {
        while at < self.bytes.len() && is_space(self.bytes[at]) {
            at += 1;
        }
        at
    }

    /// The name that starts at `at`, if one does.
    fn name(&self, at: usize) -> Option<Range<usize>> {
        let first = *self.bytes.get(at)?;
        if !is_name_start(first) {
            return None;
        }
        let end = self.bytes[at + 1..]
            .iter()
            .position(|byte| !is_name_byte(*byte))
            .map_or(self.bytes.len(), |length| at + 1 + length);
        Some(at..end)
    }

    /// Moves past the next `terminator` from `from` and gives what lay between.
    fn until(&mut self, from: usize, terminator: &[u8], construct: Construct) -> Result<Range<usize>, Error> {
        match find(&self.bytes[from.min(self.bytes.len())..], terminator) {
            Some(length) => {
                let inside = from..from + length;
                self.pos = inside.end + terminator.len();
                Ok(inside)
            }
            None => Err(self.error_at(self.pos, Problem::Unterminated(construct))),
        }
    }

    /// Adds a node to the innermost open element. Nodes outside the root are not kept.
    fn push(&mut self, slot: Slot) -> Result<(), Error> {
        if self.nodes >= MAX_NODES {
            return Err(self.error_at(self.pos, Problem::TooManyNodes));
        }
        self.nodes += 1;
        if let Some(&parent) = self.open.last() {
            self.elements[parent].nodes.push(slot);
        }
        Ok(())
    }

    /// Text before or after the root, which may only be white space.
    fn outside_text(&mut self) -> Result<(), Error> {
        let start = self.pos;
        let end = self.bytes[start..]
            .iter()
            .position(|byte| *byte == b'<')
            .map_or(self.bytes.len(), |length| start + length);
        if let Some(length) = self.bytes[start..end].iter().position(|byte| !is_space(*byte)) {
            return Err(self.error_at(start + length, Problem::TextOutsideRoot));
        }
        self.pos = end;
        Ok(())
    }

    fn text_run(&mut self) -> Result<(), Error> {
        let start = self.pos;
        let end = self.bytes[start..]
            .iter()
            .position(|byte| *byte == b'<')
            .map_or(self.bytes.len(), |length| start + length);
        self.check_references(start..end)?;
        self.pos = end;
        self.push(Slot::Text(start..end))
    }

    /// Refuses a `&` in `range` that starts no reference XML defines.
    fn check_references(&self, range: Range<usize>) -> Result<(), Error> {
        let raw = &self.text[range.clone()];
        let mut from = 0;
        while let Some(found) = raw[from..].find('&') {
            let ampersand = from + found;
            let after = &raw[ampersand + 1..];
            let Some((length, _)) = reference(after) else {
                let shown: String = after
                    .chars()
                    .take_while(|character| !character.is_whitespace() && *character != '<')
                    .take(MAX_REFERENCE + 1)
                    .collect();
                return Err(self.error_at(range.start + ampersand, Problem::BadReference(shown)));
            };
            // Past the `;`.
            from = ampersand + 1 + length + 1;
        }
        Ok(())
    }

    fn markup(&mut self) -> Result<(), Error> {
        let rest = &self.bytes[self.pos..];
        if rest.starts_with(b"<!--") {
            let inside = self.until(self.pos + 4, b"-->", Construct::Comment)?;
            self.push(Slot::Comment(inside))
        } else if rest.starts_with(b"<![CDATA[") {
            if self.open.is_empty() {
                return Err(self.error_at(self.pos, Problem::TextOutsideRoot));
            }
            let inside = self.until(self.pos + 9, b"]]>", Construct::CData)?;
            self.push(Slot::CData(inside))
        } else if rest.starts_with(b"<!DOCTYPE") {
            Err(self.error_at(self.pos, Problem::Doctype))
        } else if rest.starts_with(b"<!") {
            Err(self.error_at(self.pos, Problem::UnknownMarkup))
        } else if rest.starts_with(b"<?") {
            let inside = self.until(self.pos + 2, b"?>", Construct::Instruction)?;
            self.push(Slot::Instruction(inside))
        } else if rest.starts_with(b"</") {
            self.end_tag()
        } else {
            self.start_tag()
        }
    }

    fn start_tag(&mut self) -> Result<(), Error> {
        let start = self.pos;
        let Some(name) = self.name(start + 1) else {
            return Err(self.error_at(start, Problem::MalformedTag("`<` is not followed by a name")));
        };
        let mut at = name.end;
        let mut attributes: Vec<(Range<usize>, Range<usize>)> = Vec::new();
        let closes_itself = loop {
            let spaced = self.bytes.get(at).copied().is_some_and(is_space);
            at = self.skip_space(at);
            match self.bytes.get(at) {
                None => return Err(self.error_at(start, Problem::Unterminated(Construct::Tag))),
                Some(b'>') => {
                    at += 1;
                    break false;
                }
                Some(b'/') if self.bytes.get(at + 1) == Some(&b'>') => {
                    at += 2;
                    break true;
                }
                Some(b'/') => return Err(self.error_at(at, Problem::MalformedTag("`/` not followed by `>`"))),
                Some(_) if !spaced => {
                    return Err(
                        self.error_at(at, Problem::MalformedTag("no white space before an attribute"))
                    );
                }
                Some(_) => {
                    let attribute = self.attribute(at)?;
                    if attributes.len() >= MAX_ATTRIBUTES {
                        return Err(self.error_at(at, Problem::TooManyAttributes));
                    }
                    let attribute_name = &self.bytes[attribute.0.clone()];
                    if attributes
                        .iter()
                        .any(|(other, _)| &self.bytes[other.clone()] == attribute_name)
                    {
                        let shown = quote(&self.text[attribute.0.clone()]);
                        return Err(self.error_at(at, Problem::DuplicateAttribute(shown)));
                    }
                    // Past the closing quote.
                    at = attribute.1.end + 1;
                    attributes.push(attribute);
                }
            }
        };

        if self.open.len() >= MAX_DEPTH {
            return Err(self.error_at(start, Problem::TooDeep));
        }
        if self.open.is_empty() && !self.elements.is_empty() {
            let shown = quote(&self.text[name.clone()]);
            return Err(self.error_at(start, Problem::SecondRoot(shown)));
        }
        if self.elements.len() >= MAX_ELEMENTS {
            return Err(self.error_at(start, Problem::TooManyElements));
        }
        let index = self.elements.len();
        self.push(Slot::Element(index))?;
        let line = self.line(start);
        self.elements.push(Data {
            name,
            attributes,
            span: start..at,
            content: at..at,
            line,
            parent: self.open.last().copied(),
            nodes: Vec::new(),
        });
        if !closes_itself {
            self.open.push(index);
        }
        self.pos = at;
        Ok(())
    }

    /// The attribute that starts at `at`: its name and its value between the quotes.
    fn attribute(&self, at: usize) -> Result<(Range<usize>, Range<usize>), Error> {
        let Some(name) = self.name(at) else {
            return Err(self.error_at(at, Problem::MalformedTag("expected an attribute name")));
        };
        let equals = self.skip_space(name.end);
        match self.bytes.get(equals) {
            Some(b'=') => {}
            None => return Err(self.error_at(at, Problem::Unterminated(Construct::Tag))),
            Some(_) => return Err(self.error_at(equals, Problem::MalformedTag("an attribute without `=`"))),
        }
        let opening = self.skip_space(equals + 1);
        let quote = match self.bytes.get(opening) {
            Some(&byte @ (b'"' | b'\'')) => byte,
            None => return Err(self.error_at(at, Problem::Unterminated(Construct::Tag))),
            Some(_) => {
                return Err(self.error_at(
                    opening,
                    Problem::MalformedTag("an attribute value without quotes"),
                ));
            }
        };
        let start = opening + 1;
        let Some(length) = self.bytes[start..].iter().position(|byte| *byte == quote) else {
            return Err(self.error_at(opening, Problem::Unterminated(Construct::Value)));
        };
        let value = start..start + length;
        if let Some(less) = self.bytes[value.clone()].iter().position(|byte| *byte == b'<') {
            return Err(self.error_at(start + less, Problem::LessThanInValue));
        }
        self.check_references(value.clone())?;
        Ok((name, value))
    }

    fn end_tag(&mut self) -> Result<(), Error> {
        let start = self.pos;
        let Some(name) = self.name(start + 2) else {
            return Err(self.error_at(start, Problem::MalformedTag("`</` is not followed by a name")));
        };
        let close = self.skip_space(name.end);
        match self.bytes.get(close) {
            Some(b'>') => {}
            None => return Err(self.error_at(start, Problem::Unterminated(Construct::Tag))),
            Some(_) => {
                return Err(self.error_at(close, Problem::MalformedTag("an end tag holds more than a name")));
            }
        }
        let Some(open) = self.open.pop() else {
            let shown = quote(&self.text[name]);
            return Err(self.error_at(start, Problem::StrayEnd(shown)));
        };
        let element = &self.elements[open];
        if self.bytes[element.name.clone()] != self.bytes[name.clone()] {
            let problem = Problem::Mismatched {
                open: quote(&self.text[element.name.clone()]),
                found: quote(&self.text[name]),
            };
            return Err(self.error_at(start, problem));
        }
        let element = &mut self.elements[open];
        element.content.end = start;
        element.span.end = close + 1;
        self.pos = close + 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(text: &str) -> Problem {
        Document::parse(text).unwrap_err().problem
    }

    fn names<'d, 'a: 'd>(elements: impl Iterator<Item = Element<'d, 'a>>) -> Vec<&'a str> {
        elements.map(|element| element.name()).collect()
    }

    const SAMPLE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
        <!-- leading comment -->\n\
        <Root kind='list'>\n\
        \t<Items>\n\
        \t\t<Item><name>first</name><size value=\"1\" /></Item>\n\
        \t\t<Item>\n\
        \t\t\t<name>second</name>\n\
        \t\t</Item>\n\
        \t</Items>\n\
        </Root>\n\
        <!-- trailing comment -->\n";

    #[test]
    fn a_document_gives_its_elements_in_order_with_where_they_lie() {
        let document = Document::parse(SAMPLE).unwrap();

        assert_eq!(
            names(document.elements()),
            ["Root", "Items", "Item", "name", "size", "Item", "name"]
        );
        let root = document.root();
        assert_eq!(root.line(), 3);
        assert!(root.source().starts_with("<Root kind='list'>"));
        assert!(root.source().ends_with("</Root>"));
        let items: Vec<_> = root.child("Items").unwrap().children().collect();
        assert_eq!(
            items[0].source(),
            "<Item><name>first</name><size value=\"1\" /></Item>"
        );
        assert_eq!(items[0].line(), 5);
        assert_eq!(items[1].line(), 6);
        assert_eq!(
            &SAMPLE[items[1].content_span()],
            "\n\t\t\t<name>second</name>\n\t\t"
        );
        assert_eq!(items[1].parent().unwrap().name(), "Items");
        assert!(root.parent().is_none());
    }

    #[test]
    fn a_self_closing_element_has_an_empty_content_at_its_end() {
        let document = Document::parse(SAMPLE).unwrap();
        let size = document
            .elements()
            .find(|element| element.name() == "size")
            .unwrap();

        assert_eq!(size.source(), "<size value=\"1\" />");
        assert_eq!(size.content_span(), size.span().end..size.span().end);
        assert_eq!(size.children().count(), 0);
        assert_eq!(size.text(), "");
    }

    #[test]
    fn a_bom_is_skipped_and_spans_count_its_bytes() {
        let text = "\u{feff}<?xml version='1.0'?>\r\n<a><b/></a>";
        let document = Document::parse(text).unwrap();

        let b = document.root().child("b").unwrap();
        assert_eq!(&text[b.span()], "<b/>");
        assert_eq!(b.span().start, text.find("<b/>").unwrap());
        assert_eq!(b.line(), 2);
    }

    #[test]
    fn attributes_are_read_with_either_quote_and_decoded() {
        let document = Document::parse("<a x = \"1 &amp; 2\" y='it&apos;s' z=\"&#65;&#x42;\"/>").unwrap();
        let root = document.root();

        let attributes: Vec<_> = root
            .attributes()
            .map(|attribute| (attribute.name, attribute.raw))
            .collect();
        assert_eq!(
            attributes,
            [("x", "1 &amp; 2"), ("y", "it&apos;s"), ("z", "&#65;&#x42;")]
        );
        assert_eq!(root.attribute("x").unwrap(), "1 & 2");
        assert_eq!(root.attribute("y").unwrap(), "it's");
        assert_eq!(root.attribute("z").unwrap(), "AB");
        assert_eq!(root.attribute("w"), None);
    }

    #[test]
    fn text_decodes_the_named_entities_and_character_references() {
        let document =
            Document::parse("<a>&lt;b&gt; &amp; &quot;c&quot; &apos;d&apos; &#233;&#xE9;</a>").unwrap();

        assert_eq!(document.root().text(), "<b> & \"c\" 'd' \u{e9}\u{e9}");
    }

    #[test]
    fn cdata_is_text_taken_as_written_and_comments_are_not_text() {
        let document =
            Document::parse("<a> one <!-- not <this> --><![CDATA[ <two> & ]]>three<b>four</b></a>").unwrap();
        let root = document.root();

        assert_eq!(root.text(), " one  <two> & three");
        let kinds: Vec<_> = root
            .nodes()
            .map(|node| match node {
                Node::Element(element) => format!("element {}", element.name()),
                Node::Text(raw) => format!("text {raw:?}"),
                Node::CData(inside) => format!("cdata {inside:?}"),
                Node::Comment(inside) => format!("comment {inside:?}"),
                Node::Instruction(inside) => format!("instruction {inside:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "text \" one \"",
                "comment \" not <this> \"",
                "cdata \" <two> & \"",
                "text \"three\"",
                "element b",
            ]
        );
    }

    #[test]
    fn instructions_and_comments_may_stand_anywhere_white_space_may() {
        let text =
            "\n  <?xml version='1.0'?><!--a--><?other x?>\n<a><?inside?><!--b--></a>\n<!--c--><?after?>\n";
        let document = Document::parse(text).unwrap();

        assert_eq!(document.root().name(), "a");
        assert_eq!(document.root().nodes().count(), 2);
    }

    #[test]
    fn names_may_hold_letters_beyond_ascii_dots_dashes_and_colons() {
        let document = Document::parse("<r\u{e9}sum\u{e9} ns:a-b.c='1'><x.y-z/></r\u{e9}sum\u{e9}>").unwrap();

        assert_eq!(document.root().name(), "r\u{e9}sum\u{e9}");
        assert_eq!(document.root().attribute("ns:a-b.c").unwrap(), "1");
        assert_eq!(names(document.root().children()), ["x.y-z"]);
    }

    #[test]
    fn the_root_name_is_read_from_the_start_alone() {
        assert_eq!(root_name(SAMPLE), Ok(Some("Root")));
        assert_eq!(root_name("\u{feff}<!-- x --><Root><unclosed"), Ok(Some("Root")));
        assert_eq!(root_name("<?xml version='1.0'?>\n<Roo"), Ok(None));
        assert_eq!(
            root_name("<?xml version='1.0'?>\n<!-- cut"),
            Err(Error {
                line: 2,
                problem: Problem::Unterminated(Construct::Comment),
            })
        );
        assert_eq!(root_name("  "), Ok(None));
        assert_eq!(
            root_name("garbage <a/>").unwrap_err().problem,
            Problem::TextOutsideRoot
        );
        assert_eq!(
            root_name("<!DOCTYPE a><a/>").unwrap_err().problem,
            Problem::Doctype
        );
    }

    #[test]
    fn decoding_keeps_what_is_no_reference() {
        assert_eq!(decode("a & b"), "a & b");
        assert_eq!(decode("&unknown; &#xZZ; &#; &amp;"), "&unknown; &#xZZ; &#; &");
        assert!(matches!(decode("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn tags_that_do_not_match_are_refused_with_their_line() {
        assert_eq!(
            Document::parse("<a>\n<b>\n</a>").unwrap_err(),
            Error {
                line: 3,
                problem: Problem::Mismatched {
                    open: "b".to_owned(),
                    found: "a".to_owned(),
                },
            }
        );
        assert_eq!(
            Document::parse("<a>\n<b></b>").unwrap_err(),
            Error {
                line: 1,
                problem: Problem::Unclosed("a".to_owned()),
            }
        );
        assert_eq!(problem("<a></a></b>"), Problem::StrayEnd("b".to_owned()));
        assert_eq!(problem("<a/><b/>"), Problem::SecondRoot("b".to_owned()));
    }

    #[test]
    fn white_space_inside_tags_is_allowed_where_xml_allows_it() {
        let document = Document::parse("<a  x = '1'\n\ty=\"2\" ><b\n/></a\n>").unwrap();

        assert_eq!(document.root().attributes().count(), 2);
        assert_eq!(names(document.root().children()), ["b"]);
    }

    #[test]
    fn unterminated_markup_is_refused() {
        assert_eq!(problem("<a><!-- open"), Problem::Unterminated(Construct::Comment));
        assert_eq!(
            problem("<a><![CDATA[ open"),
            Problem::Unterminated(Construct::CData)
        );
        assert_eq!(
            problem("<a><?pi open"),
            Problem::Unterminated(Construct::Instruction)
        );
        assert_eq!(problem("<a><b"), Problem::Unterminated(Construct::Tag));
        assert_eq!(problem("<a><b x"), Problem::Unterminated(Construct::Tag));
        assert_eq!(problem("<a><b x="), Problem::Unterminated(Construct::Tag));
        assert_eq!(problem("<a><b x='1"), Problem::Unterminated(Construct::Value));
        assert_eq!(problem("<a></a"), Problem::Unterminated(Construct::Tag));
    }

    #[test]
    fn malformed_tags_and_attributes_are_refused() {
        assert!(matches!(problem("<a>< b/></a>"), Problem::MalformedTag(_)));
        assert!(matches!(problem("<a x='1'y='2'/>"), Problem::MalformedTag(_)));
        assert!(matches!(problem("<a x/>"), Problem::MalformedTag(_)));
        assert!(matches!(problem("<a x=1/>"), Problem::MalformedTag(_)));
        assert!(matches!(problem("<a / >"), Problem::MalformedTag(_)));
        assert!(matches!(problem("<a></a b>"), Problem::MalformedTag(_)));
        assert!(matches!(problem("<a></>"), Problem::MalformedTag(_)));
        assert_eq!(
            problem("<a x='1' x=\"2\"/>"),
            Problem::DuplicateAttribute("x".to_owned())
        );
        assert_eq!(problem("<a x='<'/>"), Problem::LessThanInValue);
    }

    #[test]
    fn references_xml_does_not_define_are_refused() {
        assert_eq!(
            problem("<a>&nbsp;</a>"),
            Problem::BadReference("nbsp;".to_owned())
        );
        assert_eq!(
            problem("<a>fish & chips</a>"),
            Problem::BadReference(String::new())
        );
        assert_eq!(problem("<a>&#0;</a>"), Problem::BadReference("#0;".to_owned()));
        assert_eq!(
            problem("<a>&#xD800;</a>"),
            Problem::BadReference("#xD800;".to_owned())
        );
        assert_eq!(
            problem("<a>&#x110000;</a>"),
            Problem::BadReference("#x110000;".to_owned())
        );
        assert_eq!(
            problem(&format!("<a>&#{}65;</a>", "0".repeat(12))),
            Problem::BadReference(format!("#{}", "0".repeat(10))),
            "a reference longer than any XML defines is not read to its end"
        );
        assert_eq!(
            problem("<a>&#x+41;</a>"),
            Problem::BadReference("#x+41;".to_owned())
        );
        assert_eq!(
            problem("<a x='&bad;'/>"),
            Problem::BadReference("bad;".to_owned())
        );
    }

    #[test]
    fn text_outside_the_root_and_doctypes_are_refused() {
        assert_eq!(problem("x<a/>"), Problem::TextOutsideRoot);
        assert_eq!(problem("<a/>x"), Problem::TextOutsideRoot);
        assert_eq!(problem("<![CDATA[x]]><a/>"), Problem::TextOutsideRoot);
        assert_eq!(problem("<!DOCTYPE a><a/>"), Problem::Doctype);
        assert_eq!(problem("<a><!ELEMENT a></a>"), Problem::UnknownMarkup);
        assert_eq!(problem(""), Problem::NoRoot);
        assert_eq!(
            problem("\u{feff}<?xml version='1.0'?><!-- only -->"),
            Problem::NoRoot
        );
    }

    #[test]
    fn nesting_past_the_limit_is_refused() {
        let fits = format!("{}{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH));
        assert!(Document::parse(&fits).is_ok());

        let deeper = format!("{}<a/>{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH));
        assert_eq!(problem(&deeper), Problem::TooDeep);
    }

    #[test]
    fn elements_nodes_and_attributes_past_their_limits_are_refused() {
        let elements = format!("<r>{}</r>", "<a/>".repeat(MAX_ELEMENTS));
        assert_eq!(problem(&elements), Problem::TooManyElements);

        let nodes = format!("<r>{}</r>", "<!---->".repeat(MAX_NODES));
        assert_eq!(problem(&nodes), Problem::TooManyNodes);

        let attributes = (0..=MAX_ATTRIBUTES)
            .map(|index| format!(" a{index}='1'"))
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(problem(&format!("<r{attributes}/>")), Problem::TooManyAttributes);
    }

    #[test]
    fn every_cut_of_a_document_reads_or_fails_without_panicking() {
        for (end, _) in SAMPLE.char_indices() {
            let cut = &SAMPLE[..end];
            let _ = Document::parse(cut);
            let _ = root_name(cut);
        }
        let with_bom = format!("\u{feff}{SAMPLE}");
        for (end, _) in with_bom.char_indices() {
            let _ = Document::parse(&with_bom[..end]);
        }
    }

    #[test]
    fn scrambled_markup_reads_or_fails_without_panicking() {
        // A fixed pseudo-random walk replacing bytes of the sample with markup characters.
        let alphabet = b"<>/!?-[]&;#x='\" \nAa";
        let mut state: usize = 0x2545_f491;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2000 {
            let mut bytes = SAMPLE.as_bytes().to_vec();
            for _ in 0..=next() % 4 {
                let at = next() % bytes.len();
                bytes[at] = alphabet[next() % alphabet.len()];
            }
            let text = String::from_utf8(bytes).unwrap();
            if let Ok(document) = Document::parse(&text) {
                for element in document.elements() {
                    let _ = (element.source(), element.text(), element.attributes().count());
                }
            }
            let _ = root_name(&text);
        }
    }

    #[test]
    fn line_numbers_count_line_feeds() {
        assert_eq!(line_of(b"a\nb\r\nc", 0), 1);
        assert_eq!(line_of(b"a\nb\r\nc", 2), 2);
        assert_eq!(line_of(b"a\nb\r\nc", 5), 3);
        assert_eq!(line_of(b"a\nb", 99), 2);
    }
}
