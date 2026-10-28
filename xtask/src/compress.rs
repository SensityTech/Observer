use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use flate2::Compression;
use flate2::write::GzEncoder;
use walkdir::WalkDir;

/// Smaller files gain nothing from compression and cost a lookup.
const MIN_BYTES: u64 = 1024;
const COMPRESSIBLE: &[&str] = &["css", "html", "js", "json", "map", "svg", "txt"];

/// Writes `.br` and `.gz` siblings for the panel's text files, which the server picks by
/// `Accept-Encoding`. Browsers only offer Brotli over HTTPS, so plain HTTP still needs gzip.
pub fn dist(dir: &Path) -> anyhow::Result<()> {
    for entry in WalkDir::new(dir) {
        let entry = entry?;
        let path = entry.path();
        let compressible = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| COMPRESSIBLE.contains(&extension));
        if !entry.file_type().is_file() || !compressible || entry.metadata()?.len() < MIN_BYTES {
            continue;
        }

        let data = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        write_sibling(path, "gz", &gzip(&data)?)?;
        write_sibling(path, "br", &brotli(&data)?)?;
    }
    Ok(())
}

fn gzip(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn brotli(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut output = Vec::new();
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: 22,
        ..Default::default()
    };
    brotli::BrotliCompress(&mut &data[..], &mut output, &params)?;
    Ok(output)
}

fn write_sibling(path: &Path, extension: &str, data: &[u8]) -> anyhow::Result<()> {
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(extension);
    fs::write(&name, data).with_context(|| format!("cannot write {}", Path::new(&name).display()))
}
