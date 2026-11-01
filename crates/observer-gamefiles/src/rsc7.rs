//! The RSC7 envelope of a GTA V resource: a 16-byte header (magic, version, system flags, graphics
//! flags), then the raw DEFLATE of the system segment followed by the graphics segment. The flags
//! give each segment's size, so the payload is inflated to exactly that much and never further: a
//! file cannot make the reader allocate more than its header declares, and the declaration itself is
//! capped.
//!
//! Only raw DEFLATE is read, as the game reads it: its streamer inflates resources with no header,
//! like the pages of its archives, and so do FiveM's own readers (`inflateInit2` with -15 window
//! bits). A payload behind a zlib header fails there, FiveM turning the failure into a fatal error
//! naming the streamed file, so such a file never registers anything in game. It is refused here
//! too: read as if loaded, it could be picked as the copy to keep, and rewritten as raw DEFLATE, it
//! would start loading.
//!
//! Writing keeps the 16 bytes as they were and deflates the payload again. Only a payload of the
//! same length is written, so the flags stay true without being recomputed.

use std::cmp::Ordering;
use std::fmt;
use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

use crate::Error;

/// The first four bytes of a resource.
pub const MAGIC: [u8; 4] = *b"RSC7";
/// Magic, version, system flags and graphics flags, four little-endian `u32`.
pub const HEADER_LEN: usize = 16;
/// The most a resource may inflate to. Streamed `.ytyp` files hold kilobytes, and FXServer already
/// warns about any streamed file past 16 MiB.
pub const MAX_PAYLOAD: u64 = 256 * 1024 * 1024;
/// DEFLATE expands at most about 1032 times: no larger first allocation is ever needed.
const MAX_RATIO: usize = 1032;

/// What a file looks like from its first bytes, said when it is not a resource this crate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// An RSC7 resource.
    Rsc7,
    /// Encrypted by FiveM's asset escrow (`FXAP`): only the server it was bought for can read it.
    Escrowed,
    /// PSO binary metadata (`PSIN`), which the game reads but no streamed `.ytyp` uses.
    Pso,
    /// An RBF file (`RBF0`).
    Rbf,
    /// XML text, possibly after a byte order mark.
    Xml,
    /// No bytes at all.
    Empty,
    /// None of the above.
    Unknown,
}

impl FileKind {
    /// The kind in words, as a sentence names it.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Rsc7 => "an RSC7 resource",
            Self::Escrowed => "an escrow-encrypted file (FXAP)",
            Self::Pso => "a PSO file (PSIN)",
            Self::Rbf => "an RBF file (RBF0)",
            Self::Xml => "XML text",
            Self::Empty => "an empty file",
            Self::Unknown => "an unknown format",
        }
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.describe())
    }
}

/// Names what `bytes` look like from their first bytes alone.
pub fn sniff(bytes: &[u8]) -> FileKind {
    match bytes {
        [] => FileKind::Empty,
        [b'R', b'S', b'C', b'7', ..] => FileKind::Rsc7,
        [b'F', b'X', b'A', b'P', ..] => FileKind::Escrowed,
        [b'P', b'S', b'I', b'N', ..] => FileKind::Pso,
        [b'R', b'B', b'F', b'0', ..] => FileKind::Rbf,
        _ if looks_like_xml(bytes) => FileKind::Xml,
        _ => FileKind::Unknown,
    }
}

/// A `<` after an optional UTF-8 byte order mark and blanks.
fn looks_like_xml(bytes: &[u8]) -> bool {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    text.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'<')
}

/// The size of a segment as its flags give it: counts of pages of nine sizes, from `base << 8` down
/// to `base`, where `base` is 512 bytes shifted by the low four bits. The top four bits carry half of
/// the version and no size.
pub fn segment_size(flags: u32) -> u64 {
    let bits = |shift: u32, mask: u32| u64::from((flags >> shift) & mask);
    let pages = bits(27, 1)
        + bits(26, 1) * 2
        + bits(25, 1) * 4
        + bits(24, 1) * 8
        + bits(17, 0x7F) * 16
        + bits(11, 0x3F) * 32
        + bits(7, 0xF) * 64
        + bits(5, 3) * 128
        + bits(4, 1) * 256;
    (0x200 << bits(0, 0xF)) * pages
}

/// The system segment's length and the payload's, checked against [`MAX_PAYLOAD`].
fn sizes(system_flags: u32, graphics_flags: u32) -> Result<(usize, usize), Error> {
    // Each segment is below 2^37 bytes, so the sum cannot overflow.
    let total = segment_size(system_flags) + segment_size(graphics_flags);
    if total > MAX_PAYLOAD {
        return Err(Error::TooLarge(total));
    }
    let system = usize::try_from(segment_size(system_flags)).map_err(|_| Error::TooLarge(total))?;
    let total = usize::try_from(total).map_err(|_| Error::TooLarge(total))?;
    Ok((system, total))
}

/// A resource opened: its header and its payload inflated, the system segment then the graphics one.
#[derive(Clone, PartialEq, Eq)]
pub struct Resource {
    version: u32,
    system_flags: u32,
    graphics_flags: u32,
    system_len: usize,
    payload: Vec<u8>,
}

impl Resource {
    /// Opens `bytes`. A file that does not start with `RSC7` is refused with what it looks like. The
    /// payload must inflate as raw DEFLATE to exactly the size the flags give; bytes after the end of
    /// the stream are left unread (some tools leave one).
    pub fn read(bytes: &[u8]) -> Result<Self, Error> {
        let kind = sniff(bytes);
        if kind != FileKind::Rsc7 {
            return Err(Error::NotAResource(kind));
        }
        let word = |at: usize| {
            bytes
                .get(at..at + 4)
                .and_then(|word| word.try_into().ok())
                .map(u32::from_le_bytes)
                .ok_or(Error::ShortHeader)
        };
        let (version, system_flags, graphics_flags) = (word(4)?, word(8)?, word(12)?);
        let (system_len, total) = sizes(system_flags, graphics_flags)?;
        let payload = inflate(bytes.get(HEADER_LEN..).unwrap_or_default(), total)?;
        Ok(Self {
            version,
            system_flags,
            graphics_flags,
            system_len,
            payload,
        })
    }

    /// A resource made of these parts, as a builder of new files writes one. The payload must have
    /// the length the flags give.
    pub fn new(
        version: u32,
        system_flags: u32,
        graphics_flags: u32,
        payload: Vec<u8>,
    ) -> Result<Self, Error> {
        let (system_len, total) = sizes(system_flags, graphics_flags)?;
        if payload.len() != total {
            return Err(Error::PayloadLength {
                expected: total,
                actual: payload.len(),
            });
        }
        Ok(Self {
            version,
            system_flags,
            graphics_flags,
            system_len,
            payload,
        })
    }

    /// The version in the header: 2 for binary metadata such as `.ytyp`.
    pub const fn version(&self) -> u32 {
        self.version
    }

    pub const fn system_flags(&self) -> u32 {
        self.system_flags
    }

    pub const fn graphics_flags(&self) -> u32 {
        self.graphics_flags
    }

    /// The system segment, where binary metadata lives, padding to its full size included.
    pub fn system(&self) -> &[u8] {
        self.payload.get(..self.system_len).unwrap_or_default()
    }

    /// The system segment to change in place: its length cannot change, so neither can the flags.
    pub fn system_mut(&mut self) -> &mut [u8] {
        let len = self.system_len.min(self.payload.len());
        &mut self.payload[..len]
    }

    /// The graphics segment, empty in binary metadata.
    pub fn graphics(&self) -> &[u8] {
        self.payload.get(self.system_len..).unwrap_or_default()
    }

    /// The file again: the same 16-byte header, then the payload deflated at the best compression. A
    /// payload whose length no longer matches the flags is refused.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let (_, total) = sizes(self.system_flags, self.graphics_flags)?;
        if self.payload.len() != total {
            return Err(Error::PayloadLength {
                expected: total,
                actual: self.payload.len(),
            });
        }
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len() / 8);
        out.extend_from_slice(&MAGIC);
        for word in [self.version, self.system_flags, self.graphics_flags] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        let mut encoder = DeflateEncoder::new(out, Compression::best());
        encoder
            .write_all(&self.payload)
            .map_err(|error| Error::Deflate(error.to_string()))?;
        encoder
            .finish()
            .map_err(|error| Error::Deflate(error.to_string()))
    }
}

impl fmt::Debug for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resource")
            .field("version", &self.version)
            .field("system_flags", &format_args!("{:#010X}", self.system_flags))
            .field("graphics_flags", &format_args!("{:#010X}", self.graphics_flags))
            .field("system_len", &self.system_len)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

/// Inflates `compressed`, raw DEFLATE as the game reads it, to exactly `expected` bytes.
fn inflate(compressed: &[u8], expected: usize) -> Result<Vec<u8>, Error> {
    // One byte past the expected size tells a stream that goes on.
    let limit = u64::try_from(expected).unwrap_or(u64::MAX).saturating_add(1);
    let mut payload = Vec::with_capacity(expected.min(compressed.len().saturating_mul(MAX_RATIO)));
    DeflateDecoder::new(compressed)
        .take(limit)
        .read_to_end(&mut payload)
        .map_err(|error| Error::Inflate(error.to_string()))?;
    match payload.len().cmp(&expected) {
        Ordering::Less => Err(Error::PayloadShort {
            expected,
            actual: payload.len(),
        }),
        Ordering::Greater => Err(Error::PayloadLong { expected }),
        Ordering::Equal => Ok(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deflate(payload: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    }

    fn file(system_flags: u32, graphics_flags: u32, compressed: &[u8]) -> Vec<u8> {
        let mut bytes = b"RSC7".to_vec();
        for word in [2, system_flags, graphics_flags] {
            bytes.extend_from_slice(&u32::to_le_bytes(word));
        }
        bytes.extend_from_slice(compressed);
        bytes
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|index| u8::try_from(index % 251).unwrap()).collect()
    }

    #[test]
    fn flags_seen_in_real_files_give_their_segment_sizes() {
        assert_eq!(segment_size(0x0002_0000), 8192);
        assert_eq!(segment_size(0x0004_0000), 16384);
        assert_eq!(segment_size(0x0800_0004), 8192);
        assert_eq!(segment_size(0x0400_0004), 16384);
        assert_eq!(segment_size(0x0000_0800), 16384);
        assert_eq!(segment_size(0x0C00_0005), 0x4000 * 3);
        // Graphics flags of binary metadata: the version nibble and no pages.
        assert_eq!(segment_size(0x2000_0000), 0);
    }

    #[test]
    fn every_page_count_field_adds_its_share() {
        let base = 0x200;
        assert_eq!(segment_size(1 << 4), base * 256);
        assert_eq!(segment_size(3 << 5), base * 3 * 128);
        assert_eq!(segment_size(0xF << 7), base * 15 * 64);
        assert_eq!(segment_size(0x3F << 11), base * 63 * 32);
        assert_eq!(segment_size(0x7F << 17), base * 127 * 16);
        assert_eq!(segment_size(0xF << 24), base * 15);
        assert_eq!(segment_size(0xFFFF_FFFF), (0x200 << 15) * 5663);
    }

    #[test]
    fn a_resource_reads_and_writes_back_the_same_header_and_payload() {
        let payload = pattern(8192);
        let bytes = file(0x0002_0000, 0x2000_0000, &deflate(&payload));

        let resource = Resource::read(&bytes).unwrap();
        assert_eq!(resource.version(), 2);
        assert_eq!(resource.system(), payload.as_slice());
        assert_eq!(resource.graphics(), []);

        let written = resource.to_bytes().unwrap();
        assert_eq!(written[..HEADER_LEN], bytes[..HEADER_LEN]);
        assert_eq!(Resource::read(&written).unwrap(), resource);
    }

    #[test]
    fn a_change_in_the_system_segment_survives_writing() {
        let bytes = file(0x0002_0000, 0x2000_0000, &deflate(&[0; 8192]));
        let mut resource = Resource::read(&bytes).unwrap();
        resource.system_mut()[100] = 7;

        let reread = Resource::read(&resource.to_bytes().unwrap()).unwrap();
        assert_eq!(reread.system()[100], 7);
    }

    #[test]
    fn bytes_after_the_deflate_stream_are_left_unread() {
        let mut compressed = deflate(&pattern(8192));
        compressed.push(0);
        let resource = Resource::read(&file(0x0002_0000, 0x2000_0000, &compressed)).unwrap();
        assert_eq!(resource.system(), pattern(8192).as_slice());
    }

    #[test]
    fn a_payload_behind_a_zlib_header_is_refused_as_the_game_refuses_it() {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&pattern(8192)).unwrap();
        let wrapped = encoder.finish().unwrap();
        assert_eq!(wrapped[0], 0x78);
        assert!(matches!(
            Resource::read(&file(0x0002_0000, 0x2000_0000, &wrapped)),
            Err(Error::Inflate(_))
        ));
        for header in [[0x78, 0x01], [0x78, 0x5E], [0x78, 0x9C], [0x78, 0xDA]] {
            let mut compressed = header.to_vec();
            compressed.extend(deflate(&pattern(8192)));
            compressed.extend([0, 0, 0, 0]);
            assert!(
                Resource::read(&file(0x0002_0000, 0x2000_0000, &compressed)).is_err(),
                "{header:02X?}"
            );
        }
    }

    #[test]
    fn a_payload_shorter_or_longer_than_its_flags_is_refused() {
        let short = file(0x0004_0000, 0x2000_0000, &deflate(&pattern(8192)));
        assert_eq!(
            Resource::read(&short),
            Err(Error::PayloadShort {
                expected: 16384,
                actual: 8192
            })
        );
        let long = file(0x0002_0000, 0x2000_0000, &deflate(&pattern(16384)));
        assert_eq!(Resource::read(&long), Err(Error::PayloadLong { expected: 8192 }));
    }

    #[test]
    fn a_truncated_or_corrupt_stream_is_an_error() {
        let compressed = deflate(&pattern(8192));
        for cut in [0, 1, compressed.len() / 2, compressed.len() - 1] {
            let bytes = file(0x0002_0000, 0x2000_0000, &compressed[..cut]);
            assert!(Resource::read(&bytes).is_err(), "cut at {cut}");
        }
        let garbage = file(0x0002_0000, 0x2000_0000, &[0xFF; 64]);
        assert!(Resource::read(&garbage).is_err());
        for cut in 0..HEADER_LEN {
            let bytes = file(0x0002_0000, 0x2000_0000, &compressed);
            assert!(Resource::read(&bytes[..cut]).is_err(), "header cut at {cut}");
        }
    }

    #[test]
    fn an_absurd_declared_size_is_refused_before_inflating() {
        let bytes = file(0xFFFF_FFFF, 0x2000_0000, &deflate(&[0; 16]));
        assert!(matches!(Resource::read(&bytes), Err(Error::TooLarge(_))));
        let just_over = file(0x00FE_000F, 0x2000_0000, &[]);
        assert!(segment_size(0x00FE_000F) > MAX_PAYLOAD);
        assert!(matches!(Resource::read(&just_over), Err(Error::TooLarge(_))));
    }

    #[test]
    fn a_payload_of_another_length_is_never_written() {
        assert_eq!(
            Resource::new(2, 0x0002_0000, 0x2000_0000, vec![0; 100]),
            Err(Error::PayloadLength {
                expected: 8192,
                actual: 100
            })
        );
    }

    #[test]
    fn files_that_are_not_resources_are_named_by_what_they_look_like() {
        assert_eq!(sniff(b""), FileKind::Empty);
        assert_eq!(sniff(b"RSC7\x02\0\0\0"), FileKind::Rsc7);
        assert_eq!(sniff(b"FXAP\x01\0\0\0\0\0\xdb\xb5"), FileKind::Escrowed);
        assert_eq!(sniff(b"PSIN\0\0\0\x10"), FileKind::Pso);
        assert_eq!(sniff(b"RBF0\0\0"), FileKind::Rbf);
        assert_eq!(sniff(b"\xEF\xBB\xBF\r\n  <?xml version=\"1.0\"?>"), FileKind::Xml);
        assert_eq!(sniff(b"<CMapTypes>"), FileKind::Xml);
        assert_eq!(sniff(b"\x00\x00\x00\x00"), FileKind::Unknown);
        assert_eq!(sniff(b"RSC"), FileKind::Unknown);
        assert_eq!(
            Resource::read(b"FXAP\x01\0\0\0\0\0"),
            Err(Error::NotAResource(FileKind::Escrowed))
        );
        assert_eq!(
            Error::NotAResource(FileKind::Escrowed).to_string(),
            "not an RSC7 resource: the file looks like an escrow-encrypted file (FXAP)"
        );
    }
}
