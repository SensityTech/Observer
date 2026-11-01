//! The archetypes a `.ytyp` defines, as the client registers them when the file loads, and the one
//! change Observer makes to such a file: taking entries out of its archetype list.
//!
//! The list is `CMapTypes.archetypes`, an array of packed pointers kept in a pointer block. That
//! `CMapTypes` is the root block the metadata header names, where the game starts reading, and a
//! file holding another one is refused. Each pointer leads to a `CBaseArchetypeDef`,
//! `CTimeArchetypeDef` or `CMloArchetypeDef`, told apart by the block it lands in; a file may spread
//! one kind over several blocks, so the list is the only order there is. The fields are found
//! through the file's own structure description when it has one, else at the offsets every known
//! writer uses.
//!
//! Removing entries moves the later pointers down inside the list, zeroes the freed slots and lowers
//! the count. No other byte moves and no size changes, so the RSC7 header stays true; the structures
//! taken out stay in the file, unreferenced. Every patch is read back and compared before it is
//! returned.

use crate::Error;
use crate::hash::joaat;
use crate::meta::canonical::Digester;
use crate::meta::data_type::{ARRAY, FLOAT, HASH, INT, INT_ENUM, VECTOR3};
use crate::meta::{
    ArrayHeader, Meta, POINTER_BLOCK, Pointer, StructureInfo, array_at, f32_at, u32_at, u64_at, vector3_at,
};
use crate::rsc7::Resource;

/// The literal joaat of `CMapTypes`, the root structure of a `.ytyp`.
pub const MAP_TYPES: u32 = joaat(b"CMapTypes");
/// The resource version of binary metadata.
const VERSION: u32 = 2;
const MAP_TYPES_LEN: usize = 80;
const ARCHETYPES_AT: usize = 24;
const MAP_NAME_AT: usize = 40;
const POINTER_LEN: usize = 8;

/// Which archetype structure an entry of the list leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArchetypeKind {
    /// `CBaseArchetypeDef`: a prop or building.
    Base,
    /// `CTimeArchetypeDef`: shown only at the hours its time flags give.
    Time,
    /// `CMloArchetypeDef`: an interior, with its entities and rooms.
    Mlo,
}

impl ArchetypeKind {
    pub const ALL: [Self; 3] = [Self::Base, Self::Time, Self::Mlo];

    /// The structure's name, as the schema writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Base => "CBaseArchetypeDef",
            Self::Time => "CTimeArchetypeDef",
            Self::Mlo => "CMloArchetypeDef",
        }
    }

    /// The literal joaat of the structure's name, which its blocks carry as their type.
    pub const fn structure(self) -> u32 {
        match self {
            Self::Base => joaat(b"CBaseArchetypeDef"),
            Self::Time => joaat(b"CTimeArchetypeDef"),
            Self::Mlo => joaat(b"CMloArchetypeDef"),
        }
    }

    /// The structure's size in the layout every known writer uses.
    pub const fn size(self) -> usize {
        match self {
            Self::Base => 144,
            Self::Time => 160,
            Self::Mlo => 240,
        }
    }

    pub fn from_structure(structure: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.structure() == structure)
    }
}

/// A field of the archetype structures: its name hash, its offset in the usual layout, its type.
#[derive(Clone, Copy)]
struct Field {
    name: u32,
    at: usize,
    data_type: u8,
}

const fn field(name: &[u8], at: usize, data_type: u8) -> Field {
    Field {
        name: joaat(name),
        at,
        data_type,
    }
}

const LOD_DIST: Field = field(b"lodDist", 8, FLOAT);
const FLAGS: Field = field(b"flags", 12, INT);
const SPECIAL_ATTRIBUTE: Field = field(b"specialAttribute", 16, INT);
const BB_MIN: Field = field(b"bbMin", 32, VECTOR3);
const BB_MAX: Field = field(b"bbMax", 48, VECTOR3);
const BS_CENTRE: Field = field(b"bsCentre", 64, VECTOR3);
const BS_RADIUS: Field = field(b"bsRadius", 80, FLOAT);
const HD_TEXTURE_DIST: Field = field(b"hdTextureDist", 84, FLOAT);
const NAME: Field = field(b"name", 88, HASH);
const TEXTURE_DICTIONARY: Field = field(b"textureDictionary", 92, HASH);
const CLIP_DICTIONARY: Field = field(b"clipDictionary", 96, HASH);
const DRAWABLE_DICTIONARY: Field = field(b"drawableDictionary", 100, HASH);
const PHYSICS_DICTIONARY: Field = field(b"physicsDictionary", 104, HASH);
const ASSET_TYPE: Field = field(b"assetType", 108, INT_ENUM);
const ASSET_NAME: Field = field(b"assetName", 112, HASH);
const EXTENSIONS: Field = field(b"extensions", 120, ARRAY);
const TIME_FLAGS: Field = field(b"timeFlags", 144, INT);
const MAP_ARCHETYPES: Field = field(b"archetypes", ARCHETYPES_AT, ARRAY);
const MAP_NAME: Field = field(b"name", MAP_NAME_AT, HASH);

/// One archetype of the list, with the fields the client and a reader of the report care about.
#[derive(Debug, Clone, PartialEq)]
pub struct Archetype {
    /// Its position in the archetype list, entries of unknown types counted.
    pub index: usize,
    pub kind: ArchetypeKind,
    /// The hash the client registers: the joaat of the lower-cased name.
    pub name: u32,
    pub lod_dist: f32,
    pub flags: u32,
    pub special_attribute: u32,
    pub bb_min: [f32; 3],
    pub bb_max: [f32; 3],
    pub bs_centre: [f32; 3],
    pub bs_radius: f32,
    pub hd_texture_dist: f32,
    pub texture_dictionary: u32,
    pub clip_dictionary: u32,
    pub drawable_dictionary: u32,
    pub physics_dictionary: u32,
    /// 0 uninitialised, 1 fragment, 2 drawable, 3 drawable dictionary, 4 assetless.
    pub asset_type: u32,
    pub asset_name: u32,
    pub extension_count: u16,
    /// For a time archetype, the hours it shows at.
    pub time_flags: Option<u32>,
    /// The canonical digest of the whole structure, extensions and interior contents included;
    /// `None` when it cannot be compared.
    pub digest: Option<[u8; 32]>,
    /// The same without `lodDist`, the bounds and `hdTextureDist`.
    pub major_digest: Option<[u8; 32]>,
}

/// An entry of the archetype list that leads to a structure other than the three archetypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skipped {
    pub index: usize,
    /// The type of the block it leads to.
    pub structure: u32,
}

/// What a `.ytyp` defines.
#[derive(Debug, Clone, PartialEq)]
pub struct MapTypes {
    /// `CMapTypes.name`, normally the joaat of the file's stem.
    pub name: u32,
    pub archetypes: Vec<Archetype>,
    pub skipped: Vec<Skipped>,
    /// Why [`remove_archetypes`] would refuse this file, if it would.
    pub patch_refusal: Option<&'static str>,
}

/// Reads the archetypes of a `.ytyp`.
pub fn read(bytes: &[u8]) -> Result<MapTypes, Error> {
    read_system(open(bytes)?.system())
}

/// Opens a resource of binary metadata.
fn open(bytes: &[u8]) -> Result<Resource, Error> {
    let resource = Resource::read(bytes)?;
    if resource.version() != VERSION {
        return Err(Error::Version(resource.version()));
    }
    Ok(resource)
}

/// Reads the archetypes from the system segment of a `.ytyp`.
fn read_system(system: &[u8]) -> Result<MapTypes, Error> {
    let meta = Meta::parse(system)?;
    let list = List::locate(&meta)?;
    let mut digester = Digester::new(&meta);
    let mut archetypes = Vec::new();
    let mut skipped = Vec::new();
    let mut cut_short = false;
    for (index, &raw) in list.pointers.iter().enumerate() {
        let pointer =
            Pointer::unpack(raw).ok_or(Error::Malformed("an entry of the archetype list is null"))?;
        let (block, bytes) = meta.pointed(pointer).ok_or(Error::Malformed(
            "an entry of the archetype list leads outside the file",
        ))?;
        let Some(kind) = ArchetypeKind::from_structure(block.structure) else {
            skipped.push(Skipped {
                index,
                structure: block.structure,
            });
            continue;
        };
        let info = meta.structure(kind.structure());
        let size = info.map_or(kind.size(), |info| info.size);
        // What a misdescribed structure leaves inside its block is still read, for the names.
        cut_short |= bytes.len() < size;
        let mut archetype = archetype_at(info, kind, index, bytes.get(..size).unwrap_or(bytes))?;
        if let Some(digests) = digester.structure(pointer) {
            archetype.digest = Some(digests.full);
            archetype.major_digest = Some(digests.major);
        }
        archetypes.push(archetype);
    }
    let patch_refusal = if cut_short {
        Some("an archetype runs past its block")
    } else {
        patch_refusal(&meta, &list)
    };
    Ok(MapTypes {
        name: list.name,
        archetypes,
        skipped,
        patch_refusal,
    })
}

/// Reads one archetype from `bytes`, the structure, through `info` when the file describes the
/// kind, else through the usual layout.
fn archetype_at(
    info: Option<&StructureInfo>,
    kind: ArchetypeKind,
    index: usize,
    bytes: &[u8],
) -> Result<Archetype, Error> {
    // A field the description leaves out, gives another type, or puts past the bytes, reads as zero.
    let at = |field: Field| match info {
        None => Some(field.at),
        Some(info) => info
            .field(field.name)
            .filter(|entry| entry.data_type == field.data_type)
            .map(|entry| entry.offset),
    };
    let word = |field| at(field).and_then(|at| u32_at(bytes, at)).unwrap_or_default();
    let float = |field| at(field).and_then(|at| f32_at(bytes, at)).unwrap_or_default();
    let vector = |field| at(field).and_then(|at| vector3_at(bytes, at)).unwrap_or_default();
    let name = at(NAME)
        .and_then(|at| u32_at(bytes, at))
        .ok_or(Error::Malformed("an archetype has no name"))?;
    let extensions = at(EXTENSIONS).and_then(|at| array_at(bytes, at));
    Ok(Archetype {
        index,
        kind,
        name,
        lod_dist: float(LOD_DIST),
        flags: word(FLAGS),
        special_attribute: word(SPECIAL_ATTRIBUTE),
        bb_min: vector(BB_MIN),
        bb_max: vector(BB_MAX),
        bs_centre: vector(BS_CENTRE),
        bs_radius: float(BS_RADIUS),
        hd_texture_dist: float(HD_TEXTURE_DIST),
        texture_dictionary: word(TEXTURE_DICTIONARY),
        clip_dictionary: word(CLIP_DICTIONARY),
        drawable_dictionary: word(DRAWABLE_DICTIONARY),
        physics_dictionary: word(PHYSICS_DICTIONARY),
        asset_type: word(ASSET_TYPE),
        asset_name: word(ASSET_NAME),
        extension_count: extensions.map_or(0, |header| header.count),
        time_flags: (kind == ArchetypeKind::Time).then(|| word(TIME_FLAGS)),
        digest: None,
        major_digest: None,
    })
}

/// The names of the fields in which `a` and `b` differ, as the schema writes them. When every field
/// read is the same but the digests differ or cannot be compared, the difference lies in what the
/// structure leads to: `extensions` for a prop that has some, `contents` otherwise.
pub fn differences(a: &Archetype, b: &Archetype) -> Vec<&'static str> {
    let mut names = field_differences(a, b);
    if names.is_empty() && (a.digest.is_none() || a.digest != b.digest) {
        let has_extensions = a.kind != ArchetypeKind::Mlo && a.extension_count > 0;
        names.push(if has_extensions { "extensions" } else { "contents" });
    }
    names
}

/// The names of the fields read in which `a` and `b` differ, floats compared by their bits.
fn field_differences(a: &Archetype, b: &Archetype) -> Vec<&'static str> {
    let bits = |vector: [f32; 3]| vector.map(f32::to_bits);
    let checks = [
        ("kind", a.kind != b.kind),
        ("name", a.name != b.name),
        ("lodDist", a.lod_dist.to_bits() != b.lod_dist.to_bits()),
        ("flags", a.flags != b.flags),
        ("specialAttribute", a.special_attribute != b.special_attribute),
        ("bbMin", bits(a.bb_min) != bits(b.bb_min)),
        ("bbMax", bits(a.bb_max) != bits(b.bb_max)),
        ("bsCentre", bits(a.bs_centre) != bits(b.bs_centre)),
        ("bsRadius", a.bs_radius.to_bits() != b.bs_radius.to_bits()),
        (
            "hdTextureDist",
            a.hd_texture_dist.to_bits() != b.hd_texture_dist.to_bits(),
        ),
        ("textureDictionary", a.texture_dictionary != b.texture_dictionary),
        ("clipDictionary", a.clip_dictionary != b.clip_dictionary),
        (
            "drawableDictionary",
            a.drawable_dictionary != b.drawable_dictionary,
        ),
        ("physicsDictionary", a.physics_dictionary != b.physics_dictionary),
        ("assetType", a.asset_type != b.asset_type),
        ("assetName", a.asset_name != b.asset_name),
        ("extensions", a.extension_count != b.extension_count),
        ("timeFlags", a.time_flags != b.time_flags),
    ];
    checks
        .into_iter()
        .filter(|&(_, differs)| differs)
        .map(|(name, _)| name)
        .collect()
}

/// The archetype list of a file, located.
struct List {
    /// `CMapTypes.name`.
    name: u32,
    /// Where `CMapTypes` starts in the system segment.
    map_types_at: usize,
    header: ArrayHeader,
    /// Where the pointers start in the system segment; meaningless for an empty list.
    pointers_at: usize,
    pointers: Vec<u64>,
}

impl List {
    /// The list of the root `CMapTypes`. The list must lie wholly in one pointer block.
    fn locate(meta: &Meta<'_>) -> Result<Self, Error> {
        let id = root_map_types(meta)?;
        let block = *meta.block(id).ok_or(Error::NoMapTypes)?;
        let bytes = meta
            .block_bytes(id)
            .filter(|bytes| bytes.len() >= MAP_TYPES_LEN)
            .ok_or(Error::Malformed("CMapTypes is cut short"))?;
        if let Some(info) = meta.structure(MAP_TYPES) {
            let lies_at = |field: Field| {
                info.field(field.name)
                    .is_some_and(|entry| (entry.offset, entry.data_type) == (field.at, field.data_type))
            };
            if !lies_at(MAP_ARCHETYPES) || !lies_at(MAP_NAME) {
                return Err(Error::Malformed(
                    "the file describes CMapTypes with another layout",
                ));
            }
        }
        let header = array_at(bytes, ARCHETYPES_AT).ok_or(Error::Malformed("CMapTypes is cut short"))?;
        let name = u32_at(bytes, MAP_NAME_AT).ok_or(Error::Malformed("CMapTypes is cut short"))?;
        let mut list = Self {
            name,
            map_types_at: block.start,
            header,
            pointers_at: 0,
            pointers: Vec::new(),
        };
        if header.count == 0 {
            return Ok(list);
        }
        let pointer = header
            .pointer
            .ok_or(Error::Malformed("the archetype list has entries but no address"))?;
        let (pointer_block, at) = meta
            .resolve(pointer)
            .ok_or(Error::Malformed("the archetype list leads outside the file"))?;
        if pointer_block.structure != POINTER_BLOCK {
            return Err(Error::Malformed("the archetype list is not in a pointer block"));
        }
        let end = at + usize::from(header.count) * POINTER_LEN;
        if end > pointer_block.end() {
            return Err(Error::Malformed("the archetype list runs past its pointer block"));
        }
        list.pointers_at = at;
        list.pointers = (at..end)
            .step_by(POINTER_LEN)
            .map(|at| u64_at(meta.system(), at))
            .collect::<Option<_>>()
            .ok_or(Error::Malformed(
                "the archetype list runs past the system segment",
            ))?;
        Ok(list)
    }
}

/// The number of the block holding `CMapTypes`. The game loads the root block the header names, so
/// that must be it, and it must be the only one: reading, patching and checking another list would
/// report archetypes the game never registers, or change a list it never reads.
fn root_map_types(meta: &Meta<'_>) -> Result<u16, Error> {
    let mut found = meta.blocks().iter().filter(|block| block.structure == MAP_TYPES);
    if found.next().is_none() {
        return Err(Error::NoMapTypes);
    }
    if found.next().is_some() {
        return Err(Error::Malformed("the file holds more than one CMapTypes"));
    }
    u16::try_from(meta.root_block())
        .ok()
        .filter(|&id| meta.block(id).is_some_and(|block| block.structure == MAP_TYPES))
        .ok_or(Error::Malformed("the root block is not CMapTypes"))
}

/// Why the file's layout is not the one the patch was checked against, if it is not.
fn patch_refusal(meta: &Meta<'_>, list: &List) -> Option<&'static str> {
    if meta
        .structure(MAP_TYPES)
        .is_some_and(|info| info.size != MAP_TYPES_LEN)
    {
        return Some("the file describes CMapTypes with another size");
    }
    for kind in ArchetypeKind::ALL {
        if meta
            .structure(kind.structure())
            .is_some_and(|info| info.size != kind.size())
        {
            return Some(match kind {
                ArchetypeKind::Base => "the file describes CBaseArchetypeDef with another size",
                ArchetypeKind::Time => "the file describes CTimeArchetypeDef with another size",
                ArchetypeKind::Mlo => "the file describes CMloArchetypeDef with another size",
            });
        }
    }
    if list.header.capacity < list.header.count {
        return Some("the archetype list holds more entries than its capacity");
    }
    None
}

/// The file with the archetypes at `indices` (positions in the list, as [`Archetype::index`] gives
/// them) taken out of its archetype list. An index given twice, out of range or naming an entry that
/// is no archetype, or a file whose layout is not the expected one, is refused. The result is read
/// back and must hold the same archetypes, digests included, minus exactly those, in the same order.
pub fn remove_archetypes(bytes: &[u8], indices: &[usize]) -> Result<Vec<u8>, Error> {
    let mut resource = open(bytes)?;
    let (before, removed) = patch(resource.system_mut(), indices)?;
    let patched = resource.to_bytes()?;
    let after = read(&patched).map_err(|_| Error::Verification("the patched file cannot be read"))?;
    verify(&before, &removed, &after)?;
    Ok(patched)
}

/// Takes the entries at `indices` out of the archetype list in `system`, checks the segment then
/// reads as it should, and returns what it held before and which entries went.
fn patch(system: &mut [u8], indices: &[usize]) -> Result<(MapTypes, Vec<bool>), Error> {
    let before = read_system(system)?;
    if let Some(reason) = before.patch_refusal {
        return Err(Error::NotPatchable(reason));
    }
    let list = List::locate(&Meta::parse(system)?)?;
    let removed = chosen(&before, list.pointers.len(), indices)?;
    if !indices.is_empty() {
        let kept = list
            .pointers
            .iter()
            .zip(&removed)
            .filter(|&(_, &gone)| !gone)
            .map(|(&raw, _)| raw);
        let slots = kept.chain(std::iter::repeat(0)).take(list.pointers.len());
        for (slot, raw) in slots.enumerate() {
            write(system, list.pointers_at + slot * POINTER_LEN, &raw.to_le_bytes())?;
        }
        let count = list.pointers.len() - indices.len();
        let count = u16::try_from(count).map_err(|_| Error::Malformed("the archetype list is too long"))?;
        let header_at = list.map_types_at + ARCHETYPES_AT;
        write(system, header_at + 8, &count.to_le_bytes())?;
        write(system, header_at + 10, &count.to_le_bytes())?;
        if count == 0 {
            write(system, header_at, &[0; 8])?;
        }
    }
    let after = read_system(system).map_err(|_| Error::Verification("the patched file cannot be read"))?;
    verify(&before, &removed, &after)?;
    Ok((before, removed))
}

/// Which entries `indices` take out of a list of `count`, refusing anything but distinct archetypes.
fn chosen(before: &MapTypes, count: usize, indices: &[usize]) -> Result<Vec<bool>, Error> {
    let mut removed = vec![false; count];
    for &index in indices {
        let slot = removed
            .get_mut(index)
            .ok_or(Error::IndexOutOfRange { index, count })?;
        if *slot {
            return Err(Error::DuplicateIndex(index));
        }
        // The archetypes are in list order: a search, not a scan, for each of up to 65,535 indices.
        if before
            .archetypes
            .binary_search_by_key(&index, |archetype| archetype.index)
            .is_err()
        {
            return Err(Error::NotAnArchetype(index));
        }
        *slot = true;
    }
    Ok(removed)
}

fn write(system: &mut [u8], at: usize, bytes: &[u8]) -> Result<(), Error> {
    system
        .get_mut(at..at + bytes.len())
        .ok_or(Error::Malformed(
            "a patched value lies outside the system segment",
        ))?
        .copy_from_slice(bytes);
    Ok(())
}

/// Checks `after` holds what `before` held minus the `removed` entries, in the same order.
fn verify(before: &MapTypes, removed: &[bool], after: &MapTypes) -> Result<(), Error> {
    let gone = |index: usize| removed.get(index).copied().unwrap_or(true);
    let moved = |index: usize| index - removed.iter().take(index).filter(|&&gone| gone).count();
    if after.name != before.name || after.patch_refusal.is_some() {
        return Err(Error::Verification("the file's name or layout changed"));
    }
    let expected = before
        .archetypes
        .iter()
        .filter(|archetype| !gone(archetype.index));
    let same = expected.clone().count() == after.archetypes.len()
        && expected.zip(&after.archetypes).all(|(kept, found)| {
            found.index == moved(kept.index)
                && field_differences(kept, found).is_empty()
                && found.digest == kept.digest
                && found.major_digest == kept.major_digest
        });
    if !same {
        return Err(Error::Verification("the archetypes left are not the ones kept"));
    }
    let skipped: Vec<Skipped> = before
        .skipped
        .iter()
        .map(|skipped| Skipped {
            index: moved(skipped.index),
            ..*skipped
        })
        .collect();
    if skipped != after.skipped {
        return Err(Error::Verification("the entries of other types moved"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::name_hash;
    use crate::meta::DataBlock;
    use crate::testing::{ArchetypeSpec, Extension, YtypBuilder};

    fn names(map_types: &MapTypes) -> Vec<u32> {
        map_types
            .archetypes
            .iter()
            .map(|archetype| archetype.name)
            .collect()
    }

    fn hashes(names: &[&str]) -> Vec<u32> {
        names.iter().map(|name| name_hash(name)).collect()
    }

    fn camp() -> Vec<u8> {
        YtypBuilder::new("camp_props")
            .archetype(ArchetypeSpec::base("camp_food_marshmallow"))
            .archetype(ArchetypeSpec::base("camp_stick_marshmallow"))
            .archetype(ArchetypeSpec::time("camp_lamp", 0x00F0_000F))
            .archetype(ArchetypeSpec::base("camp_food_marshmallow"))
            .archetype(ArchetypeSpec::mlo("camp_tent"))
            .build()
    }

    /// The system segment of `bytes` changed by `change`, written back.
    fn altered(bytes: &[u8], change: impl FnOnce(&Meta<'_>, &mut Vec<u8>)) -> Vec<u8> {
        let resource = Resource::read(bytes).unwrap();
        let mut system = resource.system().to_vec();
        let meta = Meta::parse(resource.system()).unwrap();
        change(&meta, &mut system);
        Resource::new(2, resource.system_flags(), resource.graphics_flags(), system)
            .unwrap()
            .to_bytes()
            .unwrap()
    }

    #[test]
    fn every_archetype_is_read_in_list_order_with_its_fields() {
        let spec = ArchetypeSpec {
            lod_dist: 250.0,
            flags: 0x0002_0000,
            special_attribute: 31,
            bb_min: [-1.0, -2.0, -3.0],
            bb_max: [1.0, 2.0, 3.0],
            bs_centre: [0.5, 0.5, 0.5],
            bs_radius: 3.5,
            hd_texture_dist: 40.0,
            texture_dictionary: name_hash("camp_txd"),
            clip_dictionary: name_hash("camp_clips"),
            drawable_dictionary: name_hash("camp_dict"),
            physics_dictionary: name_hash("camp_phys"),
            asset_type: 1,
            asset_name: name_hash("camp_asset"),
            extensions: vec![Extension::AudioCollision {
                name: 7,
                offset: [0.0; 3],
                settings: 9,
            }],
            ..ArchetypeSpec::base("camp_chair")
        };
        let bytes = YtypBuilder::new("camp_props")
            .archetype(ArchetypeSpec::time("camp_lamp", 0x00F0_000F))
            .archetype(spec.clone())
            .build();

        let map_types = read(&bytes).unwrap();
        assert_eq!(map_types.name, name_hash("camp_props"));
        assert_eq!(map_types.patch_refusal, None);
        assert_eq!(map_types.skipped, [] as [Skipped; 0]);
        let [lamp, chair] = map_types.archetypes.as_slice() else {
            panic!("two archetypes")
        };
        assert_eq!(
            (lamp.index, lamp.kind, lamp.time_flags),
            (0, ArchetypeKind::Time, Some(0x00F0_000F))
        );
        assert_eq!(chair.index, 1);
        assert_eq!(chair.kind, ArchetypeKind::Base);
        assert_eq!(chair.name, name_hash("camp_chair"));
        assert_eq!(chair.lod_dist, 250.0);
        assert_eq!((chair.flags, chair.special_attribute), (0x0002_0000, 31));
        assert_eq!(
            (chair.bb_min, chair.bb_max, chair.bs_centre),
            (spec.bb_min, spec.bb_max, spec.bs_centre)
        );
        assert_eq!((chair.bs_radius, chair.hd_texture_dist), (3.5, 40.0));
        assert_eq!(chair.texture_dictionary, name_hash("camp_txd"));
        assert_eq!(chair.clip_dictionary, name_hash("camp_clips"));
        assert_eq!(chair.drawable_dictionary, name_hash("camp_dict"));
        assert_eq!(chair.physics_dictionary, name_hash("camp_phys"));
        assert_eq!((chair.asset_type, chair.asset_name), (1, name_hash("camp_asset")));
        assert_eq!((chair.extension_count, chair.time_flags), (1, None));
        assert!(chair.digest.is_some() && chair.major_digest.is_some());
    }

    #[test]
    fn archetypes_spread_over_several_blocks_keep_the_list_order() {
        let mut builder = YtypBuilder::new("weed").archetypes_per_block(3);
        let all: Vec<String> = (0..8).map(|index| format!("weed_bud_{index}")).collect();
        for name in &all {
            builder = builder.archetype(ArchetypeSpec::base(name));
        }
        let map_types = read(&builder.build()).unwrap();
        let expected: Vec<u32> = all.iter().map(|name| name_hash(name)).collect();
        assert_eq!(names(&map_types), expected);
    }

    #[test]
    fn entries_of_other_types_are_skipped_with_their_position() {
        let bytes = YtypBuilder::new("odd")
            .archetype(ArchetypeSpec::base("a"))
            .unknown_entry()
            .archetype(ArchetypeSpec::base("b"))
            .build();
        let map_types = read(&bytes).unwrap();
        assert_eq!(names(&map_types), hashes(&["a", "b"]));
        assert_eq!(map_types.archetypes[1].index, 2);
        assert_eq!(
            map_types.skipped,
            [Skipped {
                index: 1,
                structure: crate::testing::UNKNOWN_STRUCTURE
            }]
        );
    }

    #[test]
    fn a_file_with_no_archetypes_reads_empty() {
        let map_types = read(&YtypBuilder::new("empty").build()).unwrap();
        assert_eq!(map_types.archetypes, [] as [Archetype; 0]);
        assert_eq!(map_types.patch_refusal, None);
    }

    #[test]
    fn removing_archetypes_keeps_the_others_in_order_and_unchanged() {
        let bytes = camp();
        let before = read(&bytes).unwrap();

        let patched = remove_archetypes(&bytes, &[3, 1]).unwrap();
        assert_eq!(patched[..16], bytes[..16], "the header stays as it was");
        let after = read(&patched).unwrap();
        assert_eq!(
            names(&after),
            hashes(&["camp_food_marshmallow", "camp_lamp", "camp_tent"])
        );
        assert_eq!(
            after
                .archetypes
                .iter()
                .map(|archetype| archetype.index)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(after.archetypes[1].digest, before.archetypes[2].digest);
        assert_eq!(after.archetypes[2].digest, before.archetypes[4].digest);
    }

    #[test]
    fn removing_nothing_gives_a_file_that_reads_the_same() {
        let bytes = camp();
        let patched = remove_archetypes(&bytes, &[]).unwrap();
        assert_eq!(read(&patched).unwrap(), read(&bytes).unwrap());
    }

    #[test]
    fn removing_every_archetype_leaves_an_empty_list() {
        let bytes = YtypBuilder::new("gift")
            .archetype(ArchetypeSpec::base("prop_gift"))
            .archetype(ArchetypeSpec::base("prop_gift2"))
            .build();
        let patched = remove_archetypes(&bytes, &[0, 1]).unwrap();
        let after = read(&patched).unwrap();
        assert_eq!(after.archetypes, [] as [Archetype; 0]);

        let resource = Resource::read(&patched).unwrap();
        let meta = Meta::parse(resource.system()).unwrap();
        let map_types = meta.block_bytes(meta.first_block_of(MAP_TYPES).unwrap()).unwrap();
        assert_eq!(
            map_types[ARCHETYPES_AT..ARCHETYPES_AT + 16],
            [0; 16],
            "pointer, count and capacity zeroed"
        );
    }

    #[test]
    fn removed_slots_are_zeroed_and_nothing_else_moves() {
        let bytes = camp();
        let patched = remove_archetypes(&bytes, &[0]).unwrap();
        let before = Resource::read(&bytes).unwrap();
        let after = Resource::read(&patched).unwrap();
        let meta = Meta::parse(before.system()).unwrap();
        let list = List::locate(&meta).unwrap();
        let list_range = list.pointers_at..list.pointers_at + 5 * POINTER_LEN;
        let count_range = list.map_types_at + ARCHETYPES_AT + 8..list.map_types_at + ARCHETYPES_AT + 12;

        let changed: Vec<usize> = (0..before.system().len())
            .filter(|&at| before.system()[at] != after.system()[at])
            .collect();
        assert!(
            changed
                .iter()
                .all(|at| list_range.contains(at) || count_range.contains(at))
        );
        assert_eq!(
            after.system()[list_range.end - POINTER_LEN..list_range.end],
            [0; 8]
        );
    }

    #[test]
    fn bad_indices_are_refused() {
        let bytes = camp();
        assert_eq!(
            remove_archetypes(&bytes, &[5]),
            Err(Error::IndexOutOfRange { index: 5, count: 5 })
        );
        assert_eq!(
            remove_archetypes(&bytes, &[1, 2, 1]),
            Err(Error::DuplicateIndex(1))
        );
        let odd = YtypBuilder::new("odd")
            .archetype(ArchetypeSpec::base("a"))
            .unknown_entry()
            .build();
        assert_eq!(remove_archetypes(&odd, &[1]), Err(Error::NotAnArchetype(1)));
    }

    #[test]
    fn removing_around_an_entry_of_another_type_keeps_it() {
        let bytes = YtypBuilder::new("odd")
            .archetype(ArchetypeSpec::base("a"))
            .unknown_entry()
            .archetype(ArchetypeSpec::base("b"))
            .build();
        let after = read(&remove_archetypes(&bytes, &[0]).unwrap()).unwrap();
        assert_eq!(names(&after), hashes(&["b"]));
        assert_eq!(after.skipped[0].index, 0);
        assert_eq!(after.archetypes[0].index, 1);
    }

    #[test]
    fn a_misdescribed_archetype_size_is_read_but_never_patched() {
        let bytes = YtypBuilder::new("odd")
            .archetype(ArchetypeSpec::base("a"))
            .declared_size(ArchetypeKind::Base, 160)
            .build();
        let map_types = read(&bytes).unwrap();
        assert_eq!(names(&map_types), hashes(&["a"]));
        assert!(map_types.patch_refusal.is_some());
        assert!(matches!(
            remove_archetypes(&bytes, &[0]),
            Err(Error::NotPatchable(_))
        ));
    }

    #[test]
    fn a_file_without_structure_descriptions_reads_through_the_usual_layout() {
        let bytes = YtypBuilder::new("bare")
            .archetype(ArchetypeSpec::base("a"))
            .archetype(ArchetypeSpec::time("b", 3))
            .without_structure_info(ArchetypeKind::Base)
            .without_structure_info(ArchetypeKind::Time)
            .build();
        let map_types = read(&bytes).unwrap();
        assert_eq!(names(&map_types), hashes(&["a", "b"]));
        assert_eq!(map_types.archetypes[1].time_flags, Some(3));
        assert!(
            map_types
                .archetypes
                .iter()
                .all(|archetype| archetype.digest.is_none())
        );
        let patched = remove_archetypes(&bytes, &[0]).unwrap();
        assert_eq!(names(&read(&patched).unwrap()), hashes(&["b"]));
    }

    #[test]
    fn a_list_outside_one_pointer_block_is_refused() {
        let bytes = camp();
        // The list's count raised past its block's end.
        let long = altered(&bytes, |meta, system| {
            let at = meta.block(meta.first_block_of(MAP_TYPES).unwrap()).unwrap().start + ARCHETYPES_AT + 8;
            system[at..at + 4].copy_from_slice(&[64, 0, 64, 0]);
        });
        assert!(matches!(read(&long), Err(Error::Malformed(_))));
        assert!(remove_archetypes(&long, &[]).is_err());

        // The list's pointer aimed at an archetype block instead.
        let elsewhere = altered(&bytes, |meta, system| {
            let at = meta.block(meta.first_block_of(MAP_TYPES).unwrap()).unwrap().start + ARCHETYPES_AT;
            let base = meta.first_block_of(ArchetypeKind::Base.structure()).unwrap();
            system[at..at + 8].copy_from_slice(
                &Pointer {
                    block: base,
                    offset: 0,
                }
                .pack()
                .to_le_bytes(),
            );
        });
        assert!(matches!(read(&elsewhere), Err(Error::Malformed(_))));
    }

    /// Where the record of block `id` lies in the block table.
    fn block_record(system: &[u8], id: usize) -> usize {
        usize::try_from(u64_at(system, 0x30).unwrap() & 0x0FFF_FFFF).unwrap() + 16 * (id - 1)
    }

    #[test]
    fn the_list_is_read_from_the_root_block_alone() {
        let bytes = camp();
        let refused = |bytes: &[u8]| {
            matches!(read(bytes), Err(Error::Malformed(_)))
                && matches!(remove_archetypes(bytes, &[0]), Err(Error::Malformed(_)))
        };
        // The root names an archetype block, no block, or a block past the table.
        for root in [2u32, 0, 99] {
            let elsewhere = altered(&bytes, |_, system| {
                system[0x1C..0x20].copy_from_slice(&root.to_le_bytes());
            });
            assert!(refused(&elsewhere), "root {root}");
        }
        // A second CMapTypes block, the root left on the first, then moved to the second.
        let base =
            |meta: &Meta<'_>| usize::from(meta.first_block_of(ArchetypeKind::Base.structure()).unwrap());
        let second = altered(&bytes, |meta, system| {
            let record = block_record(system, base(meta));
            system[record..record + 4].copy_from_slice(&MAP_TYPES.to_le_bytes());
        });
        assert!(refused(&second));
        let moved = altered(&bytes, |meta, system| {
            let record = block_record(system, base(meta));
            system[record..record + 4].copy_from_slice(&MAP_TYPES.to_le_bytes());
            let root = u32::try_from(base(meta)).unwrap();
            system[0x1C..0x20].copy_from_slice(&root.to_le_bytes());
        });
        assert!(refused(&moved));
        assert_eq!(names(&read(&bytes).unwrap()).len(), 5, "the untouched file reads");
    }

    #[test]
    fn hostile_list_entries_are_errors() {
        let bytes = camp();
        let entry = |raw: u64| {
            altered(&bytes, move |meta, system| {
                let list = List::locate(meta).unwrap();
                system[list.pointers_at..list.pointers_at + 8].copy_from_slice(&raw.to_le_bytes());
            })
        };
        assert!(matches!(read(&entry(0)), Err(Error::Malformed(_))));
        assert!(matches!(read(&entry(0xFFF)), Err(Error::Malformed(_))));
        assert!(matches!(read(&entry(0xFFFF_F002)), Err(Error::Malformed(_))));
    }

    #[test]
    fn every_truncation_and_byte_flip_is_an_answer_not_a_panic() {
        let bytes = YtypBuilder::new("camp_props")
            .archetype(ArchetypeSpec::base("camp_food_marshmallow"))
            .archetype(ArchetypeSpec {
                extensions: vec![Extension::ParticleEffect {
                    name: 3,
                    offset: [0.0; 3],
                    fx_name: "ent_amb_fire".to_owned(),
                    fx_type: 1,
                    scale: 1.0,
                }],
                ..ArchetypeSpec::time("camp_lamp", 0x00F0_000F)
            })
            .archetype(ArchetypeSpec::mlo("camp_tent"))
            .build();
        for cut in 0..bytes.len() {
            assert!(read(&bytes[..cut]).is_err());
        }
        let resource = Resource::read(&bytes).unwrap();
        let system = resource.system();
        let used = Meta::parse(system)
            .unwrap()
            .blocks()
            .iter()
            .map(DataBlock::end)
            .max()
            .unwrap();
        for at in 0..used {
            for value in [0x00, 0xFF, 0x7F, system[at] ^ 0x01] {
                let mut flipped = system.to_vec();
                flipped[at] = value;
                if let Ok(map_types) = read_system(&flipped) {
                    let first: Vec<usize> = map_types
                        .archetypes
                        .iter()
                        .map(|archetype| archetype.index)
                        .take(1)
                        .collect();
                    let _ = patch(&mut flipped, &first);
                }
            }
        }
    }

    #[test]
    fn a_wrong_version_or_a_resource_without_metadata_is_refused() {
        let bytes = camp();
        let resource = Resource::read(&bytes).unwrap();
        let other = Resource::new(
            13,
            resource.system_flags(),
            resource.graphics_flags(),
            resource.system().to_vec(),
        )
        .unwrap()
        .to_bytes()
        .unwrap();
        assert_eq!(read(&other), Err(Error::Version(13)));
        let blank = Resource::new(2, 0x0002_0000, 0x2000_0000, vec![0; 8192])
            .unwrap()
            .to_bytes()
            .unwrap();
        assert_eq!(read(&blank), Err(Error::NotMeta));
        assert_eq!(
            read(b"FXAP\x01\0\0\0"),
            Err(Error::NotAResource(crate::rsc7::FileKind::Escrowed))
        );
    }

    #[test]
    fn differences_name_the_fields_or_what_the_archetype_leads_to() {
        let map_types = read(&camp()).unwrap();
        let first = &map_types.archetypes[0];
        assert_eq!(differences(first, &map_types.archetypes[3]), [] as [&str; 0]);

        let farther = Archetype {
            lod_dist: 900.0,
            bb_max: [5.0; 3],
            ..first.clone()
        };
        assert_eq!(differences(first, &farther), ["lodDist", "bbMax"]);

        let other_contents = Archetype {
            digest: Some([1; 32]),
            ..first.clone()
        };
        assert_eq!(differences(first, &other_contents), ["contents"]);
        let with_extension = Archetype {
            extension_count: 1,
            ..first.clone()
        };
        let other_extension = Archetype {
            digest: Some([1; 32]),
            ..with_extension.clone()
        };
        assert_eq!(differences(&with_extension, &other_extension), ["extensions"]);

        let incomparable = Archetype {
            digest: None,
            ..first.clone()
        };
        assert_eq!(differences(&incomparable, &incomparable), ["contents"]);
    }
}
