//! The binary metadata (RBM, marked `PRD0`) that RSC7 version 2 resources such as `.ytyp` carry in
//! their system segment: a 0x70-byte header, a table of data blocks, and the file's own description
//! of every structure its blocks hold. That description says where each field lies, so a file is read
//! the same way whichever tool wrote it.
//!
//! The header reaches its tables by virtual address, `0x5xxxxxxx` being an offset in the system
//! segment. Inside the blocks, pointers are packed instead: the 1-based block number in bits 0-11 and
//! the offset in that block in bits 12-31. The bytes come from servers' resources, not from the game,
//! so every table, block and read is checked against the segment it lies in.

pub mod canonical;

use std::collections::HashMap;

use crate::Error;

/// `PRD0`, at 0x10 of the header.
const MARKER: u32 = 0x5052_4430;
const HEADER_LEN: usize = 0x70;
const BLOCK_LEN: usize = 16;
const STRUCTURE_LEN: usize = 32;
const ENTRY_LEN: usize = 16;

/// The name of an entry that is no field but describes the elements of the array entry that uses
/// it as its reference.
pub const ARRAY_INFO: u32 = 0x100;
/// The block type of arrays of packed pointers.
pub const POINTER_BLOCK: u32 = 0x07;
/// The block type of the strings character pointers lead to.
pub const STRING_BLOCK: u32 = 0x10;

/// The type codes of structure entries, as files write them.
pub mod data_type {
    pub const BOOL: u8 = 0x01;
    /// A structure stored inline, named by the entry's reference key.
    pub const STRUCTURE: u8 = 0x05;
    /// A packed pointer to a structure, typed by the block it leads to.
    pub const STRUCTURE_POINTER: u8 = 0x07;
    pub const SIGNED_BYTE: u8 = 0x10;
    pub const BYTE: u8 = 0x11;
    pub const SIGNED_SHORT: u8 = 0x12;
    pub const SHORT: u8 = 0x13;
    pub const SIGNED_INT: u8 = 0x14;
    pub const INT: u8 = 0x15;
    pub const FLOAT: u8 = 0x21;
    /// Three floats in 16 bytes.
    pub const VECTOR3: u8 = 0x33;
    pub const VECTOR4: u8 = 0x34;
    /// Characters stored inline, as many as the reference key says.
    pub const CHARS: u8 = 0x40;
    /// An array header leading to the characters of a string.
    pub const CHAR_POINTER: u8 = 0x44;
    pub const HASH: u8 = 0x4A;
    /// Elements stored inline, as many as the reference key says, described by the entry the
    /// reference index names.
    pub const FIXED_ARRAY: u8 = 0x50;
    /// An array header (packed pointer, count, capacity), its elements described by the entry the
    /// reference index names.
    pub const ARRAY: u8 = 0x52;
    pub const DATA_BLOCK_POINTER: u8 = 0x59;
    pub const BYTE_ENUM: u8 = 0x60;
    pub const INT_ENUM: u8 = 0x62;
    pub const INT_FLAGS: u8 = 0x63;
    pub const SHORT_FLAGS: u8 = 0x64;
    pub const INT_FLAGS2: u8 = 0x65;
}

/// A data block: a run of the system segment holding values of one type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataBlock {
    /// The name hash of the structure the block holds, or a primitive type code such as
    /// [`POINTER_BLOCK`].
    pub structure: u32,
    /// Where the block starts in the system segment.
    pub start: usize,
    pub len: usize,
}

impl DataBlock {
    /// Where the block ends in the system segment.
    pub const fn end(&self) -> usize {
        self.start + self.len
    }
}

/// How the file describes one structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureInfo {
    /// The literal joaat of the structure's name.
    pub name: u32,
    pub key: u32,
    pub size: usize,
    pub entries: Vec<Entry>,
}

impl StructureInfo {
    /// The field named `name`; array element descriptors are no fields.
    pub fn field(&self, name: u32) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.name == name && name != ARRAY_INFO)
    }

    /// The fields in the order they lie in the structure, array element descriptors left out.
    pub fn fields_by_offset(&self) -> Vec<&Entry> {
        let mut fields: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.name != ARRAY_INFO)
            .collect();
        fields.sort_by_key(|entry| entry.offset);
        fields
    }
}

/// One entry of a structure's description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The literal joaat of the field's name, or [`ARRAY_INFO`].
    pub name: u32,
    /// Where the field lies in the structure.
    pub offset: usize,
    /// One of the [`data_type`] codes.
    pub data_type: u8,
    /// For arrays, the index in the same description of the entry describing the elements.
    pub reference_index: u16,
    /// A structure or enum name hash, or a count, depending on the type.
    pub reference_key: u32,
}

/// A packed pointer inside block data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pointer {
    /// The 1-based block number.
    pub block: u16,
    /// The offset in that block.
    pub offset: u32,
}

impl Pointer {
    /// Reads a packed pointer; `None` when it is null (block 0). The upper 32 bits are unused.
    pub fn unpack(raw: u64) -> Option<Self> {
        let block = u16::try_from(raw & 0xFFF).ok().filter(|&block| block != 0)?;
        let offset = u32::try_from((raw >> 12) & 0xF_FFFF).ok()?;
        Some(Self { block, offset })
    }

    /// The pointer packed again.
    pub fn pack(self) -> u64 {
        u64::from(self.block & 0xFFF) | (u64::from(self.offset & 0xF_FFFF) << 12)
    }
}

/// The 16-byte header of an array: a packed pointer, the count and the capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayHeader {
    pub pointer: Option<Pointer>,
    pub count: u16,
    pub capacity: u16,
}

pub fn u8_at(bytes: &[u8], at: usize) -> Option<u8> {
    bytes.get(at).copied()
}

pub fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(take(bytes, at)?))
}

pub fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(take(bytes, at)?))
}

pub fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(take(bytes, at)?))
}

pub fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(take(bytes, at)?))
}

/// Three floats from `at`, the fourth lane of a vector left aside.
pub fn vector3_at(bytes: &[u8], at: usize) -> Option<[f32; 3]> {
    Some([
        f32_at(bytes, at)?,
        f32_at(bytes, at.checked_add(4)?)?,
        f32_at(bytes, at.checked_add(8)?)?,
    ])
}

pub fn array_at(bytes: &[u8], at: usize) -> Option<ArrayHeader> {
    Some(ArrayHeader {
        pointer: Pointer::unpack(u64_at(bytes, at)?),
        count: u16_at(bytes, at.checked_add(8)?)?,
        capacity: u16_at(bytes, at.checked_add(10)?)?,
    })
}

fn take<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}

/// The offset in the system segment a virtual address stands for: `0x5xxxxxxx` only, since
/// binary metadata keeps nothing in the graphics segment.
fn system_offset(address: u64) -> Option<usize> {
    if address >> 28 == 5 {
        usize::try_from(address & 0x0FFF_FFFF).ok()
    } else {
        None
    }
}

/// Binary metadata parsed from a system segment: its blocks and structure descriptions, with the
/// segment kept to read values from.
#[derive(Debug, Clone)]
pub struct Meta<'a> {
    system: &'a [u8],
    root_block: u32,
    blocks: Vec<DataBlock>,
    structures: Vec<StructureInfo>,
    by_name: HashMap<u32, usize>,
}

impl<'a> Meta<'a> {
    /// Parses the header and tables of `system`. Every table and every block must lie inside the
    /// segment; values inside blocks are only read, and checked, when asked for.
    pub fn parse(system: &'a [u8]) -> Result<Self, Error> {
        if system.len() < HEADER_LEN || u32_at(system, 0x10) != Some(MARKER) {
            return Err(Error::NotMeta);
        }
        let header = |at| u64_at(system, at).ok_or(Error::NotMeta);
        let count = |at| u16_at(system, at).map(usize::from).ok_or(Error::NotMeta);
        let root_block = u32_at(system, 0x1C).ok_or(Error::NotMeta)?;

        let blocks_at = table(system, header(0x30)?, count(0x4C)?, BLOCK_LEN)?;
        let blocks = blocks_at
            .map(|at| {
                let start = system_offset(u64_at(system, at + 8).unwrap_or_default());
                let len = u32_at(system, at + 4).and_then(|len| usize::try_from(len).ok());
                match (u32_at(system, at), start, len) {
                    (Some(structure), Some(start), Some(len))
                        if start.checked_add(len).is_some_and(|end| end <= system.len()) =>
                    {
                        Ok(DataBlock {
                            structure,
                            start,
                            len,
                        })
                    }
                    _ => Err(Error::Malformed("a data block lies outside the system segment")),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        let structures_at = table(system, header(0x20)?, count(0x48)?, STRUCTURE_LEN)?;
        let structures = structures_at
            .map(|at| structure_info(system, at))
            .collect::<Result<Vec<_>, _>>()?;
        let mut by_name = HashMap::with_capacity(structures.len());
        for (index, structure) in structures.iter().enumerate() {
            by_name.entry(structure.name).or_insert(index);
        }

        Ok(Self {
            system,
            root_block,
            blocks,
            structures,
            by_name,
        })
    }

    /// The system segment the metadata was parsed from.
    pub const fn system(&self) -> &'a [u8] {
        self.system
    }

    /// The 1-based number of the root block, as the header gives it.
    pub const fn root_block(&self) -> u32 {
        self.root_block
    }

    pub fn blocks(&self) -> &[DataBlock] {
        &self.blocks
    }

    /// The block numbered `id`, from 1.
    pub fn block(&self, id: u16) -> Option<&DataBlock> {
        self.blocks.get(usize::from(id.checked_sub(1)?))
    }

    /// The bytes of the block numbered `id`.
    pub fn block_bytes(&self, id: u16) -> Option<&'a [u8]> {
        let block = self.block(id)?;
        self.system.get(block.start..block.end())
    }

    /// The number of the first block holding `structure`.
    pub fn first_block_of(&self, structure: u32) -> Option<u16> {
        let index = self
            .blocks
            .iter()
            .position(|block| block.structure == structure)?;
        u16::try_from(index + 1).ok()
    }

    pub fn structures(&self) -> &[StructureInfo] {
        &self.structures
    }

    /// How the file describes the structure named `name`, the first description if there are two.
    pub fn structure(&self, name: u32) -> Option<&StructureInfo> {
        self.structures.get(*self.by_name.get(&name)?)
    }

    /// The block `pointer` leads into and the offset in the system segment it leads to, which may be
    /// the block's end but not past it.
    pub fn resolve(&self, pointer: Pointer) -> Option<(&DataBlock, usize)> {
        let block = self.block(pointer.block)?;
        let offset = usize::try_from(pointer.offset).ok()?;
        (offset <= block.len).then_some((block, block.start + offset))
    }

    /// The block `pointer` leads into and its bytes from there to the block's end.
    pub fn pointed(&self, pointer: Pointer) -> Option<(&DataBlock, &'a [u8])> {
        let (block, at) = self.resolve(pointer)?;
        Some((block, self.system.get(at..block.end())?))
    }

    /// The elements of an array, `item_len` bytes each. An array that runs past its block goes on
    /// at the start of the next one if that block holds the same type, as writers split large
    /// arrays; bytes left at a block's end too few for an element are skipped. `block_type`, when
    /// given, is the type the first block must hold. `None` when the array leads nowhere.
    pub fn array_items(
        &self,
        header: ArrayHeader,
        item_len: usize,
        block_type: Option<u32>,
    ) -> Option<Vec<&'a [u8]>> {
        let count = usize::from(header.count);
        if count == 0 {
            return Some(Vec::new());
        }
        let pointer = header.pointer?;
        if item_len == 0 {
            return None;
        }
        let structure = self.block(pointer.block)?.structure;
        if block_type.is_some_and(|expected| expected != structure) {
            return None;
        }
        let mut items = Vec::with_capacity(count);
        let mut id = pointer.block;
        let mut offset = usize::try_from(pointer.offset).ok()?;
        loop {
            let block = self.block(id).filter(|block| block.structure == structure)?;
            let mut rest = self.system.get(block.start..block.end())?.get(offset..)?;
            while items.len() < count && rest.len() >= item_len {
                let (item, tail) = rest.split_at(item_len);
                items.push(item);
                rest = tail;
            }
            if items.len() == count {
                return Some(items);
            }
            id = id.checked_add(1)?;
            offset = 0;
        }
    }
}

/// The offsets of the `count` records of `len` bytes a header address leads to, all inside `system`.
fn table(
    system: &[u8],
    address: u64,
    count: usize,
    len: usize,
) -> Result<impl Iterator<Item = usize>, Error> {
    let start = if count == 0 {
        0
    } else {
        let start =
            system_offset(address).ok_or(Error::Malformed("a table lies outside the system segment"))?;
        let fits = count
            .checked_mul(len)
            .and_then(|size| start.checked_add(size))
            .is_some_and(|end| end <= system.len());
        if !fits {
            return Err(Error::Malformed("a table runs past the system segment"));
        }
        start
    };
    Ok((0..count).map(move |index| start + index * len))
}

fn structure_info(system: &[u8], at: usize) -> Result<StructureInfo, Error> {
    let malformed = || Error::Malformed("a structure description runs past the system segment");
    let name = u32_at(system, at).ok_or_else(malformed)?;
    let key = u32_at(system, at + 4).ok_or_else(malformed)?;
    let entries_address = u64_at(system, at + 0x10).ok_or_else(malformed)?;
    let size = u32_at(system, at + 0x18).ok_or_else(malformed)?;
    let size = usize::try_from(size).map_err(|_| malformed())?;
    let count = u16_at(system, at + 0x1E).map(usize::from).ok_or_else(malformed)?;
    let entries = table(system, entries_address, count, ENTRY_LEN)?
        .map(|at| {
            Some(Entry {
                name: u32_at(system, at)?,
                offset: usize::try_from(u32_at(system, at + 4)?).ok()?,
                data_type: u8_at(system, at + 8)?,
                reference_index: u16_at(system, at + 10)?,
                reference_key: u32_at(system, at + 12)?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(malformed)?;
    Ok(StructureInfo {
        name,
        key,
        size,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ArchetypeSpec, YtypBuilder};
    use crate::ytyp;

    fn system_of(bytes: &[u8]) -> Vec<u8> {
        crate::rsc7::Resource::read(bytes).unwrap().system().to_vec()
    }

    #[test]
    fn packed_pointers_carry_a_block_number_and_an_offset() {
        let pointer = Pointer::unpack(0x0005_0004).unwrap();
        assert_eq!(
            pointer,
            Pointer {
                block: 4,
                offset: 0x50
            }
        );
        assert_eq!(pointer.pack(), 0x0005_0004);
        assert_eq!(Pointer::unpack(0), None);
        assert_eq!(
            Pointer::unpack(0x0009_0000),
            None,
            "block 0 is null whatever the offset"
        );
        assert_eq!(
            Pointer::unpack(0xFFFF_FFFF_0000_0002),
            Some(Pointer { block: 2, offset: 0 })
        );
    }

    #[test]
    fn readers_stop_at_the_end_of_their_bytes() {
        let bytes = [1, 0, 0, 0, 0, 0, 0x80, 0x3F];
        assert_eq!(u32_at(&bytes, 0), Some(1));
        assert_eq!(f32_at(&bytes, 4), Some(1.0));
        assert_eq!(u32_at(&bytes, 5), None);
        assert_eq!(u64_at(&bytes, 1), None);
        assert_eq!(u16_at(&bytes, usize::MAX), None);
        assert_eq!(array_at(&bytes, 0), None);
    }

    #[test]
    fn a_synthetic_ytyp_parses_into_blocks_and_structure_descriptions() {
        let bytes = YtypBuilder::new("props")
            .archetype(ArchetypeSpec::base("prop_a"))
            .build();
        let system = system_of(&bytes);
        let meta = Meta::parse(&system).unwrap();

        assert_eq!(meta.root_block(), 1);
        let map_types = meta.first_block_of(ytyp::MAP_TYPES).unwrap();
        assert_eq!(map_types, 1);
        assert_eq!(meta.block_bytes(map_types).unwrap().len(), 80);
        let base = meta.structure(ytyp::ArchetypeKind::Base.structure()).unwrap();
        assert_eq!(base.size, 144);
        let name = base.field(crate::hash::joaat(b"name")).unwrap();
        assert_eq!((name.offset, name.data_type), (88, data_type::HASH));
        let offsets: Vec<usize> = base.fields_by_offset().iter().map(|entry| entry.offset).collect();
        assert!(offsets.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn a_segment_without_the_marker_is_not_metadata() {
        assert_eq!(Meta::parse(&[0; 0x70]).unwrap_err(), Error::NotMeta);
        assert_eq!(Meta::parse(&[]).unwrap_err(), Error::NotMeta);
    }

    #[test]
    fn tables_or_blocks_outside_the_segment_are_refused() {
        let bytes = YtypBuilder::new("props")
            .archetype(ArchetypeSpec::base("prop_a"))
            .build();
        let system = system_of(&bytes);

        let mut far_blocks = system.clone();
        far_blocks[0x30..0x38].copy_from_slice(&0x5FFF_FFF0u64.to_le_bytes());
        assert!(matches!(Meta::parse(&far_blocks), Err(Error::Malformed(_))));

        let mut graphics_blocks = system.clone();
        graphics_blocks[0x30..0x38].copy_from_slice(&0x6000_0100u64.to_le_bytes());
        assert!(matches!(Meta::parse(&graphics_blocks), Err(Error::Malformed(_))));

        let mut many_blocks = system.clone();
        many_blocks[0x4C..0x4E].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(Meta::parse(&many_blocks), Err(Error::Malformed(_))));

        let blocks = usize::try_from(u64_at(&system, 0x30).unwrap() & 0x0FFF_FFFF).unwrap();
        let mut long_block = system.clone();
        long_block[blocks + 4..blocks + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(Meta::parse(&long_block), Err(Error::Malformed(_))));
    }

    #[test]
    fn an_array_runs_on_into_the_next_block_of_its_type() {
        let bytes = YtypBuilder::new("props")
            .archetypes_per_block(2)
            .archetype(ArchetypeSpec::base("a"))
            .archetype(ArchetypeSpec::base("b"))
            .archetype(ArchetypeSpec::base("c"))
            .build();
        let system = system_of(&bytes);
        let meta = Meta::parse(&system).unwrap();
        let first = meta
            .first_block_of(ytyp::ArchetypeKind::Base.structure())
            .unwrap();
        let header = ArrayHeader {
            pointer: Some(Pointer {
                block: first,
                offset: 0,
            }),
            count: 3,
            capacity: 3,
        };

        let items = meta
            .array_items(header, 144, Some(ytyp::ArchetypeKind::Base.structure()))
            .unwrap();
        assert_eq!(items.len(), 3);
        let names: Vec<u32> = items.iter().map(|item| u32_at(item, 88).unwrap()).collect();
        let expected: Vec<u32> = ["a", "b", "c"].map(crate::hash::name_hash).to_vec();
        assert_eq!(names, expected);

        let too_many = ArrayHeader { count: 4, ..header };
        assert_eq!(meta.array_items(too_many, 144, None), None);
        assert_eq!(meta.array_items(header, 144, Some(POINTER_BLOCK)), None);
        assert_eq!(
            meta.array_items(
                ArrayHeader {
                    pointer: None,
                    ..header
                },
                144,
                None
            ),
            None
        );
        assert_eq!(meta.array_items(header, 0, None), None);
    }
}
