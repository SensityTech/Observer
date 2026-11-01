//! GTA V game files as FiveM servers stream them, read and patched in Rust with no game install: the
//! RSC7 resource envelope, the binary metadata (RBM) inside it, the archetypes of a `.ytyp`, and the
//! model-info `.meta` files written as XML. Loose resource files carry no encryption, so no key is
//! needed; a file that is not one of these formats is refused with what it looks like instead.

pub mod hash;
pub mod meta;
pub mod model_info;
pub mod rsc7;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod xml;
pub mod ytyp;

use rsc7::FileKind;

/// Why a resource, its metadata or its archetypes cannot be read, or a `.ytyp` cannot be patched.
/// The bytes come from servers' resources, so each of these is an answer, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("not an RSC7 resource: the file looks like {0}")]
    NotAResource(FileKind),
    #[error("the RSC7 header is cut short")]
    ShortHeader,
    #[error("the resource declares {0} bytes, more than the {limit} bytes read", limit = rsc7::MAX_PAYLOAD)]
    TooLarge(u64),
    #[error("the payload cannot be inflated: {0}")]
    Inflate(String),
    #[error("the payload inflates to {actual} bytes where the header gives {expected}")]
    PayloadShort { expected: usize, actual: usize },
    #[error("the payload inflates to more than the {expected} bytes the header gives")]
    PayloadLong { expected: usize },
    #[error("the payload holds {actual} bytes where the header gives {expected}")]
    PayloadLength { expected: usize, actual: usize },
    #[error("the payload cannot be compressed: {0}")]
    Deflate(String),
    #[error("resource version {0}: binary metadata is version 2")]
    Version(u32),
    #[error("the resource holds no binary metadata")]
    NotMeta,
    #[error("malformed metadata: {0}")]
    Malformed(&'static str),
    #[error("the metadata holds no CMapTypes")]
    NoMapTypes,
    #[error("archetype index {index} is out of range: the file lists {count}")]
    IndexOutOfRange { index: usize, count: usize },
    #[error("archetype index {0} is given twice")]
    DuplicateIndex(usize),
    #[error("entry {0} of the archetype list is not an archetype")]
    NotAnArchetype(usize),
    #[error("the file cannot be patched: {0}")]
    NotPatchable(&'static str),
    #[error("the patched file does not read back as expected: {0}")]
    Verification(&'static str),
}
