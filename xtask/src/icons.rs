//! `cargo xtask icons`: the mark of the design (a ring and the bead on it, on a rounded tile) drawn at every size
//! Windows asks an executable for, without a rasteriser.

use std::f64::consts::FRAC_1_SQRT_2;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, bail};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

/// The sizes Windows picks from for the shell, the taskbar and the notification area, at every
/// scaling up to 400 %.
const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];
/// The status window's own icon, which winit takes as raw pixels.
const WINDOW_SIZE: u32 = 64;
/// Samples per pixel along each axis.
const SUPERSAMPLING: u32 = 4;

/// The light theme's ink for the tile, then the dark theme's ink and accent on it.
const TILE: [u8; 3] = [0x1C, 0x1D, 0x1F];
const RING: [u8; 3] = [0xF5, 0xF5, 0xF5];
const BEAD: [u8; 3] = [0x4D, 0x8D, 0xFF];

/// The bead's centre: on the ring's middle line, top right.
const BEAD_AT: (f64, f64) = (12.0 + 7.5 * FRAC_1_SQRT_2, 12.0 - 7.5 * FRAC_1_SQRT_2);

const ICO: &str = "crates/observer/assets/observer.ico";
const RGBA: &str = "crates/observer/assets/mark-64.rgba";

pub fn run(root: &Path, check: bool) -> anyhow::Result<()> {
    let files = [(ICO, ico(&ICO_SIZES)?), (RGBA, mark(WINDOW_SIZE))];
    let mut stale = Vec::new();
    for (path, drawn) in files {
        let path = root.join(path);
        if check {
            let committed = fs::read(&path).unwrap_or_default();
            // Deflate's output depends on the zlib backend: an ICO is compared by what it shows.
            let same = if path.extension().is_some_and(|extension| extension == "ico") {
                shown(&committed).is_some_and(|images| Some(images) == shown(&drawn))
            } else {
                committed == drawn
            };
            if !same {
                stale.push(path.display().to_string());
            }
        } else {
            fs::write(&path, drawn).with_context(|| format!("cannot write {}", path.display()))?;
        }
    }
    if !stale.is_empty() {
        bail!(
            "the icons differ from what `cargo xtask icons` draws; run it and commit: {}",
            stale.join(", ")
        );
    }
    Ok(())
}

/// The mark at `size` pixels, as straight RGBA rows from the top. The design draws it on a grid
/// of 24 units: a tile with corners of 6, a ring of radius 7.5 and stroke 3, a bead of 3 on it at
/// 45°, and the ring cleared 1.5 around the bead.
pub fn mark(size: u32) -> Vec<u8> {
    let unit = 24.0 / f64::from(size);
    let samples = f64::from(SUPERSAMPLING * SUPERSAMPLING);
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // Premultiplied sums, so that edges blend towards transparency, not towards black.
            let mut sum = [0.0f64; 4];
            for sy in 0..SUPERSAMPLING {
                for sx in 0..SUPERSAMPLING {
                    let offset = |sub: u32| (f64::from(sub) + 0.5) / f64::from(SUPERSAMPLING);
                    let px = (f64::from(x) + offset(sx)) * unit;
                    let py = (f64::from(y) + offset(sy)) * unit;
                    if let Some(colour) = colour_at(px, py) {
                        for (channel, value) in sum.iter_mut().zip(colour) {
                            *channel += f64::from(value);
                        }
                        sum[3] += 255.0;
                    }
                }
            }
            let alpha = sum[3] / samples;
            for channel in &sum[..3] {
                let straight = if alpha > 0.0 {
                    channel / sum[3] * 255.0
                } else {
                    0.0
                };
                pixels.push(byte(straight));
            }
            pixels.push(byte(alpha));
        }
    }
    pixels
}

/// The colour of the topmost shape at a point of the 24-unit grid, if any.
fn colour_at(x: f64, y: f64) -> Option<[u8; 3]> {
    let from_bead = (x - BEAD_AT.0).hypot(y - BEAD_AT.1);
    if from_bead <= 3.0 {
        Some(BEAD)
    } else if from_bead >= 4.5 && ((x - 12.0).hypot(y - 12.0) - 7.5).abs() <= 1.5 {
        Some(RING)
    } else if rounded_square(x, y, 24.0, 6.0) <= 0.0 {
        Some(TILE)
    } else {
        None
    }
}

/// The signed distance to a square of side `side` at the origin, with corners of `radius`.
fn rounded_square(x: f64, y: f64, side: f64, radius: f64) -> f64 {
    let half = side / 2.0;
    let qx = (x - half).abs() - (half - radius);
    let qy = (y - half).abs() - (half - radius);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "rounded and clamped to a byte first"
)]
fn byte(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// An ICO of PNG entries, which Windows reads from Vista on.
fn ico(sizes: &[u32]) -> anyhow::Result<Vec<u8>> {
    let images = sizes
        .iter()
        .map(|&size| png(size, &mark(size)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let count = u16::try_from(sizes.len())?;
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * u32::from(count);
    for (&size, image) in sizes.iter().zip(&images) {
        // 256 is written as 0: the field is a byte.
        let side = u8::try_from(size).unwrap_or(0);
        ico.extend_from_slice(&[side, side, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        let length = u32::try_from(image.len())?;
        ico.extend_from_slice(&length.to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += length;
    }
    for image in images {
        ico.extend_from_slice(&image);
    }
    Ok(ico)
}

fn png(size: u32, rgba: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut rows = Vec::with_capacity(rgba.len() + size as usize);
    for row in rgba.chunks(size as usize * 4) {
        // Filter type 0: the row as it is.
        rows.push(0);
        rows.extend_from_slice(row);
    }
    let mut zlib = ZlibEncoder::new(Vec::new(), Compression::best());
    zlib.write_all(&rows)?;

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&size.to_be_bytes());
    header.extend_from_slice(&size.to_be_bytes());
    // 8 bits per channel, RGBA, deflate, no filter method beyond the standard one, no interlace.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, *b"IHDR", &header);
    chunk(&mut png, *b"IDAT", &zlib.finish()?);
    chunk(&mut png, *b"IEND", &[]);
    Ok(png)
}

/// What an ICO of ours shows: each entry's side, then its PNG header and its rows inflated.
fn shown(ico: &[u8]) -> Option<Vec<(u8, Vec<u8>)>> {
    let count = u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?);
    (0..usize::from(count))
        .map(|index| {
            let entry = ico.get(6 + 16 * index..6 + 16 * (index + 1))?;
            let length = usize::try_from(u32::from_le_bytes(entry[8..12].try_into().ok()?)).ok()?;
            let offset = usize::try_from(u32::from_le_bytes(entry[12..16].try_into().ok()?)).ok()?;
            let png = ico.get(offset..offset.checked_add(length)?)?;
            Some((entry[0], decoded(png)?))
        })
        .collect()
}

fn decoded(png: &[u8]) -> Option<Vec<u8>> {
    let mut header = Vec::new();
    let mut compressed = Vec::new();
    let mut at = 8;
    while at + 8 <= png.len() {
        let length = usize::try_from(u32::from_be_bytes(png[at..at + 4].try_into().ok()?)).ok()?;
        let data = png.get(at + 8..at + 8 + length)?;
        match &png[at + 4..at + 8] {
            b"IHDR" => header.extend_from_slice(data),
            b"IDAT" => compressed.extend_from_slice(data),
            _ => {}
        }
        at += 12 + length;
    }
    ZlibDecoder::new(compressed.as_slice())
        .read_to_end(&mut header)
        .ok()?;
    Some(header)
}

fn chunk(png: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
    png.extend_from_slice(&length.to_be_bytes());
    png.extend_from_slice(&kind);
    png.extend_from_slice(data);
    let mut crc = flate2::Crc::new();
    crc.update(&kind);
    crc.update(data);
    png.extend_from_slice(&crc.sum().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * size + x) * 4) as usize;
        image[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn the_mark_has_its_bead_top_right_on_its_ring_cleared_around_it_and_clear_corners() {
        let image = mark(64);

        // The bead's centre, at 17.3 by 6.7 units: 46.1 by 17.9 pixels at 64.
        assert_eq!(pixel(&image, 64, 46, 17), [0x4D, 0x8D, 0xFF, 255]);
        // The ring's middle line on the left, 4.5 units in: 12 pixels at 64.
        assert_eq!(pixel(&image, 64, 12, 32), [0xF5, 0xF5, 0xF5, 255]);
        // On the ring's middle line, between 3 and 4.5 units from the bead: cleared to the tile.
        assert_eq!(pixel(&image, 64, 51, 26), [0x1C, 0x1D, 0x1F, 255]);
        assert_eq!(pixel(&image, 64, 32, 32), [0x1C, 0x1D, 0x1F, 255]);
        assert_eq!(pixel(&image, 64, 0, 0)[3], 0);
    }

    /// What the check compares is the pixels, never the deflate stream, which another zlib
    /// backend writes differently.
    #[test]
    fn an_ico_is_compared_by_the_pixels_it_holds() {
        let expected: Vec<(u8, Vec<u8>)> = ICO_SIZES
            .iter()
            .map(|&size| {
                let mut image = size.to_be_bytes().to_vec();
                image.extend(size.to_be_bytes());
                image.extend([8, 6, 0, 0, 0]);
                for row in mark(size).chunks(size as usize * 4) {
                    image.push(0);
                    image.extend_from_slice(row);
                }
                (u8::try_from(size).unwrap_or(0), image)
            })
            .collect();

        assert_eq!(shown(&ico(&ICO_SIZES).unwrap()), Some(expected));
        assert_ne!(
            shown(&ico(&ICO_SIZES).unwrap()),
            shown(&ico(&ICO_SIZES[1..]).unwrap())
        );
        assert_eq!(shown(b"not an icon"), None);
    }

    #[test]
    fn the_ico_lists_every_size_and_points_at_png_entries() {
        let ico = ico(&ICO_SIZES).unwrap();

        assert_eq!(&ico[..6], &[0, 0, 1, 0, 8, 0]);
        let entry = |index: usize| &ico[6 + 16 * index..6 + 16 * (index + 1)];
        assert_eq!(entry(0)[0], 16);
        assert_eq!(entry(7)[0], 0, "256 is written as 0");
        let offset = u32::from_le_bytes(entry(7)[12..16].try_into().unwrap()) as usize;
        assert_eq!(&ico[offset..offset + 8], b"\x89PNG\r\n\x1a\n");
    }
}
