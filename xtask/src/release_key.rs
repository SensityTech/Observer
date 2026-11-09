//! `cargo xtask release-key <file>`: a new Ed25519 key for signing releases. The secret half goes
//! to `<file>` as PKCS#8 PEM, which `openssl pkeyutl -sign` reads in the release workflow; the
//! public half goes into the source, where Observer reads it to verify what it installs.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use ed25519_dalek::SigningKey;

use crate::root;

/// Where Observer's source keeps the public half, as 64 hexadecimal digits.
pub const PUBLIC: &str = "crates/observer-artifacts/src/releases/release-key.pub";
/// What comes before the 32-byte seed in an Ed25519 key's PKCS#8 encoding (RFC 8410).
const PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

pub fn create(secret: &str) -> anyhow::Result<()> {
    let secret = PathBuf::from(secret);
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|error| anyhow::anyhow!("no random bytes: {error}"))?;
    write_secret(&secret, &seed)?;
    let public = hex(&SigningKey::from_bytes(&seed).verifying_key().to_bytes());
    std::fs::write(root().join(PUBLIC), format!("{public}\n"))
        .with_context(|| format!("cannot write {PUBLIC}"))?;
    println!("public key {public}, written to {PUBLIC}");
    println!("secret key written to {}", secret.display());
    println!(
        "
Put the secret key in the release workflow's secret, then keep the file offline:
  gh secret set OBSERVER_RELEASE_KEY < {}
Commit {PUBLIC}: every Observer built from it trusts this key alone. A release signed with
another key, or a lost key, means installing the next release by hand.",
        secret.display()
    );
    Ok(())
}

/// The seed as PKCS#8 PEM, in a new file only its owner reads.
fn write_secret(path: &Path, seed: &[u8; 32]) -> anyhow::Result<()> {
    let mut options = File::options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path).with_context(|| {
        if path.exists() {
            format!("{} exists already: a key is never overwritten", path.display())
        } else {
            format!("cannot create {}", path.display())
        }
    })?;
    file.write_all(pem(seed).as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn pem(seed: &[u8; 32]) -> String {
    let mut der = PKCS8_PREFIX.to_vec();
    der.extend_from_slice(seed);
    format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        base64(&der)
    )
}

/// The seed a PEM written by [`pem`] holds.
#[cfg(test)]
fn seed_of(pem: &str) -> anyhow::Result<[u8; 32]> {
    let body: String = pem.lines().filter(|line| !line.starts_with("-----")).collect();
    let der = unbase64(&body)?;
    let Some(seed) = der.strip_prefix(PKCS8_PREFIX.as_slice()) else {
        anyhow::bail!("not an Ed25519 private key");
    };
    Ok(seed.try_into()?)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = chunk.iter().enumerate().fold(0_u32, |triple, (at, byte)| {
            triple | u32::from(*byte) << (16 - 8 * at)
        });
        for at in 0..4 {
            if at <= chunk.len() {
                text.push(char::from(ALPHABET[(triple >> (18 - 6 * at)) as usize & 63]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

#[cfg(test)]
fn unbase64(text: &str) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut bits = 0_u32;
    let mut count = 0;
    for character in text.bytes().filter(|byte| *byte != b'=') {
        let Some(value) = ALPHABET.iter().position(|letter| *letter == character) else {
            anyhow::bail!("not base64");
        };
        bits = bits << 6 | u32::try_from(value)?;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push(u8::try_from(bits >> count & 0xff)?);
        }
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_the_standard_alphabet_with_padding() {
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(unbase64("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn a_secret_key_reads_back_as_the_seed_it_was_written_from() {
        let seed = [7; 32];
        let pem = pem(&seed);

        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEI"));
        assert_eq!(seed_of(&pem).unwrap(), seed);
    }

    #[test]
    fn a_key_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("release.pem");
        std::fs::write(&path, b"the key in use").unwrap();

        assert!(write_secret(&path, &[1; 32]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"the key in use");
    }
}
