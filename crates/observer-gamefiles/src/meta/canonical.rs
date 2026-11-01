//! A digest of one structure that tells whether two files define the same thing, whichever tool wrote
//! them and wherever it put the bytes. The structure is walked through the file's own description:
//! its name, then its fields in offset order, each as its name hash, its type code and its value.
//! Padding and the eight bytes before the first field (a class pointer at run time) are never read.
//! Pointers are followed to the structure they lead to, arrays are taken element by element, strings
//! by their text, so block numbers and offsets never reach the digest.
//!
//! The major digest leaves out, at the top level only, the fields that tools recompute and authors
//! tune without changing what an archetype is: `lodDist`, the bounds and `hdTextureDist`. Two copies
//! with the same major digest differ in those at most. Nested structures keep every field: the
//! bounds of an interior's room are part of what it is.
//!
//! A structure the walk cannot follow (no description, an unknown type code, a pointer that leads
//! nowhere, too deep, or past the file's budget) has no digest, which only means "not comparable".

use sha2::{Digest, Sha256};

use crate::hash::joaat;
use crate::meta::data_type::{
    ARRAY, BOOL, BYTE, BYTE_ENUM, CHAR_POINTER, CHARS, FIXED_ARRAY, FLOAT, HASH, INT, INT_ENUM, INT_FLAGS,
    INT_FLAGS2, SHORT, SHORT_FLAGS, SIGNED_BYTE, SIGNED_INT, SIGNED_SHORT, STRUCTURE, STRUCTURE_POINTER,
    VECTOR3, VECTOR4,
};
use crate::meta::{Entry, Meta, POINTER_BLOCK, Pointer, StructureInfo, array_at, u64_at};

/// The fields the major digest leaves out, by the literal joaat of their names.
pub const MINOR_FIELDS: [u32; 6] = [
    joaat(b"lodDist"),
    joaat(b"bbMin"),
    joaat(b"bbMax"),
    joaat(b"bsCentre"),
    joaat(b"bsRadius"),
    joaat(b"hdTextureDist"),
];

/// How deep structures and arrays may nest below the one digested. An interior's entity and its
/// extension sit at depth 2.
const MAX_DEPTH: usize = 16;

/// The two digests of one structure: SHA-256 over its canonical bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digests {
    pub full: [u8; 32],
    /// Without the [`MINOR_FIELDS`] of the structure itself.
    pub major: [u8; 32],
}

/// Digests the structures of one file. They share a budget of canonical bytes, four times the size
/// of the system segment and a mebibyte more, so a file whose pointers lead to one large structure
/// many times costs a bounded time; a digest past the budget is `None`. Array elements are paid for
/// when fetched as well, and a digest that fails keeps what it spent.
#[derive(Debug)]
pub struct Digester<'m, 'a> {
    meta: &'m Meta<'a>,
    budget: usize,
}

impl<'m, 'a> Digester<'m, 'a> {
    pub fn new(meta: &'m Meta<'a>) -> Self {
        let budget = meta.system().len().saturating_mul(4).saturating_add(1 << 20);
        Self { meta, budget }
    }

    /// The digests of the structure `pointer` leads to, typed by the block it lands in.
    pub fn structure(&mut self, pointer: Pointer) -> Option<Digests> {
        let (block, bytes) = self.meta.pointed(pointer)?;
        let mut sink = Sink {
            full: Sha256::new(),
            major: Sha256::new(),
            in_major: true,
            budget: self.budget,
        };
        let walked = Walk { meta: self.meta }.structure(&mut sink, block.structure, bytes, 0, true);
        self.budget = sink.budget;
        walked?;
        Some(Digests {
            full: sink.full.finalize().into(),
            major: sink.major.finalize().into(),
        })
    }
}

/// Where canonical bytes go: both digests, or the full one alone inside a minor field.
struct Sink {
    full: Sha256,
    major: Sha256,
    in_major: bool,
    budget: usize,
}

impl Sink {
    /// Takes `len` from the budget without writing anything.
    fn charge(&mut self, len: usize) -> Option<()> {
        self.budget = self.budget.checked_sub(len)?;
        Some(())
    }

    fn put(&mut self, bytes: &[u8]) -> Option<()> {
        self.charge(bytes.len())?;
        self.full.update(bytes);
        if self.in_major {
            self.major.update(bytes);
        }
        Some(())
    }

    fn put_u32(&mut self, value: u32) -> Option<()> {
        self.put(&value.to_le_bytes())
    }

    /// Text up to its first NUL, after its length.
    fn put_text(&mut self, raw: &[u8]) -> Option<()> {
        let text = raw.split(|&byte| byte == 0).next().unwrap_or_default();
        self.put_u32(u32::try_from(text.len()).ok()?)?;
        self.put(text)
    }
}

struct Walk<'m, 'a> {
    meta: &'m Meta<'a>,
}

impl Walk<'_, '_> {
    /// The structure named `name` at the start of `bytes`.
    fn structure(&self, sink: &mut Sink, name: u32, bytes: &[u8], depth: usize, top: bool) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        let info = self.meta.structure(name)?;
        let bytes = bytes.get(..info.size)?;
        sink.put_u32(name)?;
        for entry in info.fields_by_offset() {
            if top {
                sink.in_major = !MINOR_FIELDS.contains(&entry.name);
            }
            sink.put_u32(entry.name)?;
            sink.put(&[entry.data_type])?;
            self.value(sink, info, entry, bytes, entry.offset, depth)?;
        }
        if top {
            sink.in_major = true;
        }
        Some(())
    }

    /// The value `entry` describes at `at` in `bytes`, `info` holding the entries arrays refer to.
    fn value(
        &self,
        sink: &mut Sink,
        info: &StructureInfo,
        entry: &Entry,
        bytes: &[u8],
        at: usize,
        depth: usize,
    ) -> Option<()> {
        // Arrays nest through here with no structure in between: an array whose elements the file
        // describes as arrays of the same kind, leading back to itself, would otherwise go on until
        // the stack runs out, which aborts the whole process.
        if depth > MAX_DEPTH {
            return None;
        }
        let fixed = |len: usize| bytes.get(at..at.checked_add(len)?);
        if let Some(len) = primitive_len(entry.data_type) {
            return sink.put(fixed(len)?);
        }
        match entry.data_type {
            VECTOR3 => sink.put(fixed(12)?),
            VECTOR4 => sink.put(fixed(16)?),
            CHARS => sink.put_text(fixed(usize::try_from(entry.reference_key).ok()?)?),
            CHAR_POINTER => self.char_pointer(sink, fixed(16)?),
            STRUCTURE => self.structure(sink, entry.reference_key, bytes.get(at..)?, depth + 1, false),
            STRUCTURE_POINTER => self.pointer(sink, fixed(8)?, depth),
            ARRAY => self.array(sink, info, entry, fixed(16)?, depth),
            FIXED_ARRAY => self.fixed_array(sink, info, entry, bytes.get(at..)?, depth),
            _ => None,
        }
    }

    fn char_pointer(&self, sink: &mut Sink, header: &[u8]) -> Option<()> {
        let header = array_at(header, 0)?;
        match header.pointer {
            None => sink.put(&[0]),
            Some(pointer) => {
                sink.put(&[1])?;
                let (_, bytes) = self.meta.pointed(pointer)?;
                sink.put_text(bytes.get(..usize::from(header.count))?)
            }
        }
    }

    fn pointer(&self, sink: &mut Sink, raw: &[u8], depth: usize) -> Option<()> {
        match Pointer::unpack(u64_at(raw, 0)?) {
            None => sink.put(&[0]),
            Some(pointer) => {
                sink.put(&[1])?;
                let (block, bytes) = self.meta.pointed(pointer)?;
                self.structure(sink, block.structure, bytes, depth + 1, false)
            }
        }
    }

    fn array(
        &self,
        sink: &mut Sink,
        info: &StructureInfo,
        entry: &Entry,
        header: &[u8],
        depth: usize,
    ) -> Option<()> {
        let header = array_at(header, 0)?;
        sink.put_u32(u32::from(header.count))?;
        if header.count == 0 {
            return Some(());
        }
        let element = info.entries.get(usize::from(entry.reference_index))?;
        let block_type = match element.data_type {
            STRUCTURE_POINTER => Some(POINTER_BLOCK),
            STRUCTURE => Some(element.reference_key),
            _ => None,
        };
        // Fetching the elements is paid for before it is done: an array whose first element cannot
        // be digested would otherwise be fetched in full again at every level of nesting and for
        // every archetype leading to it, with nothing taken from the budget.
        sink.charge(usize::from(header.count))?;
        let items = self.meta.array_items(header, self.stride(element)?, block_type)?;
        for item in items {
            self.value(sink, info, element, item, 0, depth + 1)?;
        }
        Some(())
    }

    fn fixed_array(
        &self,
        sink: &mut Sink,
        info: &StructureInfo,
        entry: &Entry,
        bytes: &[u8],
        depth: usize,
    ) -> Option<()> {
        let element = info.entries.get(usize::from(entry.reference_index))?;
        let count = usize::try_from(entry.reference_key).ok()?;
        let stride = self.stride(element).filter(|&stride| stride > 0)?;
        let items = bytes.get(..count.checked_mul(stride)?)?;
        sink.put_u32(entry.reference_key)?;
        for item in items.chunks_exact(stride) {
            self.value(sink, info, element, item, 0, depth + 1)?;
        }
        Some(())
    }

    /// The bytes one element of the type `element` describes takes in an array.
    fn stride(&self, element: &Entry) -> Option<usize> {
        if let Some(len) = primitive_len(element.data_type) {
            return Some(len);
        }
        match element.data_type {
            STRUCTURE_POINTER => Some(8),
            VECTOR3 | VECTOR4 | CHAR_POINTER | ARRAY => Some(16),
            CHARS => usize::try_from(element.reference_key).ok(),
            STRUCTURE => Some(self.meta.structure(element.reference_key)?.size),
            _ => None,
        }
    }
}

/// The length of the numbers, enums and flags read as they are.
const fn primitive_len(data_type: u8) -> Option<usize> {
    match data_type {
        BOOL | SIGNED_BYTE | BYTE | BYTE_ENUM => Some(1),
        SIGNED_SHORT | SHORT | SHORT_FLAGS => Some(2),
        SIGNED_INT | INT | FLOAT | HASH | INT_ENUM | INT_FLAGS | INT_FLAGS2 => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rsc7::Resource;
    use crate::testing::{ArchetypeSpec, Extension, YtypBuilder};
    use crate::ytyp::ArchetypeKind;

    /// The digests of every archetype of a synthetic file, in list order.
    fn digests(bytes: &[u8]) -> Vec<Option<Digests>> {
        let resource = Resource::read(bytes).unwrap();
        let meta = Meta::parse(resource.system()).unwrap();
        let map_types = meta
            .block_bytes(meta.first_block_of(crate::ytyp::MAP_TYPES).unwrap())
            .unwrap();
        let header = array_at(map_types, 24).unwrap();
        let list = meta.array_items(header, 8, Some(POINTER_BLOCK)).unwrap();
        let mut digester = Digester::new(&meta);
        list.iter()
            .map(|raw| digester.structure(Pointer::unpack(u64_at(raw, 0).unwrap()).unwrap()))
            .collect()
    }

    fn one(spec: ArchetypeSpec) -> Digests {
        digests(&YtypBuilder::new("file").archetype(spec).build())[0].unwrap()
    }

    /// Where the entries of the description of `name` start in the system segment.
    fn entries_at(meta: &Meta<'_>, name: u32) -> usize {
        let index = meta
            .structures()
            .iter()
            .position(|info| info.name == name)
            .unwrap();
        let structures_at = usize::try_from(u64_at(meta.system(), 0x20).unwrap() & 0x0FFF_FFFF).unwrap();
        usize::try_from(u64_at(meta.system(), structures_at + 32 * index + 0x10).unwrap() & 0x0FFF_FFFF)
            .unwrap()
    }

    #[test]
    fn identical_archetypes_digest_the_same_wherever_they_lie() {
        let spec = ArchetypeSpec::base("prop_gift");
        let alone = YtypBuilder::new("a").archetype(spec.clone()).build();
        let among = YtypBuilder::new("b")
            .archetype(ArchetypeSpec::time("prop_lamp", 0x00FF_FF00))
            .archetype(ArchetypeSpec::base("prop_other"))
            .archetype(spec)
            .build();

        assert_eq!(digests(&alone)[0], digests(&among)[2]);
        assert!(digests(&alone)[0].is_some());
    }

    #[test]
    fn a_changed_lod_distance_changes_the_full_digest_only() {
        let spec = ArchetypeSpec::base("prop_gift");
        let near = one(spec.clone());
        let far = one(ArchetypeSpec {
            lod_dist: spec.lod_dist * 2.0,
            ..spec.clone()
        });
        assert_ne!(near.full, far.full);
        assert_eq!(near.major, far.major);

        let bounds = one(ArchetypeSpec {
            bb_max: [9.0, 9.0, 9.0],
            bs_radius: 12.0,
            ..spec
        });
        assert_ne!(near.full, bounds.full);
        assert_eq!(near.major, bounds.major);
    }

    #[test]
    fn a_changed_asset_name_changes_both_digests() {
        let spec = ArchetypeSpec::base("prop_gift");
        let before = one(spec.clone());
        let after = one(ArchetypeSpec {
            asset_name: crate::hash::name_hash("prop_gift_b"),
            ..spec
        });
        assert_ne!(before.full, after.full);
        assert_ne!(before.major, after.major);
    }

    #[test]
    fn extensions_are_followed_and_their_text_counts() {
        let spec = ArchetypeSpec::base("prop_fire");
        let effect = |fx_name: &str| Extension::ParticleEffect {
            name: crate::hash::name_hash("fire"),
            offset: [0.0, 0.0, 1.0],
            fx_name: fx_name.to_owned(),
            fx_type: 2,
            scale: 1.0,
        };
        let plain = one(spec.clone());
        let fire = one(ArchetypeSpec {
            extensions: vec![effect("ent_amb_fire")],
            ..spec.clone()
        });
        let smoke = one(ArchetypeSpec {
            extensions: vec![effect("ent_amb_smoke")],
            ..spec.clone()
        });
        let fire_again = one(ArchetypeSpec {
            extensions: vec![effect("ent_amb_fire")],
            lod_dist: 300.0,
            ..spec
        });

        assert_ne!(plain.full, fire.full);
        assert_ne!(fire.full, smoke.full);
        assert_ne!(fire.major, smoke.major);
        assert_eq!(fire.major, fire_again.major);
    }

    #[test]
    fn interiors_and_time_archetypes_digest_too() {
        let bytes = YtypBuilder::new("shell")
            .archetype(ArchetypeSpec::mlo("shell_interior"))
            .archetype(ArchetypeSpec::time("shell_lamp", 0x0000_0F00))
            .build();
        assert!(digests(&bytes).iter().all(Option::is_some));
    }

    #[test]
    fn an_undescribed_structure_has_no_digest() {
        let bytes = YtypBuilder::new("file")
            .archetype(ArchetypeSpec::base("described"))
            .archetype(ArchetypeSpec::time("undescribed", 1))
            .without_structure_info(ArchetypeKind::Time)
            .build();
        let found = digests(&bytes);
        assert!(found[0].is_some());
        assert_eq!(found[1], None);
    }

    #[test]
    fn a_structure_leading_back_to_itself_stops_at_the_depth_limit() {
        // An extension whose description says its first field points to a structure of its own kind,
        // and a pointer there that leads back to it.
        let bytes = YtypBuilder::new("file")
            .archetype(ArchetypeSpec {
                extensions: vec![Extension::AudioCollision {
                    name: 1,
                    offset: [0.0; 3],
                    settings: 2,
                }],
                ..ArchetypeSpec::base("looped")
            })
            .build();
        let mut resource = Resource::read(&bytes).unwrap();
        let meta = Meta::parse(resource.system()).unwrap();
        let audio = crate::testing::AUDIO_COLLISION;
        let block = meta.first_block_of(audio).unwrap();
        let block_start = meta.block(block).unwrap().start;
        let entries_at = entries_at(&meta, audio);
        drop(meta);
        let system = resource.system_mut();
        // The `settings` entry (the third) becomes a structure pointer at offset 32...
        system[entries_at + 2 * 16 + 8] = STRUCTURE_POINTER;
        // ...and the value there points at the extension itself.
        let pointer = Pointer { block, offset: 0 }.pack();
        system[block_start + 32..block_start + 40].copy_from_slice(&pointer.to_le_bytes());

        assert_eq!(digests(&resource.to_bytes().unwrap())[0], None);
    }

    #[test]
    fn an_array_of_arrays_leading_back_to_itself_stops_at_the_depth_limit() {
        // The element descriptor of `extensions` says its elements are arrays it describes itself,
        // and the archetype's extension header leads back to that header: every level reads it again.
        let bytes = YtypBuilder::new("file")
            .archetype(ArchetypeSpec::base("looped"))
            .build();
        let mut resource = Resource::read(&bytes).unwrap();
        let meta = Meta::parse(resource.system()).unwrap();
        let base = ArchetypeKind::Base.structure();
        let extensions = meta.structure(base).unwrap().field(joaat(b"extensions")).unwrap();
        let (offset, element) = (extensions.offset, extensions.reference_index);
        let element_at = entries_at(&meta, base) + 16 * usize::from(element);
        let block = meta.first_block_of(base).unwrap();
        let header_at = meta.block(block).unwrap().start + offset;
        drop(meta);
        let system = resource.system_mut();
        system[element_at + 8] = ARRAY;
        system[element_at + 10..element_at + 12].copy_from_slice(&element.to_le_bytes());
        let pointer = Pointer {
            block,
            offset: u32::try_from(offset).unwrap(),
        };
        system[header_at..header_at + 8].copy_from_slice(&pointer.pack().to_le_bytes());
        system[header_at + 8..header_at + 12].copy_from_slice(&[1, 0, 1, 0]);
        let looped = resource.to_bytes().unwrap();

        assert_eq!(digests(&looped)[0], None);
        let map_types = crate::ytyp::read(&looped).unwrap();
        assert_eq!(map_types.archetypes[0].name, crate::hash::name_hash("looped"));
        assert_eq!(map_types.archetypes[0].digest, None);
    }

    /// A file of 64 props whose first one's extension list is the archetype list itself, cut to
    /// `count` entries: the digest of that prop leads to itself until the depth limit.
    fn looped_through_the_list(count: u16) -> Vec<u8> {
        let builder = (0..64).fold(YtypBuilder::new("file"), |builder, index| {
            builder.archetype(ArchetypeSpec::base(&format!("prop_{index}")))
        });
        let mut resource = Resource::read(&builder.build()).unwrap();
        let meta = Meta::parse(resource.system()).unwrap();
        let map_types = meta
            .block(meta.first_block_of(crate::ytyp::MAP_TYPES).unwrap())
            .unwrap();
        let list = u64_at(meta.system(), map_types.start + 24).unwrap();
        let first = meta.block(meta.first_block_of(ArchetypeKind::Base.structure()).unwrap());
        let header_at = first.unwrap().start + 120;
        drop(meta);
        let system = resource.system_mut();
        system[header_at..header_at + 8].copy_from_slice(&list.to_le_bytes());
        system[header_at + 8..header_at + 10].copy_from_slice(&count.to_le_bytes());
        system[header_at + 10..header_at + 12].copy_from_slice(&count.to_le_bytes());
        resource.to_bytes().unwrap()
    }

    /// How much of the file's budget the failed digest of its first archetype takes.
    fn spent_on_the_first(bytes: &[u8]) -> usize {
        let resource = Resource::read(bytes).unwrap();
        let meta = Meta::parse(resource.system()).unwrap();
        let map_types = meta
            .block_bytes(meta.first_block_of(crate::ytyp::MAP_TYPES).unwrap())
            .unwrap();
        let list = meta
            .array_items(array_at(map_types, 24).unwrap(), 8, Some(POINTER_BLOCK))
            .unwrap();
        let first = Pointer::unpack(u64_at(list[0], 0).unwrap()).unwrap();
        let mut digester = Digester::new(&meta);
        let before = digester.budget;
        assert_eq!(digester.structure(first), None);
        before - digester.budget
    }

    #[test]
    fn the_elements_of_an_array_are_paid_for_even_when_the_digest_fails() {
        // Only the first element is ever walked before the digest fails at the depth limit; the
        // other 63 are fetched at every level all the same, and the budget must say so.
        let one = spent_on_the_first(&looped_through_the_list(1));
        let all = spent_on_the_first(&looped_through_the_list(64));
        assert!(all >= one + 63 * 8, "{one} then {all}");
    }

    #[test]
    fn minor_fields_are_the_literal_hashes_of_their_names() {
        assert_eq!(MINOR_FIELDS[0], 0xBF74_DFA7);
        assert_eq!(MINOR_FIELDS[5], 0xAD5D_5B4C);
    }
}
