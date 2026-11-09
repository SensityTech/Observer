use ed25519_dalek::{Signature, VerifyingKey};

/// The public half of the key releases are signed with, which `cargo xtask release-key` writes
/// here as 64 hexadecimal digits. Empty in a build made before the key existed.
const PUBLISHED: &str = include_str!("release-key.pub");

/// An Ed25519 public key that signs the checksums of Observer's releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseKey(VerifyingKey);

impl ReleaseKey {
    /// The key this build was made with, if any.
    pub fn published() -> Option<Self> {
        Self::from_hex(PUBLISHED.trim())
    }

    /// 64 hexadecimal digits; a key of small order, which anyone can forge for, is refused.
    pub fn from_hex(text: &str) -> Option<Self> {
        let bytes = bytes32(text)?;
        let key = VerifyingKey::from_bytes(&bytes).ok()?;
        (!key.is_weak()).then_some(Self(key))
    }

    pub fn from_bytes(bytes: &[u8; 32]) -> Option<Self> {
        let key = VerifyingKey::from_bytes(bytes).ok()?;
        (!key.is_weak()).then_some(Self(key))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// Whether `signature` is this key's over `message`, read strictly: no malleable encoding.
    pub fn verifies(&self, message: &[u8], signature: &[u8]) -> bool {
        let Ok(signature) = <[u8; 64]>::try_from(signature) else {
            return false;
        };
        self.0
            .verify_strict(message, &Signature::from_bytes(&signature))
            .is_ok()
    }
}

/// 32 bytes written as 64 hexadecimal digits, in either case.
pub(crate) fn bytes32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0; 32];
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    for (byte, pair) in bytes.iter_mut().zip(pairs) {
        let pair = std::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(bytes)
}

/// The checksum `sums` gives `name`, in the format `sha256sum` writes: `<hex>  <name>`, or
/// `<hex> *<name>` in binary mode. A name listed twice is no answer.
pub(crate) fn checksum_of(sums: &str, name: &str) -> Option<[u8; 32]> {
    let mut found = None;
    for line in sums.lines() {
        let Some((hash, listed)) = line.split_once(' ') else {
            continue;
        };
        let listed = listed.strip_prefix(' ').or_else(|| listed.strip_prefix('*'));
        if listed == Some(name) {
            if found.is_some() {
                return None;
            }
            found = Some(bytes32(hash)?);
        }
    }
    found
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}
