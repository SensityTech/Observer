//! `cargo xtask icons`: the mark of the design (a rounded tile, a ring, a dot) drawn at every size
//! Windows asks an executable for, without a rasteriser.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, bail};
use flate2::Compression;
use flate2::write::ZlibEncoder;

/// The sizes Windows picks from for the shell, the taskbar and the notification area, at every
/// scaling up to 400 %.
const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];
/// The status window's own icon, which winit takes as raw pixels.
const WINDOW_SIZE: u32 = 64;
/// Samples per pixel along each axis.
const SUPERSAMPLING: u32 = 4;

/// The light theme's ink, page and accent.
const TILE: [u8; 3] = [0x1C, 0x1D, 0x1F];
const RING: [u8; 3] = [0xFF, 0xFF, 0xFF];
const DOT: [u8; 3] = [0x1F, 0x5B, 0xEA];

const ICO: &str = "crates/observer/assets/observer.ico";
const RGBA: &str = "crates/observer/assets/mark-64.rgba";

pub fn run(root: &Path, check: bool) -> anyhow::Result<()> {
    let files = [(ICO, ico(&ICO_SIZES)?), (RGBA, mark(WINDOW_SIZE))];
    let mut stale = Vec::new();
    for (path, drawn) in files {
        let path = root.join(path);
        if check {
            if fs::read(&path).ok().as_deref() != Some(drawn.as_slice()) {
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
/// of 24 units: a tile with corners of 6, a ring of radius 6.25 and stroke 2, a dot of 2.25.
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
    let from_centre = (x - 12.0).hypot(y - 12.0);
    if from_centre <= 2.25 {
        Some(DOT)
    } else if (from_centre - 6.25).abs() <= 1.0 {
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
    fn the_mark_has_its_dot_in_the_middle_its_ring_around_and_clear_corners() {
        let image = mark(64);

        assert_eq!(pixel(&image, 64, 32, 32), [0x1F, 0x5B, 0xEA, 255]);
        // The ring's middle line, 6.25 units out: 16.7 pixels at 64.
        assert_eq!(pixel(&image, 64, 32, 15), [255, 255, 255, 255]);
        assert_eq!(pixel(&image, 64, 32, 4), [0x1C, 0x1D, 0x1F, 255]);
        assert_eq!(pixel(&image, 64, 0, 0)[3], 0);
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
