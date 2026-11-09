//! The kind and size of a map image, read from its first bytes as each format lays them out, so
//! that Observer keeps only images it knows the size of.

/// The size of a PNG, JPEG or WebP image, read from its first bytes; `None` for anything else.
pub(super) fn image_size(head: &[u8]) -> Option<(ImageKind, u32, u32)> {
    png(head)
        .map(|(width, height)| (ImageKind::Png, width, height))
        .or_else(|| jpeg(head).map(|(width, height)| (ImageKind::Jpeg, width, height)))
        .or_else(|| webp(head).map(|(width, height)| (ImageKind::Webp, width, height)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Webp,
}

impl ImageKind {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    pub(super) fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    pub(super) fn from_file(file: &str) -> Option<Self> {
        match file.rsplit_once('.')?.1 {
            "png" => Some(Self::Png),
            "jpg" => Some(Self::Jpeg),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(
        bytes.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        bytes.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
}

/// The signature, then the IHDR chunk with the width and height.
fn png(head: &[u8]) -> Option<(u32, u32)> {
    if head.get(..8)? != b"\x89PNG\r\n\x1a\n" || head.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be32(head, 16)?, be32(head, 20)?))
}

/// The markers up to the first start of frame, which holds the height then the width.
fn jpeg(head: &[u8]) -> Option<(u32, u32)> {
    if head.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2;
    loop {
        if *head.get(at)? != 0xFF {
            return None;
        }
        let marker = *head.get(at + 1)?;
        if marker == 0xFF {
            at += 1;
            continue;
        }
        // Start of frame: baseline, progressive and the rest, but DHT, JPG and DAC.
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            return Some((be16(head, at + 7)?, be16(head, at + 5)?));
        }
        if marker == 0xD9 || marker == 0xDA {
            return None;
        }
        at += 2 + usize::try_from(be16(head, at + 2)?).ok()?;
    }
}

/// RIFF, then one of WebP's three chunks: lossy, lossless or extended.
fn webp(head: &[u8]) -> Option<(u32, u32)> {
    if head.get(..4)? != b"RIFF" || head.get(8..12)? != b"WEBP" {
        return None;
    }
    match head.get(12..16)? {
        b"VP8 " if head.get(23..26)? == [0x9D, 0x01, 0x2A] => {
            Some((le16(head, 26)? & 0x3FFF, le16(head, 28)? & 0x3FFF))
        }
        b"VP8L" if *head.get(20)? == 0x2F => {
            let bits = u32::from_le_bytes(head.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => Some((le24(head, 24)? + 1, le24(head, 27)? + 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_gives_its_size_from_its_header() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&8192u32.to_be_bytes());
        png.extend_from_slice(&4096u32.to_be_bytes());

        assert_eq!(image_size(&png), Some((ImageKind::Png, 8192, 4096)));
        assert_eq!(image_size(&png[..20]), None);
    }

    #[test]
    fn a_jpeg_gives_its_size_from_its_first_frame_past_other_markers() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xC2, 0x00, 0x11, 0x08]);
        jpeg.extend_from_slice(&2048u16.to_be_bytes());
        jpeg.extend_from_slice(&3000u16.to_be_bytes());

        assert_eq!(image_size(&jpeg), Some((ImageKind::Jpeg, 3000, 2048)));
    }

    #[test]
    fn each_webp_chunk_gives_its_size() {
        let riff = |chunk: &[u8], body: &[u8]| {
            let mut file = b"RIFF\0\0\0\0WEBP".to_vec();
            file.extend_from_slice(chunk);
            file.extend_from_slice(&[0; 4]);
            file.extend_from_slice(body);
            file
        };
        let mut lossy = vec![0; 3];
        lossy.extend_from_slice(&[0x9D, 0x01, 0x2A]);
        lossy.extend_from_slice(&1000u16.to_le_bytes());
        lossy.extend_from_slice(&500u16.to_le_bytes());
        let bits: u32 = (1000 - 1) | ((500 - 1) << 14);
        let mut lossless = vec![0x2F];
        lossless.extend_from_slice(&bits.to_le_bytes());
        let mut extended = vec![0; 4];
        extended.extend_from_slice(&(16_383u32).to_le_bytes()[..3]);
        extended.extend_from_slice(&(8191u32).to_le_bytes()[..3]);

        assert_eq!(
            image_size(&riff(b"VP8 ", &lossy)),
            Some((ImageKind::Webp, 1000, 500))
        );
        assert_eq!(
            image_size(&riff(b"VP8L", &lossless)),
            Some((ImageKind::Webp, 1000, 500))
        );
        assert_eq!(
            image_size(&riff(b"VP8X", &extended)),
            Some((ImageKind::Webp, 16_384, 8192))
        );
        assert_eq!(image_size(b"GIF89a"), None);
    }

    #[test]
    fn a_kept_file_names_its_kind() {
        for kind in [ImageKind::Png, ImageKind::Jpeg, ImageKind::Webp] {
            assert_eq!(
                ImageKind::from_file(&format!("map-1a2b.{}", kind.extension())),
                Some(kind)
            );
        }
        assert_eq!(ImageKind::from_file("map-1a2b.partial"), None);
    }
}
