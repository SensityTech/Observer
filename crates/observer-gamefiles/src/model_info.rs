//! The model-info `.meta` files resources declare as `WEAPON_METADATA_FILE`, `VEHICLE_METADATA_FILE` or
//! `PED_METADATA_FILE`. The game makes an archetype of each `Item` of their `InitDatas`, named by its
//! `modelName` (`Name` for peds), with no `.ytyp` behind it: the client's duplicate warning calls these
//! `Extra`. The same item written twice, in one file or in two, is a duplicate archetype.
//!
//! This module lists the items with where they lie and a text to compare them by, and cuts items out
//! of a file leaving every other byte as it was: the file stays as its author wrote it, comments,
//! indentation and line endings included, and a later diff shows the removed items alone.

use std::collections::HashMap;
use std::ops::Range;

use crate::hash::name_hash;
use crate::xml::{self, Document, Element, Node};

/// The largest file read. Real ones hold at most a few hundred items, well under 1 MiB.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

const BOM: &[u8] = b"\xEF\xBB\xBF";
/// The element of each archetype in `InitDatas`.
const ITEM: &str = "Item";
const LIST: &str = "InitDatas";

/// What a model-info file defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Weapon,
    Vehicle,
    Ped,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Weapon, Self::Vehicle, Self::Ped];

    /// The root element of this kind's files, matched as written.
    pub const fn root(self) -> &'static str {
        match self {
            Self::Weapon => "CWeaponModelInfo__InitDataList",
            Self::Vehicle => "CVehicleModelInfo__InitDataList",
            Self::Ped => "CPedModelInfo__InitDataList",
        }
    }

    /// The child of an item that names its model.
    pub const fn name_element(self) -> &'static str {
        match self {
            Self::Weapon | Self::Vehicle => "modelName",
            Self::Ped => "Name",
        }
    }

    /// The `data_file` type a manifest loads this kind's files with.
    pub const fn data_file(self) -> &'static str {
        match self {
            Self::Weapon => "WEAPON_METADATA_FILE",
            Self::Vehicle => "VEHICLE_METADATA_FILE",
            Self::Ped => "PED_METADATA_FILE",
        }
    }

    /// The kind whose files have the root element `name`.
    pub fn from_root(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.root() == name)
    }
}

/// The archetypes a model-info file defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfoFile {
    pub kind: Kind,
    /// The named items, in the order they are written.
    pub items: Vec<ModelItem>,
    /// Items without a name, which define no archetype. They keep their place in the ordinals.
    pub unnamed: usize,
}

impl ModelInfoFile {
    /// Every `Item` of `InitDatas`, named or not: ordinals run below this.
    pub fn item_count(&self) -> usize {
        self.items.len() + self.unnamed
    }
}

/// One `Item` of `InitDatas` that names a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelItem {
    /// Its place among the `Item` elements of `InitDatas`, from 0, unnamed ones counted: what
    /// [`remove_items`] takes.
    pub ordinal: usize,
    /// The model's name as written, white space around it trimmed.
    pub name: String,
    /// The archetype's hash, which the game gives the name whatever its case.
    pub hash: u32,
    /// The line its start tag is on, from 1.
    pub line: usize,
    /// Its bytes in the file, from the `<` of `<Item>` to past the `>` of `</Item>`.
    pub span: Range<usize>,
    /// The item without comments and without the white space around text, references decoded and
    /// attributes quoted alike: two items that differ only in layout, comments, line endings or the
    /// way a character is written compare equal.
    pub canonical: String,
    /// Its child elements, in order, each with its canonical text.
    pub fields: Vec<Field>,
}

/// A child element of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub canonical: String,
}

/// Why a model-info file cannot be read or changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("larger than {} MiB", MAX_BYTES >> 20)]
    TooLarge,
    #[error("not XML text")]
    NotXml,
    #[error("not UTF-8 text (line {line})")]
    NotUtf8 { line: usize },
    #[error("malformed XML, {0}")]
    Xml(#[from] xml::Error),
    #[error("not a model-info file")]
    NotModelInfo,
    #[error("item {0} is asked for twice")]
    DuplicateOrdinal(usize),
    #[error("no item {ordinal}: the file holds {count}")]
    NoSuchItem { ordinal: usize, count: usize },
    #[error("the file without those items does not read back as expected: {0}")]
    Unverified(&'static str),
}

/// Reads a model-info file: `None` when its root element is none of the three kinds. Only the start
/// of a file of another kind is read, so a broken `handling.meta` is no error here.
pub fn read(bytes: &[u8]) -> Result<Option<ModelInfoFile>, Error> {
    let Some((kind, document)) = parse(bytes)? else {
        return Ok(None);
    };
    let mut items = Vec::new();
    let mut unnamed = 0;
    for (ordinal, element) in item_elements(&document).into_iter().enumerate() {
        match model_item(kind, ordinal, element) {
            Some(item) => items.push(item),
            None => unnamed += 1,
        }
    }
    Ok(Some(ModelInfoFile { kind, items, unnamed }))
}

/// The children of `a` and `b` whose canonical texts differ, by name, in the order they first come.
/// A name written several times differs when its values, taken in order, do. Fields written in
/// another order are no difference: the game reads them by name. `Item` is listed when the items'
/// own attributes or text differ and their fields do not.
pub fn differing_fields(a: &ModelItem, b: &ModelItem) -> Vec<String> {
    let mut order: Vec<&str> = Vec::new();
    let mut values: HashMap<&str, [Vec<&str>; 2]> = HashMap::new();
    for (side, item) in [a, b].into_iter().enumerate() {
        for field in &item.fields {
            let entry = values.entry(&field.name).or_insert_with(|| {
                order.push(&field.name);
                [Vec::new(), Vec::new()]
            });
            entry[side].push(&field.canonical);
        }
    }
    let mut differing: Vec<String> = order
        .into_iter()
        .filter(|name| values.get(name).is_some_and(|[left, right]| left != right))
        .map(str::to_owned)
        .collect();
    if differing.is_empty() && a.canonical != b.canonical && a.fields == b.fields {
        differing.push(ITEM.to_owned());
    }
    differing
}

/// The file without the items at `ordinals` (see [`ModelItem::ordinal`]). Each item goes with the
/// spaces and tabs before it on its line and, when nothing else is on its lines, with the line
/// break after it, so whole lines go; every other byte stays. The result is read again to check that
/// exactly those items went and that the others are byte for byte what they were.
pub fn remove_items(bytes: &[u8], ordinals: &[usize]) -> Result<Vec<u8>, Error> {
    let Some((kind, document)) = parse(bytes)? else {
        return Err(Error::NotModelInfo);
    };
    let items = item_elements(&document);
    let mut removed = ordinals.to_vec();
    removed.sort_unstable();
    if let Some(pair) = removed.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(Error::DuplicateOrdinal(pair[0]));
    }
    if let Some(&last) = removed.last()
        && last >= items.len()
    {
        return Err(Error::NoSuchItem {
            ordinal: last,
            count: items.len(),
        });
    }

    // Items come in file order, so the cuts do too; two cuts can only share the blanks between
    // items written on one line.
    let mut cuts: Vec<Range<usize>> = Vec::with_capacity(removed.len());
    for &ordinal in &removed {
        let next = cut(bytes, &items[ordinal].span());
        match cuts.last_mut() {
            Some(previous) if next.start <= previous.end => previous.end = previous.end.max(next.end),
            _ => cuts.push(next),
        }
    }
    let mut output = Vec::with_capacity(bytes.len());
    let mut at = 0;
    for cut in &cuts {
        output.extend_from_slice(&bytes[at..cut.start]);
        at = cut.end;
    }
    output.extend_from_slice(&bytes[at..]);

    let kept: Vec<&str> = items
        .iter()
        .enumerate()
        .filter(|(ordinal, _)| removed.binary_search(ordinal).is_err())
        .map(|(_, item)| item.source())
        .collect();
    verify(&output, kind, &kept)?;
    Ok(output)
}

/// The kind and the parsed text of a model-info file, `None` for another file.
fn parse(bytes: &[u8]) -> Result<Option<(Kind, Document<'_>)>, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let body = bytes.strip_prefix(BOM).unwrap_or(bytes);
    if body
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        != Some(&b'<')
    {
        return Err(Error::NotXml);
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            // Bytes that are not UTF-8 matter only in a model-info file: the root's name, at the
            // start, tells whether this is one.
            let valid = std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or_default();
            if let Ok(Some(name)) = xml::root_name(valid)
                && Kind::from_root(name).is_none()
            {
                return Ok(None);
            }
            return Err(Error::NotUtf8 {
                line: xml::line_of(bytes, error.valid_up_to()),
            });
        }
    };
    let Some(name) = xml::root_name(text)? else {
        // No root element: the full read says what is wrong.
        Document::parse(text)?;
        return Ok(None);
    };
    match Kind::from_root(name) {
        Some(kind) => Ok(Some((kind, Document::parse(text)?))),
        None => Ok(None),
    }
}

/// Every `Item` of the root's first `InitDatas`, in order.
fn item_elements<'d, 'a>(document: &'d Document<'a>) -> Vec<Element<'d, 'a>> {
    document
        .root()
        .child(LIST)
        .map(|list| list.children().filter(|child| child.name() == ITEM).collect())
        .unwrap_or_default()
}

fn model_item(kind: Kind, ordinal: usize, element: Element<'_, '_>) -> Option<ModelItem> {
    let written = element.child(kind.name_element())?.text();
    let name = written.trim_matches(is_space);
    if name.is_empty() {
        return None;
    }
    Some(ModelItem {
        ordinal,
        name: name.to_owned(),
        hash: name_hash(name),
        line: element.line(),
        span: element.span(),
        canonical: canonical(element),
        fields: element
            .children()
            .map(|child| Field {
                name: child.name().to_owned(),
                canonical: canonical(child),
            })
            .collect(),
    })
}

const fn is_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\r' | '\n')
}

/// `element` written on one line: tags with their attributes double-quoted, text decoded, trimmed and
/// escaped again, comments and processing instructions left out, an empty element self-closed.
fn canonical(element: Element<'_, '_>) -> String {
    let mut text = String::new();
    write_canonical(element, &mut text);
    text
}

/// Recurses once per level, which [`xml::MAX_DEPTH`] bounds.
fn write_canonical(element: Element<'_, '_>, out: &mut String) {
    out.push('<');
    out.push_str(element.name());
    for attribute in element.attributes() {
        out.push(' ');
        out.push_str(attribute.name);
        out.push_str("=\"");
        escape(&attribute.value(), out);
        out.push('"');
    }
    out.push('>');
    let opened = out.len();
    // The text between two child elements, comments taken out, written once whole.
    let mut run = String::new();
    for node in element.nodes() {
        match node {
            Node::Element(child) => {
                flush(&mut run, out);
                write_canonical(child, out);
            }
            Node::Text(raw) => run.push_str(&xml::decode(raw)),
            Node::CData(inside) => run.push_str(inside),
            Node::Comment(_) | Node::Instruction(_) => {}
        }
    }
    flush(&mut run, out);
    if out.len() == opened {
        out.pop();
        out.push_str("/>");
    } else {
        out.push_str("</");
        out.push_str(element.name());
        out.push('>');
    }
}

fn flush(run: &mut String, out: &mut String) {
    escape(run.trim_matches(is_space), out);
    run.clear();
}

fn escape(value: &str, out: &mut String) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            character => out.push(character),
        }
    }
}

/// The bytes that go with the item at `span`.
fn cut(bytes: &[u8], span: &Range<usize>) -> Range<usize> {
    let blank = |byte: u8| matches!(byte, b' ' | b'\t');
    let mut before = span.start;
    while before > 0 && blank(bytes[before - 1]) {
        before -= 1;
    }
    let starts_line = before == 0 || matches!(bytes[before - 1], b'\n' | b'\r');
    let mut after = span.end;
    while after < bytes.len() && blank(bytes[after]) {
        after += 1;
    }
    let line_break = if bytes[after..].starts_with(b"\r\n") {
        2
    } else {
        usize::from(matches!(bytes.get(after), Some(b'\n' | b'\r')))
    };
    let ends_line = line_break > 0 || after == bytes.len();
    match (starts_line, ends_line) {
        // Alone on its lines: the lines go.
        (true, true) => before..after + line_break,
        // Something precedes it on its first line, which keeps its line break.
        (false, true) => before..after,
        // Something follows it on its last line, which keeps the indentation before the item.
        (_, false) => span.start..after,
    }
}

/// Checks that `output` reads as a file of `kind` whose items are exactly `kept`, byte for byte.
fn verify(output: &[u8], kind: Kind, kept: &[&str]) -> Result<(), Error> {
    let Ok(Some((after, document))) = parse(output) else {
        return Err(Error::Unverified("it no longer reads as a model-info file"));
    };
    if after != kind {
        return Err(Error::Unverified("its kind changed"));
    }
    let items = item_elements(&document);
    if items.len() != kept.len() {
        return Err(Error::Unverified("another number of items is left"));
    }
    if items.iter().zip(kept).any(|(item, kept)| item.source() != *kept) {
        return Err(Error::Unverified("an item left changed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real weapon pack's archetypes: CRLF, indentation that wanders, blank lines
    /// holding tabs, the same item twice, an element given twice and no line break at the end.
    const GIFT_PACK: [&str; 30] = [
        "<?xml version='1.0' encoding='UTF-8'?>",
        "<CWeaponModelInfo__InitDataList>",
        "\t<InitDatas>",
        "\t\t\t\t",
        "\t<Item>",
        "      <modelName>w_pi_gift</modelName>",
        "      <txdName>w_pi_gift</txdName>",
        "      <ptfxAssetName>null</ptfxAssetName>",
        "      <lodDist value=\"50\" />",
        "    </Item>\t",
        "\t\t\t\t\t<Item>",
        "\t\t\t<modelName>w_at_gift_supp</modelName>",
        "\t\t\t<txdName>w_at_gift_supp</txdName>",
        "\t\t\t<ptfxAssetName>NULL</ptfxAssetName>",
        "\t\t\t<lodDist value=\"30\" />",
        "\t\t</Item>",
        "\t\t\t\t<Item>",
        "\t\t\t<modelName>w_at_gift_supp</modelName>",
        "\t\t\t<txdName>w_at_gift_supp</txdName>",
        "\t\t\t<ptfxAssetName>NULL</ptfxAssetName>",
        "\t\t\t<lodDist value=\"30\" />",
        "\t\t</Item>",
        "\t\t<Item>",
        "\t\t\t<modelName>w_at_gift_grip</modelName>",
        "\t\t\t<ptfxAssetName>NULL</ptfxAssetName>",
        "\t\t\t<ptfxAssetName>null</ptfxAssetName>",
        "\t\t\t<lodDist value=\"30\" />",
        "\t\t</Item>",
        "\t</InitDatas>",
        "</CWeaponModelInfo__InitDataList>",
    ];

    fn gift_pack() -> String {
        GIFT_PACK.join("\r\n")
    }

    /// The gift pack without the lines from `first` to `last` of each pair, counted from 0.
    fn gift_pack_without(lines: &[(usize, usize)]) -> String {
        GIFT_PACK
            .iter()
            .enumerate()
            .filter(|(index, _)| !lines.iter().any(|(first, last)| (first..=last).contains(&index)))
            .map(|(_, line)| *line)
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    fn read_text(text: &str) -> ModelInfoFile {
        read(text.as_bytes()).unwrap().unwrap()
    }

    fn removed(text: &str, ordinals: &[usize]) -> String {
        String::from_utf8(remove_items(text.as_bytes(), ordinals).unwrap()).unwrap()
    }

    const WEAPONS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<CWeaponModelInfo__InitDataList>
  <InitDatas>
    <Item>
      <modelName>w_pi_test</modelName>
      <txdName>w_pi_test</txdName>
      <lodDist value="50" />
    </Item>
    <!-- <Item><modelName>w_pi_commented</modelName></Item> -->
    <Item>
      <!-- <modelName>w_pi_old</modelName> -->
      <modelName>w_pi_test_mag1</modelName>
      <txdName><![CDATA[w_pi_test_mag1]]></txdName>
      <lodDist value="50" />
    </Item>
  </InitDatas>
</CWeaponModelInfo__InitDataList>
"#;

    #[test]
    fn a_weapon_file_gives_its_items_in_order_with_their_lines_and_spans() {
        let file = read_text(WEAPONS);

        assert_eq!(file.kind, Kind::Weapon);
        assert_eq!(file.unnamed, 0);
        let names: Vec<_> = file.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(
            names,
            ["w_pi_test", "w_pi_test_mag1"],
            "a commented item is no item"
        );
        let first = &file.items[0];
        assert_eq!(first.ordinal, 0);
        assert_eq!(first.hash, name_hash("w_pi_test"));
        assert_eq!(first.line, 4);
        assert!(WEAPONS[first.span.clone()].starts_with("<Item>\n      <modelName>w_pi_test<"));
        assert!(WEAPONS[first.span.clone()].ends_with("</Item>"));
        assert_eq!(
            first.canonical,
            "<Item><modelName>w_pi_test</modelName><txdName>w_pi_test</txdName><lodDist value=\"50\"/></Item>"
        );
        let second = &file.items[1];
        assert_eq!((second.ordinal, second.line), (1, 10));
        assert_eq!(
            second.canonical,
            "<Item><modelName>w_pi_test_mag1</modelName><txdName>w_pi_test_mag1</txdName><lodDist value=\"50\"/></Item>",
            "comments go and CDATA reads as text"
        );
        let fields: Vec<_> = second.fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(fields, ["modelName", "txdName", "lodDist"]);
    }

    #[test]
    fn the_same_item_written_twice_gives_one_hash_and_one_canonical_text() {
        let text = gift_pack();
        let file = read_text(&text);

        let names: Vec<_> = file.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(
            names,
            ["w_pi_gift", "w_at_gift_supp", "w_at_gift_supp", "w_at_gift_grip"]
        );
        let (first, second) = (&file.items[1], &file.items[2]);
        assert_eq!((first.line, second.line), (11, 17));
        assert_eq!(first.hash, second.hash);
        assert_eq!(first.canonical, second.canonical);
        assert_eq!(differing_fields(first, second), Vec::<String>::new());
        assert_ne!(first.span, second.span);
        assert_eq!(text[first.span.clone()], text[second.span.clone()]);
    }

    #[test]
    fn items_written_with_other_layouts_compare_equal() {
        let tabs = read_text(&gift_pack());
        let other = read_text(
            "\u{feff}<CWeaponModelInfo__InitDataList><InitDatas>\n\
             <Item>\n  <modelName>\n    w_at_gift_supp\n  </modelName> <!-- the suppressor -->\n\
             \x20 <txdName><![CDATA[w_at_gift_supp]]></txdName>\n\
             \x20 <ptfxAssetName>&#78;ULL</ptfxAssetName>\n  <lodDist value='30'></lodDist>\n</Item>\n\
             </InitDatas></CWeaponModelInfo__InitDataList>",
        );

        assert_eq!(other.items[0].canonical, tabs.items[1].canonical);
        assert_eq!(other.items[0].name, "w_at_gift_supp");
    }

    #[test]
    fn differing_fields_are_listed_by_name_in_the_order_they_come() {
        let items = |body: &str| {
            let text = format!(
                "<CWeaponModelInfo__InitDataList><InitDatas>{body}</InitDatas></CWeaponModelInfo__InitDataList>"
            );
            read_text(&text).items
        };

        let changed = items(
            "<Item><modelName>a</modelName><txdName>t</txdName><lodDist value='30'/></Item>\
             <Item><modelName>a</modelName><txdName>u</txdName><lodDist value='60'/></Item>",
        );
        assert_eq!(differing_fields(&changed[0], &changed[1]), ["txdName", "lodDist"]);

        let added = items(
            "<Item><modelName>a</modelName></Item><Item><modelName>a</modelName><txdName>t</txdName></Item>",
        );
        assert_eq!(differing_fields(&added[0], &added[1]), ["txdName"]);

        let reordered = items(
            "<Item><modelName>a</modelName><txdName>t</txdName></Item>\
             <Item><txdName>t</txdName><modelName>a</modelName></Item>",
        );
        assert_ne!(reordered[0].canonical, reordered[1].canonical);
        assert_eq!(
            differing_fields(&reordered[0], &reordered[1]),
            Vec::<String>::new()
        );

        let repeated = items(
            "<Item><modelName>a</modelName><fx>1</fx><fx>2</fx></Item>\
             <Item><modelName>a</modelName><fx>2</fx><fx>1</fx></Item>",
        );
        assert_eq!(differing_fields(&repeated[0], &repeated[1]), ["fx"]);

        let tagged =
            items("<Item kind='x'><modelName>a</modelName></Item><Item><modelName>a</modelName></Item>");
        assert_eq!(differing_fields(&tagged[0], &tagged[1]), ["Item"]);
    }

    #[test]
    fn a_name_is_trimmed_and_kept_in_its_case_and_hashed_as_the_game_does() {
        let file = read_text(
            "<CVehicleModelInfo__InitDataList><InitDatas><Item><modelName>\n  BF_Test \r\n</modelName></Item>\
             </InitDatas></CVehicleModelInfo__InitDataList>",
        );

        assert_eq!(file.items[0].name, "BF_Test");
        assert_eq!(file.items[0].hash, name_hash("bf_test"));
    }

    #[test]
    fn vehicle_files_count_the_items_of_init_datas_alone() {
        let file = read_text(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CVehicleModelInfo__InitDataList>\n\
             \x20 <residentTxd>vehshare</residentTxd>\n  <residentAnims />\n  <InitDatas>\n\
             \x20   <Item>\n      <modelName>blitz</modelName>\n      <txdName>blitz</txdName>\n    </Item>\n\
             \x20 </InitDatas>\n  <txdRelationships>\n    <Item>\n      <parent>vehicles_muscle_n_w_interior</parent>\n\
             \x20     <child>blitz</child>\n    </Item>\n  </txdRelationships>\n</CVehicleModelInfo__InitDataList>\n",
        );

        assert_eq!(file.kind, Kind::Vehicle);
        assert_eq!(file.item_count(), 1);
        assert_eq!(file.items[0].name, "blitz");
        assert_eq!(file.items[0].line, 6);
    }

    #[test]
    fn ped_files_name_their_items_with_name() {
        let file = read_text(
            "<CPedModelInfo__InitDataList>\n<residentTxd>comp_peds_generic</residentTxd>\n<InitDatas>\n\
             <Item>\n<Name>a_c_test</Name>\n<PropsName>null</PropsName>\n</Item>\n\
             <Item>\n<modelName>not_a_ped_name</modelName>\n</Item>\n\
             </InitDatas>\n</CPedModelInfo__InitDataList>",
        );

        assert_eq!(file.kind, Kind::Ped);
        assert_eq!(file.items.len(), 1);
        assert_eq!(file.items[0].name, "a_c_test");
        assert_eq!(file.unnamed, 1);
        assert_eq!(Kind::Ped.data_file(), "PED_METADATA_FILE");
    }

    #[test]
    fn items_without_a_name_are_counted_and_keep_their_ordinals() {
        let file = read_text(
            "<CWeaponModelInfo__InitDataList><InitDatas>\
             <Item><txdName>no_model</txdName></Item>\
             <Item><modelName>first</modelName></Item>\
             <Item><modelName> </modelName></Item>\
             <Item/>\
             <NotAnItem><modelName>ignored</modelName></NotAnItem>\
             <Item><modelName>second</modelName></Item>\
             </InitDatas></CWeaponModelInfo__InitDataList>",
        );

        assert_eq!(file.unnamed, 3);
        assert_eq!(file.item_count(), 5);
        let named: Vec<_> = file
            .items
            .iter()
            .map(|item| (item.ordinal, item.name.as_str()))
            .collect();
        assert_eq!(named, [(1, "first"), (4, "second")]);
    }

    #[test]
    fn other_meta_files_are_none() {
        for text in [
            "<?xml version=\"1.0\"?>\n<CWeaponInfoBlob><Infos/></CWeaponInfoBlob>",
            "<CHandlingDataMgr><HandlingData><Item type=\"CHandlingData\"/></HandlingData></CHandlingDataMgr>",
            "<cweaponmodelinfo__initdatalist><InitDatas/></cweaponmodelinfo__initdatalist>",
            "<CHandlingDataMgr>never closed, which is no concern of this reader",
        ] {
            assert_eq!(read(text.as_bytes()), Ok(None), "{text}");
        }
        let empty = read_text("<CPedModelInfo__InitDataList/>");
        assert_eq!((empty.kind, empty.item_count()), (Kind::Ped, 0));
    }

    #[test]
    fn bytes_that_are_not_utf8_are_an_error_in_a_model_info_file_alone() {
        let model_info =
            b"<CWeaponModelInfo__InitDataList>\n<!-- caf\xE9 -->\n</CWeaponModelInfo__InitDataList>";
        assert_eq!(read(model_info), Err(Error::NotUtf8 { line: 2 }));

        let handling = b"<CHandlingDataMgr>\n<!-- caf\xE9 -->\n</CHandlingDataMgr>";
        assert_eq!(read(handling), Ok(None));

        let cut_before_the_root = b"<!-- caf\xE9 --><CHandlingDataMgr/>";
        assert_eq!(read(cut_before_the_root), Err(Error::NotUtf8 { line: 1 }));
    }

    #[test]
    fn what_is_not_xml_text_is_refused() {
        let refused: [&[u8]; 5] = [
            b"",
            b"  \r\n",
            b"PSIN\x00\x00\x00\x10",
            b"\xFF\xFE<\x00a\x00",
            b"RSC7",
        ];
        for bytes in refused {
            assert_eq!(read(bytes), Err(Error::NotXml), "{bytes:?}");
        }
        assert_eq!(read(&vec![b' '; MAX_BYTES + 1]), Err(Error::TooLarge));
    }

    #[test]
    fn a_malformed_model_info_file_is_an_error_with_its_line() {
        let error = read(b"<CWeaponModelInfo__InitDataList>\n<InitDatas>\n<Item>\n</InitDatas>").unwrap_err();

        let Error::Xml(error) = error else {
            panic!("{error:?}");
        };
        assert_eq!(error.line, 4);
    }

    #[test]
    fn removing_a_copy_cuts_its_lines_and_leaves_every_other_byte() {
        let text = gift_pack();

        assert_eq!(removed(&text, &[2]), gift_pack_without(&[(16, 21)]));
        assert_eq!(
            removed(&text, &[0]),
            gift_pack_without(&[(4, 9)]),
            "a trailing tab goes with its line"
        );
        assert_eq!(removed(&text, &[3]), gift_pack_without(&[(22, 27)]));
        assert_eq!(
            removed(&text, &[3, 0, 1]),
            gift_pack_without(&[(4, 15), (22, 27)])
        );
        assert_eq!(removed(&text, &[]), text);

        let file = read_text(&removed(&text, &[2]));
        let names: Vec<_> = file.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["w_pi_gift", "w_at_gift_supp", "w_at_gift_grip"]);
    }

    #[test]
    fn line_feeds_stay_line_feeds() {
        let without = removed(WEAPONS, &[0]);

        assert_eq!(
            without,
            WEAPONS.replace(
                "    <Item>\n      <modelName>w_pi_test</modelName>\n      <txdName>w_pi_test</txdName>\n      \
                 <lodDist value=\"50\" />\n    </Item>\n",
                ""
            )
        );
        assert!(!without.contains('\r'));
    }

    #[test]
    fn items_sharing_a_line_go_without_the_line() {
        let text = "<CWeaponModelInfo__InitDataList>\n  <InitDatas><Item><modelName>a</modelName></Item> \
                    <Item><modelName>b</modelName></Item>\n    <Item><modelName>c</modelName></Item> <!-- c -->\n  \
                    </InitDatas>\n</CWeaponModelInfo__InitDataList>";

        assert_eq!(
            removed(text, &[0]),
            text.replace("<Item><modelName>a</modelName></Item> ", "")
        );
        assert_eq!(
            removed(text, &[1]),
            text.replace(" <Item><modelName>b</modelName></Item>", "")
        );
        assert_eq!(
            removed(text, &[0, 1]),
            text.replace(
                "<Item><modelName>a</modelName></Item> <Item><modelName>b</modelName></Item>",
                ""
            )
        );
        assert_eq!(
            removed(text, &[2]),
            text.replace("<Item><modelName>c</modelName></Item> ", "")
        );
    }

    #[test]
    fn removals_that_do_not_fit_the_file_are_refused() {
        let text = gift_pack();

        assert_eq!(
            remove_items(text.as_bytes(), &[1, 1]),
            Err(Error::DuplicateOrdinal(1))
        );
        assert_eq!(
            remove_items(text.as_bytes(), &[4]),
            Err(Error::NoSuchItem { ordinal: 4, count: 4 })
        );
        assert_eq!(
            remove_items(b"<CHandlingDataMgr/>", &[0]),
            Err(Error::NotModelInfo)
        );
        assert!(matches!(
            remove_items(b"<CWeaponModelInfo__InitDataList><InitDatas>", &[0]),
            Err(Error::Xml(_))
        ));
    }

    #[test]
    fn unnamed_items_can_be_removed_by_their_ordinal() {
        let text = "<CWeaponModelInfo__InitDataList><InitDatas>\n<Item/>\n<Item><modelName>a</modelName></Item>\n\
                    </InitDatas></CWeaponModelInfo__InitDataList>";

        assert_eq!(removed(text, &[0]), text.replace("<Item/>\n", ""));
    }

    #[test]
    fn every_cut_of_a_file_reads_or_fails_without_panicking() {
        let text = gift_pack();
        for (end, _) in text.char_indices() {
            let cut = &text.as_bytes()[..end];
            let _ = read(cut);
            let _ = remove_items(cut, &[0]);
        }
        let bytes = text.as_bytes();
        for at in 0..bytes.len() {
            let mut scrambled = bytes.to_vec();
            scrambled[at] = 0xFF;
            let _ = read(&scrambled);
            scrambled[at] = b'<';
            let _ = read(&scrambled);
            let _ = remove_items(&scrambled, &[1, 2]);
        }
    }
}
