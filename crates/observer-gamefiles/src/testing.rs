//! Valid synthetic `.ytyp` files, for the tests of this crate and of the crates that build fake
//! servers on it. They are laid out the way `CodeWalker` lays out the files it writes, which real files
//! confirm: the 0x70-byte RBM header, the page description, the structure and enum descriptions,
//! the block table, then the blocks, 16-byte aligned, in one system segment deflated behind an RSC7
//! version 2 header. No game file is copied: every byte is written here from the published layouts.

use crate::hash::{joaat, name_hash};
use crate::meta::data_type::{
    ARRAY, CHAR_POINTER, FLOAT, HASH, INT, INT_ENUM, SIGNED_INT, STRUCTURE, STRUCTURE_POINTER, VECTOR3,
    VECTOR4,
};
use crate::meta::{ARRAY_INFO, POINTER_BLOCK, Pointer, STRING_BLOCK};
use crate::rsc7::{Resource, segment_size};
use crate::ytyp::{ArchetypeKind, MAP_TYPES};

/// The block type of the entries [`YtypBuilder::unknown_entry`] adds, which is no archetype.
pub const UNKNOWN_STRUCTURE: u32 = joaat(b"CSyntheticUnknownDef");
pub const AUDIO_COLLISION: u32 = joaat(b"CExtensionDefAudioCollisionSettings");
pub const PARTICLE_EFFECT: u32 = joaat(b"CExtensionDefParticleEffect");
const ASSET_TYPE: u32 = joaat(b"rage__fwArchetypeDef__eAssetType");
const ASSET_TYPES: [&[u8]; 5] = [
    b"ASSET_TYPE_UNINITIALIZED",
    b"ASSET_TYPE_FRAGMENT",
    b"ASSET_TYPE_DRAWABLE",
    b"ASSET_TYPE_DRAWABLEDICTIONARY",
    b"ASSET_TYPE_ASSETLESS",
];
/// `CodeWalker` starts a new block of a type past this many bytes.
const MAX_BLOCK_LEN: usize = 0x4000;
const HEADER_LEN: usize = 0x70;
/// The version nibble of binary metadata and no graphics pages.
const GRAPHICS_FLAGS: u32 = 0x2000_0000;
const UNKNOWN_LEN: usize = 16;

/// One archetype to write. The constructors give a plausible prop; every field can then be set.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchetypeSpec {
    pub kind: ArchetypeKind,
    /// The hash the client registers.
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
    pub asset_type: u32,
    pub asset_name: u32,
    /// Written for a time archetype only.
    pub time_flags: u32,
    /// Written for an interior only, whose entity, room, portal, entity set and time cycle modifier
    /// arrays are empty.
    pub mlo_flags: u32,
    pub extensions: Vec<Extension>,
}

impl ArchetypeSpec {
    /// A prop named `name`: a drawable of the same name with its own texture dictionary, in a box
    /// one metre wide.
    pub fn base(name: &str) -> Self {
        let hash = name_hash(name);
        Self {
            kind: ArchetypeKind::Base,
            name: hash,
            lod_dist: 100.0,
            flags: 0,
            special_attribute: 0,
            bb_min: [-0.5; 3],
            bb_max: [0.5; 3],
            bs_centre: [0.0; 3],
            bs_radius: 0.866,
            hd_texture_dist: 15.0,
            texture_dictionary: hash,
            clip_dictionary: 0,
            drawable_dictionary: 0,
            physics_dictionary: 0,
            asset_type: 2,
            asset_name: hash,
            time_flags: 0,
            mlo_flags: 0,
            extensions: Vec::new(),
        }
    }

    /// A prop shown at the hours `time_flags` gives.
    pub fn time(name: &str, time_flags: u32) -> Self {
        Self {
            kind: ArchetypeKind::Time,
            time_flags,
            ..Self::base(name)
        }
    }

    /// An empty interior.
    pub fn mlo(name: &str) -> Self {
        Self {
            kind: ArchetypeKind::Mlo,
            lod_dist: 300.0,
            bb_min: [-10.0; 3],
            bb_max: [10.0; 3],
            bs_radius: 17.32,
            asset_type: 4,
            ..Self::base(name)
        }
    }
}

/// An extension of an archetype, in a block of its own type that the archetype points to.
#[derive(Debug, Clone, PartialEq)]
pub enum Extension {
    /// `CExtensionDefAudioCollisionSettings`: three plain fields.
    AudioCollision {
        name: u32,
        offset: [f32; 3],
        settings: u32,
    },
    /// `CExtensionDefParticleEffect`, whose effect name is a string it points to.
    ParticleEffect {
        name: u32,
        offset: [f32; 3],
        fx_name: String,
        fx_type: i32,
        scale: f32,
    },
}

/// A `.ytyp` to build: its name, its archetype list, and how to lay it out.
#[derive(Debug, Clone)]
pub struct YtypBuilder {
    name: u32,
    entries: Vec<Option<ArchetypeSpec>>,
    per_block: Option<usize>,
    omitted: Vec<ArchetypeKind>,
    declared: Vec<(ArchetypeKind, usize)>,
}

impl YtypBuilder {
    /// A file whose `CMapTypes.name` is the hash of `name`, normally the file's stem.
    pub fn new(name: &str) -> Self {
        Self {
            name: name_hash(name),
            entries: Vec::new(),
            per_block: None,
            omitted: Vec::new(),
            declared: Vec::new(),
        }
    }

    /// Adds an archetype at the end of the list.
    #[must_use]
    pub fn archetype(mut self, spec: ArchetypeSpec) -> Self {
        self.entries.push(Some(spec));
        self
    }

    /// Adds an entry at the end of the list that leads to a block of [`UNKNOWN_STRUCTURE`].
    #[must_use]
    pub fn unknown_entry(mut self) -> Self {
        self.entries.push(None);
        self
    }

    /// Spreads the archetypes of each kind over blocks of at most `count`, as real files do past
    /// 16 KiB; by default a block holds as many as fit in 16 KiB.
    #[must_use]
    pub fn archetypes_per_block(mut self, count: usize) -> Self {
        self.per_block = Some(count.max(1));
        self
    }

    /// Leaves out the file's description of `kind`, as if a tool had not written it.
    #[must_use]
    pub fn without_structure_info(mut self, kind: ArchetypeKind) -> Self {
        self.omitted.push(kind);
        self
    }

    /// Describes `kind` with another size than its layout's, its blocks unchanged.
    #[must_use]
    pub fn declared_size(mut self, kind: ArchetypeKind, size: usize) -> Self {
        self.declared.push((kind, size));
        self
    }

    /// The file's bytes.
    ///
    /// # Panics
    ///
    /// Past 256 MiB, or past a million bytes of one block, which no test needs.
    pub fn build(&self) -> Vec<u8> {
        let system = self.system(&self.blocks());
        let flags = system_flags(system.len());
        let mut payload = system;
        payload.resize(
            usize::try_from(segment_size(flags)).expect("a synthetic ytyp stays small"),
            0,
        );
        Resource::new(2, flags, GRAPHICS_FLAGS, payload)
            .and_then(|resource| resource.to_bytes())
            .expect("a synthetic ytyp stays under the resource limits")
    }

    /// The data blocks in the order of their numbers: `CMapTypes`, the archetypes by kind, the
    /// unknown entries, the extensions by type, the pointers, then the strings.
    fn blocks(&self) -> Vec<Block> {
        let (mut archetype_blocks, places) = self.archetype_blocks();
        let has_unknown = self.entries.iter().any(Option::is_none);
        let extensions = || self.entries.iter().flatten().flat_map(|spec| &spec.extensions);
        let has_audio = extensions().any(|extension| matches!(extension, Extension::AudioCollision { .. }));
        let has_particle =
            extensions().any(|extension| matches!(extension, Extension::ParticleEffect { .. }));
        let mut next = 2 + archetype_blocks.len();
        let mut number = |used: bool| {
            used.then(|| {
                next += 1;
                block_id(next - 1)
            })
        };
        let unknown_id = number(has_unknown);
        let audio_id = number(has_audio);
        let particle_id = number(has_particle);
        let pointer_id = block_id(next);
        let string_id = block_id(next + 1);

        let mut unknown = Block::new(UNKNOWN_STRUCTURE);
        let mut audio = Block::new(AUDIO_COLLISION);
        let mut particle = Block::new(PARTICLE_EFFECT);
        let mut pointers = Block::new(POINTER_BLOCK);
        let mut strings = Block::new(STRING_BLOCK);

        // Each archetype's extension list, then the archetype list, in the pointer block.
        for (position, spec) in self.entries.iter().enumerate() {
            let (Some(spec), Some((index, offset))) = (spec, places[position]) else {
                continue;
            };
            if spec.extensions.is_empty() {
                continue;
            }
            let list = pointers.data.len();
            for extension in &spec.extensions {
                let target = match extension {
                    Extension::AudioCollision { .. } => (audio_id, &mut audio),
                    Extension::ParticleEffect { .. } => (particle_id, &mut particle),
                };
                let (Some(id), block) = target else { continue };
                pointers.put_u64(pointer(id, block.data.len()));
                block
                    .data
                    .extend(extension_bytes(extension, string_id, &mut strings));
            }
            let header = array(pointer(pointer_id, list), spec.extensions.len());
            archetype_blocks[index].data[offset + 120..offset + 136].copy_from_slice(&header);
        }
        let list = pointers.data.len();
        for (position, entry) in self.entries.iter().enumerate() {
            let raw = match (entry, places[position], unknown_id) {
                (Some(_), Some((index, offset)), _) => pointer(block_id(2 + index), offset),
                (None, _, Some(id)) => {
                    unknown.data.extend([0; UNKNOWN_LEN]);
                    pointer(id, unknown.data.len() - UNKNOWN_LEN)
                }
                _ => 0,
            };
            pointers.put_u64(raw);
        }

        let mut map_types = Block::new(MAP_TYPES);
        map_types.data.resize(80, 0);
        if !self.entries.is_empty() {
            map_types.data[24..40].copy_from_slice(&array(pointer(pointer_id, list), self.entries.len()));
        }
        map_types.data[40..44].copy_from_slice(&self.name.to_le_bytes());

        let mut blocks = vec![map_types];
        blocks.extend(archetype_blocks);
        blocks.extend(
            [
                (has_unknown, unknown),
                (has_audio, audio),
                (has_particle, particle),
            ]
            .into_iter()
            .filter_map(|(used, block)| used.then_some(block)),
        );
        blocks.push(pointers);
        if !strings.data.is_empty() {
            blocks.push(strings);
        }
        blocks
    }

    /// The archetype blocks, by kind in the order kinds first appear, and where each entry of the
    /// list lies among them: the block's index and the offset in it.
    fn archetype_blocks(&self) -> (Vec<Block>, Vec<Option<(usize, usize)>>) {
        let mut kinds: Vec<ArchetypeKind> = Vec::new();
        for spec in self.entries.iter().flatten() {
            if !kinds.contains(&spec.kind) {
                kinds.push(spec.kind);
            }
        }
        let mut blocks: Vec<Block> = Vec::new();
        let mut places = vec![None; self.entries.len()];
        for kind in kinds {
            let per_block = self.per_block.unwrap_or(MAX_BLOCK_LEN / kind.size());
            let mut in_block = per_block;
            for (position, spec) in self.entries.iter().enumerate() {
                let Some(spec) = spec.as_ref().filter(|spec| spec.kind == kind) else {
                    continue;
                };
                if in_block == per_block {
                    blocks.push(Block::new(kind.structure()));
                    in_block = 0;
                }
                let index = blocks.len() - 1;
                places[position] = Some((index, blocks[index].data.len()));
                blocks[index].data.extend(archetype_bytes(spec));
                in_block += 1;
            }
        }
        (blocks, places)
    }

    /// The structures the file describes: the archetype kinds and extension types it holds, then
    /// `CMapTypes`, as `CodeWalker` orders them.
    fn structure_infos(&self, blocks: &[Block]) -> Vec<StructureSpec> {
        let holds = |structure: u32| blocks.iter().any(|block| block.structure == structure);
        let mut infos = Vec::new();
        for kind in ArchetypeKind::ALL {
            if holds(kind.structure()) && !self.omitted.contains(&kind) {
                let mut info = archetype_info(kind);
                if let Some(&(_, size)) = self.declared.iter().rev().find(|(declared, _)| *declared == kind) {
                    info.size = size;
                }
                infos.push(info);
            }
        }
        if holds(AUDIO_COLLISION) {
            infos.push(StructureSpec::new(
                AUDIO_COLLISION,
                0,
                48,
                vec![
                    field(b"name", 8, HASH),
                    field(b"offsetPosition", 16, VECTOR3),
                    field(b"settings", 32, HASH),
                ],
            ));
        }
        if holds(PARTICLE_EFFECT) {
            infos.push(StructureSpec::new(
                PARTICLE_EFFECT,
                0,
                96,
                vec![
                    field(b"name", 8, HASH),
                    field(b"offsetPosition", 16, VECTOR3),
                    field(b"offsetRotation", 32, VECTOR4),
                    field(b"fxName", 48, CHAR_POINTER),
                    field(b"fxType", 64, SIGNED_INT),
                    field(b"boneTag", 68, SIGNED_INT),
                    field(b"scale", 72, FLOAT),
                    field(b"probability", 76, SIGNED_INT),
                    field(b"flags", 80, SIGNED_INT),
                    field(b"color", 84, INT),
                ],
            ));
        }
        let mut map_types = StructureSpec::new(
            MAP_TYPES,
            0x9B80_46D4,
            80,
            vec![
                element(STRUCTURE_POINTER, 0),
                array_field(b"extensions", 8, 0),
                element(STRUCTURE_POINTER, 0),
                array_field(b"archetypes", 24, 2),
                field(b"name", 40, HASH),
                element(HASH, 0),
                array_field(b"dependencies", 48, 5),
                element(STRUCTURE, joaat(b"CCompositeEntityType")),
                array_field(b"compositeEntityTypes", 64, 7),
            ],
        );
        map_types.alignment = 0x300;
        infos.push(map_types);
        infos
    }

    /// The system segment: header, page description, descriptions, block table and blocks.
    fn system(&self, blocks: &[Block]) -> Vec<u8> {
        let infos = self.structure_infos(blocks);
        let mut out = Writer(vec![0; HEADER_LEN]);

        let pages = out.align();
        out.0.extend([0; 24]);
        out.0[pages + 8] = 1;

        let infos_at = out.align();
        out.0.resize(infos_at + 32 * infos.len(), 0);
        for (index, info) in infos.iter().enumerate() {
            let entries_at = out.align();
            for entry in &info.entries {
                out.put_u32(entry.name);
                out.put_u32(entry.offset);
                out.0.extend([entry.data_type, 0]);
                out.0.extend(entry.reference_index.to_le_bytes());
                out.put_u32(entry.reference_key);
            }
            let record = infos_at + 32 * index;
            out.set_u32(record, info.name);
            out.set_u32(record + 4, info.key);
            out.set_u32(record + 8, info.alignment);
            out.set_u64(record + 0x10, address(entries_at));
            out.set_u32(
                record + 0x18,
                u32::try_from(info.size).expect("a synthetic structure stays small"),
            );
            out.set_u16(record + 0x1E, count(info.entries.len()));
        }

        let enums_at = out.align();
        out.0.resize(enums_at + 24, 0);
        let values_at = out.align();
        for (value, name) in (0u32..).zip(ASSET_TYPES) {
            out.put_u32(joaat(name));
            out.put_u32(value);
        }
        out.set_u32(enums_at, ASSET_TYPE);
        out.set_u32(enums_at + 4, 0x6F39_632C);
        out.set_u64(enums_at + 8, address(values_at));
        out.set_u32(enums_at + 0x10, count(ASSET_TYPES.len()).into());

        let table_at = out.align();
        out.0.resize(table_at + 16 * blocks.len(), 0);
        for (index, block) in blocks.iter().enumerate() {
            let at = out.align();
            out.0.extend(&block.data);
            let record = table_at + 16 * index;
            out.set_u32(record, block.structure);
            out.set_u32(
                record + 4,
                u32::try_from(block.data.len()).expect("a synthetic block stays small"),
            );
            out.set_u64(record + 8, address(at));
        }

        out.set_u32(0, 0x405B_C808);
        out.set_u32(4, 1);
        out.set_u64(8, address(pages));
        out.set_u32(0x10, 0x5052_4430);
        out.set_u16(0x14, 0x0079);
        out.set_u32(0x1C, 1);
        out.set_u64(0x20, address(infos_at));
        out.set_u64(0x28, address(enums_at));
        out.set_u64(0x30, address(table_at));
        out.set_u16(0x48, count(infos.len()));
        out.set_u16(0x4A, 1);
        out.set_u16(0x4C, count(blocks.len()));
        out.0
    }
}

/// A `.ytyp` named `stem` defining a prop for each of `names`, in that order.
pub fn ytyp(stem: &str, names: &[&str]) -> Vec<u8> {
    names
        .iter()
        .fold(YtypBuilder::new(stem), |builder, name| {
            builder.archetype(ArchetypeSpec::base(name))
        })
        .build()
}

struct Block {
    structure: u32,
    data: Vec<u8>,
}

impl Block {
    const fn new(structure: u32) -> Self {
        Self {
            structure,
            data: Vec::new(),
        }
    }

    fn put_u64(&mut self, value: u64) {
        self.data.extend(value.to_le_bytes());
    }
}

struct StructureSpec {
    name: u32,
    key: u32,
    /// The word after the key: the alignment's power of two in its second byte.
    alignment: u32,
    size: usize,
    entries: Vec<EntrySpec>,
}

impl StructureSpec {
    const fn new(name: u32, key: u32, size: usize, entries: Vec<EntrySpec>) -> Self {
        Self {
            name,
            key,
            alignment: 0x400,
            size,
            entries,
        }
    }
}

#[derive(Clone, Copy)]
struct EntrySpec {
    name: u32,
    offset: u32,
    data_type: u8,
    reference_index: u16,
    reference_key: u32,
}

const fn field(name: &[u8], offset: u32, data_type: u8) -> EntrySpec {
    EntrySpec {
        name: joaat(name),
        offset,
        data_type,
        reference_index: 0,
        reference_key: 0,
    }
}

/// An array field whose elements the entry at `element` describes.
const fn array_field(name: &[u8], offset: u32, element: u16) -> EntrySpec {
    EntrySpec {
        reference_index: element,
        ..field(name, offset, ARRAY)
    }
}

/// An element descriptor.
const fn element(data_type: u8, reference_key: u32) -> EntrySpec {
    EntrySpec {
        name: ARRAY_INFO,
        offset: 0,
        data_type,
        reference_index: 0,
        reference_key,
    }
}

fn archetype_info(kind: ArchetypeKind) -> StructureSpec {
    let mut entries = vec![
        field(b"lodDist", 8, FLOAT),
        field(b"flags", 12, INT),
        field(b"specialAttribute", 16, INT),
        field(b"bbMin", 32, VECTOR3),
        field(b"bbMax", 48, VECTOR3),
        field(b"bsCentre", 64, VECTOR3),
        field(b"bsRadius", 80, FLOAT),
        field(b"hdTextureDist", 84, FLOAT),
        field(b"name", 88, HASH),
        field(b"textureDictionary", 92, HASH),
        field(b"clipDictionary", 96, HASH),
        field(b"drawableDictionary", 100, HASH),
        field(b"physicsDictionary", 104, HASH),
        EntrySpec {
            reference_key: ASSET_TYPE,
            ..field(b"assetType", 108, INT_ENUM)
        },
        field(b"assetName", 112, HASH),
        element(STRUCTURE_POINTER, 0),
        array_field(b"extensions", 120, 15),
    ];
    let key = match kind {
        ArchetypeKind::Base => 0x8FBA_DAA4,
        ArchetypeKind::Time => {
            entries.push(field(b"timeFlags", 144, INT));
            0x963D_9B86
        }
        ArchetypeKind::Mlo => {
            entries.extend([
                field(b"mloFlags", 144, INT),
                element(STRUCTURE_POINTER, 0),
                array_field(b"entities", 152, 18),
                element(STRUCTURE, joaat(b"CMloRoomDef")),
                array_field(b"rooms", 168, 20),
                element(STRUCTURE, joaat(b"CMloPortalDef")),
                array_field(b"portals", 184, 22),
                element(STRUCTURE, joaat(b"CMloEntitySet")),
                array_field(b"entitySets", 200, 24),
                element(STRUCTURE, joaat(b"CMloTimeCycleModifier")),
                array_field(b"timeCycleModifiers", 216, 26),
            ]);
            0x37E3_A0F2
        }
    };
    StructureSpec::new(kind.structure(), key, kind.size(), entries)
}

/// An archetype's structure, its extension list left empty for [`YtypBuilder::blocks`] to fill.
fn archetype_bytes(spec: &ArchetypeSpec) -> Vec<u8> {
    let mut bytes = Writer(vec![0; spec.kind.size()]);
    bytes.set_f32(8, spec.lod_dist);
    bytes.set_u32(12, spec.flags);
    bytes.set_u32(16, spec.special_attribute);
    for (at, vector) in [(32, spec.bb_min), (48, spec.bb_max), (64, spec.bs_centre)] {
        for (lane, value) in vector.into_iter().enumerate() {
            bytes.set_f32(at + 4 * lane, value);
        }
    }
    bytes.set_f32(80, spec.bs_radius);
    bytes.set_f32(84, spec.hd_texture_dist);
    bytes.set_u32(88, spec.name);
    bytes.set_u32(92, spec.texture_dictionary);
    bytes.set_u32(96, spec.clip_dictionary);
    bytes.set_u32(100, spec.drawable_dictionary);
    bytes.set_u32(104, spec.physics_dictionary);
    bytes.set_u32(108, spec.asset_type);
    bytes.set_u32(112, spec.asset_name);
    match spec.kind {
        ArchetypeKind::Base => {}
        ArchetypeKind::Time => bytes.set_u32(144, spec.time_flags),
        ArchetypeKind::Mlo => bytes.set_u32(144, spec.mlo_flags),
    }
    bytes.0
}

/// An extension's structure; a particle effect's name goes into `strings`, block `string_id`.
fn extension_bytes(extension: &Extension, string_id: u16, strings: &mut Block) -> Vec<u8> {
    let (name, offset) = match extension {
        Extension::AudioCollision { name, offset, .. } | Extension::ParticleEffect { name, offset, .. } => {
            (*name, *offset)
        }
    };
    let mut bytes = Writer(Vec::new());
    match extension {
        Extension::AudioCollision { settings, .. } => {
            bytes.0.resize(48, 0);
            bytes.set_u32(32, *settings);
        }
        Extension::ParticleEffect {
            fx_name,
            fx_type,
            scale,
            ..
        } => {
            bytes.0.resize(96, 0);
            bytes.set_f32(44, 1.0);
            let text = pointer(string_id, strings.data.len());
            strings.data.extend(fx_name.as_bytes());
            strings.data.push(0);
            let len = count(fx_name.len());
            let mut header = array(text, usize::from(len));
            header[10..12].copy_from_slice(&(len + 1).to_le_bytes());
            bytes.0[48..64].copy_from_slice(&header);
            bytes.set_u32(64, fx_type.cast_unsigned());
            bytes.set_f32(72, *scale);
            bytes.set_u32(76, 100);
            bytes.set_u32(84, u32::MAX);
        }
    }
    bytes.set_u32(8, name);
    for (lane, value) in offset.into_iter().enumerate() {
        bytes.set_f32(16 + 4 * lane, value);
    }
    bytes.0
}

/// An array header: pointer, count, capacity equal to the count.
fn array(raw: u64, len: usize) -> [u8; 16] {
    let mut header = [0; 16];
    header[..8].copy_from_slice(&raw.to_le_bytes());
    header[8..10].copy_from_slice(&count(len).to_le_bytes());
    header[10..12].copy_from_slice(&count(len).to_le_bytes());
    header
}

fn pointer(block: u16, offset: usize) -> u64 {
    let offset = u32::try_from(offset).ok().filter(|&offset| offset <= 0xF_FFFF);
    Pointer {
        block,
        offset: offset.expect("a synthetic block stays under a mebibyte"),
    }
    .pack()
}

fn block_id(number: usize) -> u16 {
    u16::try_from(number)
        .ok()
        .filter(|&id| id <= 0xFFF)
        .expect("a synthetic file has few blocks")
}

fn count(len: usize) -> u16 {
    u16::try_from(len).expect("a synthetic list stays under 65536 entries")
}

/// A system address: the offset in the segment, in the `0x5xxxxxxx` range.
fn address(offset: usize) -> u64 {
    0x5000_0000 | u64::try_from(offset).expect("a synthetic segment stays small")
}

/// The flags of the smallest system segment of `len` bytes or more: up to 127 pages of 8 KiB, or of
/// a power of two times that.
fn system_flags(len: usize) -> u32 {
    let len = u64::try_from(len).expect("a synthetic segment stays small");
    (0..16)
        .find_map(|shift| {
            let pages = len.div_ceil(0x2000 << shift).max(1);
            (pages <= 0x7F).then(|| (u32::try_from(pages).expect("checked") << 17) | shift)
        })
        .expect("a synthetic segment stays under 256 MiB")
}

/// Little-endian writes into a growing buffer.
struct Writer(Vec<u8>);

impl Writer {
    /// Pads to a multiple of 16 and returns the offset reached.
    fn align(&mut self) -> usize {
        self.0.resize(self.0.len().next_multiple_of(16), 0);
        self.0.len()
    }

    fn put_u32(&mut self, value: u32) {
        self.0.extend(value.to_le_bytes());
    }

    fn set(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn set_u16(&mut self, at: usize, value: u16) {
        self.set(at, &value.to_le_bytes());
    }

    fn set_u32(&mut self, at: usize, value: u32) {
        self.set(at, &value.to_le_bytes());
    }

    fn set_u64(&mut self, at: usize, value: u64) {
        self.set(at, &value.to_le_bytes());
    }

    fn set_f32(&mut self, at: usize, value: f32) {
        self.set(at, &value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{Meta, u32_at, u64_at};
    use crate::ytyp;

    #[test]
    fn a_small_file_takes_one_8_kib_page_like_real_ones() {
        let bytes = ytyp("bz_prop_gift2", &["bz_prop_gift", "bz_prop_gift2"]);
        assert_eq!(bytes[..8], *b"RSC7\x02\0\0\0");
        let resource = Resource::read(&bytes).unwrap();
        assert_eq!(
            (resource.system_flags(), resource.graphics_flags()),
            (0x0002_0000, 0x2000_0000)
        );
        assert_eq!(resource.system().len(), 8192);
    }

    #[test]
    fn the_layout_matches_what_real_files_hold() {
        let bytes = YtypBuilder::new("camp")
            .archetype(ArchetypeSpec::base("a"))
            .archetype(ArchetypeSpec::time("b", 1))
            .archetype(ArchetypeSpec::mlo("c"))
            .build();
        let resource = Resource::read(&bytes).unwrap();
        let system = resource.system();
        assert_eq!(u32_at(system, 0x10), Some(0x5052_4430));
        assert_eq!(
            u32_at(system, 0x1C),
            Some(1),
            "the root block is CMapTypes, block 1"
        );
        let meta = Meta::parse(system).unwrap();
        let types: Vec<u32> = meta.blocks().iter().map(|block| block.structure).collect();
        assert_eq!(
            types,
            [
                MAP_TYPES,
                ArchetypeKind::Base.structure(),
                ArchetypeKind::Time.structure(),
                ArchetypeKind::Mlo.structure(),
                POINTER_BLOCK,
            ]
        );
        assert!(meta.blocks().iter().all(|block| block.start % 16 == 0));
        // The pointer block holds exactly the list, and capacity equals count, as in real files.
        let map_types = meta.block_bytes(1).unwrap();
        assert_eq!(
            u64_at(map_types, 24).map(Pointer::unpack),
            Some(Some(Pointer { block: 5, offset: 0 }))
        );
        assert_eq!(map_types[32..36], [3, 0, 3, 0]);
        assert_eq!(meta.block(5).unwrap().len, 24);
        for kind in ArchetypeKind::ALL {
            assert_eq!(meta.structure(kind.structure()).unwrap().size, kind.size());
        }
        let mlo = meta.structure(ArchetypeKind::Mlo.structure()).unwrap();
        let rooms = mlo.field(joaat(b"rooms")).unwrap();
        assert_eq!((rooms.offset, rooms.reference_index), (168, 20));
    }

    #[test]
    fn many_archetypes_spill_into_further_blocks_as_real_files_do() {
        let names: Vec<String> = (0..203).map(|index| format!("weed_{index}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let bytes = ytyp("weed", &refs);
        let resource = Resource::read(&bytes).unwrap();
        let meta = Meta::parse(resource.system()).unwrap();
        let lens: Vec<usize> = meta
            .blocks()
            .iter()
            .filter(|block| block.structure == ArchetypeKind::Base.structure())
            .map(|block| block.len / 144)
            .collect();
        assert_eq!(lens, [113, 90]);
        assert_eq!(ytyp::read(&bytes).unwrap().archetypes.len(), 203);
    }

    #[test]
    fn extensions_get_their_blocks_and_a_list_in_the_pointer_block() {
        let spec = ArchetypeSpec {
            extensions: vec![
                Extension::AudioCollision {
                    name: 1,
                    offset: [0.0; 3],
                    settings: 2,
                },
                Extension::ParticleEffect {
                    name: 3,
                    offset: [1.0; 3],
                    fx_name: "ent_amb_smoke".to_owned(),
                    fx_type: 0,
                    scale: 1.0,
                },
            ],
            ..ArchetypeSpec::base("smoker")
        };
        let bytes = YtypBuilder::new("smoke").archetype(spec).build();
        let map_types = ytyp::read(&bytes).unwrap();
        assert_eq!(map_types.archetypes[0].extension_count, 2);
        assert!(map_types.archetypes[0].digest.is_some());

        let resource = Resource::read(&bytes).unwrap();
        let meta = Meta::parse(resource.system()).unwrap();
        let strings = meta
            .block_bytes(meta.first_block_of(STRING_BLOCK).unwrap())
            .unwrap();
        assert_eq!(strings, b"ent_amb_smoke\0");
    }
}
