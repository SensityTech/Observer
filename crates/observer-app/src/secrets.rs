//! Random secrets and their encodings. Secrets are stored as SHA-256 hashes only: a fast hash is
//! enough for 160 or 256 random bits, and a lookup by hash compares nothing secret-dependent.

use sha2::{Digest, Sha256};

use crate::Error;

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// No `0`/`O`, `1`/`I`/`l`: a temporary password is read off a screen and typed by hand.
const UNAMBIGUOUS: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const TEMPORARY_PASSWORD_CHARS: usize = 20;

pub(crate) fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(Error::Random)?;
    Ok(bytes)
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Compares two hashes in time that does not depend on where they differ.
pub(crate) fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0, |difference, (a, b)| difference | (a ^ b))
            == 0
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| [DIGITS[usize::from(byte >> 4)], DIGITS[usize::from(byte & 0xf)]])
        .map(char::from)
        .collect()
}

pub(crate) fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let (pairs, rest) = text.as_bytes().as_chunks::<2>();
    if pairs.len() != N || !rest.is_empty() {
        return None;
    }
    let mut bytes = [0; N];
    for (byte, &[high, low]) in bytes.iter_mut().zip(pairs) {
        *byte = digit(high)? << 4 | digit(low)?;
    }
    Some(bytes)
}

fn digit(symbol: u8) -> Option<u8> {
    char::from(symbol)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// RFC 4648 base32 without padding, for setup tokens: 20 bytes make 32 characters.
pub(crate) fn base32(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = buffer << 8 | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            text.push(char::from(BASE32[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        text.push(char::from(BASE32[((buffer << (5 - bits)) & 31) as usize]));
    }
    text
}

/// Reads base32 in either case, the way a link may have been retyped.
pub(crate) fn from_base32(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for symbol in text.bytes() {
        let value = BASE32
            .iter()
            .position(|&candidate| candidate == symbol.to_ascii_uppercase())?;
        buffer = buffer << 5 | u32::try_from(value).ok()?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
        }
    }
    Some(bytes)
}

/// Twenty characters from 57 symbols, about 116 bits.
pub(crate) fn temporary_password() -> Result<String, Error> {
    // 228 is the largest multiple of 57 under 256; bytes above it would favour the first symbols.
    let limit = u8::try_from(UNAMBIGUOUS.len() * 4).unwrap_or(u8::MAX);
    let mut password = String::with_capacity(TEMPORARY_PASSWORD_CHARS);
    while password.len() < TEMPORARY_PASSWORD_CHARS {
        for byte in random::<32>()? {
            if byte < limit && password.len() < TEMPORARY_PASSWORD_CHARS {
                password.push(char::from(UNAMBIGUOUS[usize::from(byte) % UNAMBIGUOUS.len()]));
            }
        }
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_the_wrong_length() {
        let bytes = [0x00, 0x7f, 0xa5, 0xff];

        assert_eq!(hex(&bytes), "007fa5ff");
        assert_eq!(from_hex::<4>("007fa5ff"), Some(bytes));
        assert_eq!(from_hex::<4>("007FA5FF"), Some(bytes));
        assert_eq!(from_hex::<4>("007fa5f"), None);
        assert_eq!(from_hex::<4>("007fa5fg"), None);
        assert_eq!(from_hex::<4>("+07fa5ff"), None);
    }

    #[test]
    fn base32_matches_rfc_4648_and_reads_either_case() {
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(from_base32("mzxw6ytboi").unwrap(), b"foobar");

        let token = [0x5a; 20];
        let text = base32(&token);
        assert_eq!(text.len(), 32);
        assert_eq!(from_base32(&text).unwrap(), token);
        assert_eq!(from_base32("MZXW6YTB0I"), None);
    }

    #[test]
    fn temporary_passwords_use_unambiguous_characters_only() {
        let password = temporary_password().unwrap();

        assert_eq!(password.len(), 20);
        assert!(password.bytes().all(|byte| UNAMBIGUOUS.contains(&byte)));
        assert_ne!(password, temporary_password().unwrap());
    }

    #[test]
    fn hashes_compare_equal_only_when_identical() {
        assert!(same(&sha256(b"key"), &sha256(b"key")));
        assert!(!same(&sha256(b"key"), &sha256(b"kez")));
        assert!(!same(&[1, 2], &[1, 2, 3]));
    }
}
