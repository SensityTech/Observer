//! A release as GitHub serves it, signed by a key only tests and test builds know: its archive for
//! this platform, the same shape `cargo xtask dist` writes, its checksums, their signature, and
//! the API's answer for the latest release.
#![allow(clippy::unwrap_used, reason = "test support, where a failure is the test's")]

use std::fmt::Write as _;
use std::io::{Cursor, Write};

use ed25519_dalek::{Signer, SigningKey};
use observer_domain::updates::{self, REPOSITORY, SIGNATURE, SUMS};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{ReleaseKey, key};

/// The seed of the test key: in the source, so it signs nothing anyone should trust.
const SEED: [u8; 32] = *b"observer's test release key seed";

/// The public half of the test key, which test builds accept in place of the release key.
pub fn test_key() -> ReleaseKey {
    ReleaseKey::from_bytes(&SigningKey::from_bytes(&SEED).verifying_key().to_bytes()).unwrap()
}

/// Where each download is served.
const DOWNLOADS: &str = "/download/";

/// The path GitHub's API answers the latest release on.
pub fn latest_path() -> String {
    format!("/repos/{REPOSITORY}/releases/latest")
}

#[derive(Debug, Clone)]
pub struct FakeRelease {
    pub version: String,
    pub notes: String,
    files: Vec<(String, Vec<u8>)>,
}

impl FakeRelease {
    /// A release of `version` whose archive for this platform holds `executable`, with
    /// checksums signed by the test key.
    pub fn new(version: &str, executable: &[u8]) -> Self {
        Self::holding(
            version,
            updates::executable_name(std::env::consts::OS),
            executable,
        )
    }

    /// A release whose archive holds `executable` under the name `entry`.
    pub fn holding(version: &str, entry: &str, executable: &[u8]) -> Self {
        let name = updates::archive_name(version, std::env::consts::OS, std::env::consts::ARCH);
        let archive = archive(&name, entry, executable);
        let mut release = Self {
            version: version.to_owned(),
            notes: notes(version),
            files: vec![(name, archive)],
        };
        release.sign();
        release
    }

    pub fn archive_name(&self) -> String {
        updates::archive_name(&self.version, std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn file(&self, name: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(listed, _)| listed == name)
            .map(|(_, bytes)| bytes.as_slice())
    }

    /// Puts `bytes` in the place of a file, or adds it; the checksums are not written again.
    pub fn set(&mut self, name: &str, bytes: Vec<u8>) {
        self.remove(name);
        self.files.push((name.to_owned(), bytes));
    }

    pub fn remove(&mut self, name: &str) {
        self.files.retain(|(listed, _)| listed != name);
    }

    /// Writes the checksums of the archives again, and signs them with the test key.
    pub fn sign(&mut self) {
        let sums = self
            .files
            .iter()
            .filter(|(name, _)| name != SUMS && name != SIGNATURE)
            .fold(String::new(), |mut sums, (name, bytes)| {
                let _ = writeln!(sums, "{}  {name}", key::hex(&Sha256::digest(bytes)));
                sums
            });
        let signature = SigningKey::from_bytes(&SEED)
            .sign(sums.as_bytes())
            .to_bytes()
            .to_vec();
        self.set(SUMS, sums.into_bytes());
        self.set(SIGNATURE, signature);
    }

    /// The API's answer for the latest release, its downloads under `base`, such as
    /// `http://127.0.0.1:41950`.
    pub fn json(&self, base: &str) -> String {
        let assets: Vec<_> = self
            .files
            .iter()
            .map(|(name, bytes)| {
                json!({
                    "name": name,
                    "size": bytes.len(),
                    "state": "uploaded",
                    "browser_download_url": format!("{base}{DOWNLOADS}v{}/{name}", self.version),
                    "digest": format!("sha256:{}", key::hex(&Sha256::digest(bytes))),
                })
            })
            .collect();
        json!({
            "tag_name": format!("v{}", self.version),
            "name": format!("Observer {}", self.version),
            "body": self.notes,
            "html_url": format!("https://github.com/{REPOSITORY}/releases/tag/v{}", self.version),
            "draft": false,
            "prerelease": false,
            "immutable": true,
            "published_at": "2026-11-09T10:00:00Z",
            "assets": assets,
        })
        .to_string()
    }

    /// The answer to a `GET` of `path`: the latest release, or a download. `None` for a 404.
    pub fn answer(&self, path: &str, base: &str) -> Option<Vec<u8>> {
        if path == latest_path() {
            return Some(self.json(base).into_bytes());
        }
        let name = path
            .strip_prefix(DOWNLOADS)?
            .strip_prefix(&format!("v{}/", self.version))?;
        self.file(name).map(<[u8]>::to_vec)
    }
}

fn notes(version: &str) -> String {
    format!(
        "Observer {version}, built from 0123456789ab.\n\n\
         - `{}`: Windows 10 or Server 2016 and later, x64.\n\
         - `{}`: Linux x86_64 with glibc 2.35 or later.\n\
         - `SHA256SUMS`: check the archives with `sha256sum -c SHA256SUMS`.\n\n\
         ## What's Changed\n\
         * Install Observer's updates from the panel in https://github.com/{REPOSITORY}/pull/45\n\
         * Draw Observer's mark as a ring with its bead in https://github.com/{REPOSITORY}/pull/44\n\n\
         **Full Changelog**: https://github.com/{REPOSITORY}/compare/v0.1.0...v{version}\n",
        updates::archive_name(version, "windows", "x86_64"),
        updates::archive_name(version, "linux", "x86_64"),
    )
}

/// The executable and the licences, as `cargo xtask dist` packs them: a zip when `name` ends in
/// `.zip`, a gzipped tar otherwise.
pub fn archive(name: &str, program: &str, executable: &[u8]) -> Vec<u8> {
    const LICENCES: &[u8] = b"Third-party licences.\n";
    if super::stage::is_zip(name) {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (entry, data) in [(program, executable), ("THIRD-PARTY-LICENCES.txt", LICENCES)] {
            zip.start_file(entry, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    } else {
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gzip);
        for (entry, data, mode) in [
            (program, executable, 0o755),
            ("THIRD-PARTY-LICENCES.txt", LICENCES, 0o644),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(mode);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            tar.append_data(&mut header, entry, data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }
}
