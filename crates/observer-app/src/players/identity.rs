//! What a report says a player is, and the checks before any of it is kept: FXServer's own player
//! list must agree, and an address is only ever kept as a keyed fingerprint.

use hmac::{Hmac, KeyInit, Mac};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use observer_domain::players::{IdentifierKind, Identifiers, address_network, clean_name};
use observer_fxserver::health::Listed;
use observer_fxserver::players::{Address, Shown};
use sha2::Sha256;

/// Keyed fingerprints: "same address as" or "this licence was forgotten" without the address or
/// the licence. The key lives in the state database.
#[derive(Clone)]
pub(crate) struct Fingerprints {
    key: [u8; 32],
}

impl Fingerprints {
    pub(crate) fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    fn mac(&self, purpose: &[u8], value: &[u8]) -> [u8; 16] {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC takes a key of any length");
        mac.update(purpose);
        mac.update(&[0]);
        mac.update(value);
        let digest = mac.finalize().into_bytes();
        let mut fingerprint = [0; 16];
        fingerprint.copy_from_slice(&digest[..16]);
        fingerprint
    }

    /// The fingerprint of the address's IPv4 address or IPv6 /64, in hex; `None` when it does not
    /// parse.
    pub(crate) fn address(&self, address: &Address) -> Option<String> {
        let network = address_network(address.expose())?;
        Some(hex(&self.mac(b"address", &network)))
    }

    pub(crate) fn license(&self, license: &str) -> [u8; 16] {
        self.mac(b"license", license.as_bytes())
    }
}

/// Keys and pseudonyms from a secret drawn at start and a counter: unique, unpredictable without
/// the secret, and never failing once Observer runs.
pub(crate) struct Keys {
    secret: Fingerprints,
    next: u64,
}

impl Keys {
    pub(crate) fn new(secret: [u8; 32]) -> Self {
        Self {
            secret: Fingerprints::new(secret),
            next: 0,
        }
    }

    pub(crate) fn next(&mut self) -> [u8; 16] {
        self.next += 1;
        self.secret.mac(b"key", &self.next.to_be_bytes())
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0xf)]));
    }
    text
}

/// What a join or a roster entry says of a player, until FXServer's list confirms it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Claim {
    pub(super) license: String,
    /// The identifiers FXServer's list also shows: compared with it.
    listed: Identifiers,
    /// Tokens and the address's fingerprint, which the list does not show: kept only with a
    /// confirmed claim.
    pub(super) others: Vec<(IdentifierKind, String)>,
    raw_name: Option<String>,
    pub(super) name: Option<String>,
}

impl Claim {
    /// `None` without a licence: such a player gets no identity. Also returns how many
    /// identifiers were dropped.
    pub(super) fn of(shown: &Shown, fingerprints: &Fingerprints) -> (Option<Self>, usize) {
        let listed = Identifiers::parse(shown.identifiers.iter().map(String::as_str), []);
        let tokens = Identifiers::parse([], shown.tokens.iter().map(String::as_str));
        let address = shown
            .address
            .as_ref()
            .map(|address| fingerprints.address(address));
        let dropped =
            shown.dropped + listed.invalid + tokens.invalid + usize::from(matches!(address, Some(None)));
        let Some(license) = listed.license.clone() else {
            return (None, dropped);
        };
        let mut others: Vec<(IdentifierKind, String)> = listed
            .others
            .iter()
            .chain(&tokens.others)
            .map(|identifier| (identifier.kind, identifier.value.clone()))
            .collect();
        others.extend(
            address
                .flatten()
                .map(|fingerprint| (IdentifierKind::Ip, fingerprint)),
        );
        let claim = Self {
            license,
            listed,
            others,
            name: shown.name.as_deref().and_then(clean_name),
            raw_name: shown.name.clone(),
        };
        (Some(claim), dropped)
    }

    /// Whether FXServer lists the player with the same licence, identifiers and name.
    pub(super) fn matches(&self, listed: &Listed) -> bool {
        let shown = Identifiers::parse(listed.identifiers.iter().map(String::as_str), []);
        shown.license == self.listed.license
            && shown.others == self.listed.others
            && listed.name == self.raw_name
    }
}

/// A length split across the days it covered, in `zone`, oldest first; at least one day.
pub(super) fn split_days(start_ms: i64, end_ms: i64, zone: &TimeZone) -> Vec<(String, i64)> {
    let day_of = |ms: i64| {
        Timestamp::from_millisecond(ms)
            .ok()
            .map(|at| at.to_zoned(zone.clone()).date())
    };
    let mut days = Vec::new();
    let mut from = start_ms.min(end_ms);
    while let Some(date) = day_of(from) {
        let next = date
            .tomorrow()
            .ok()
            .and_then(|tomorrow| tomorrow.to_zoned(zone.clone()).ok())
            .map_or(end_ms, |zoned| zoned.timestamp().as_millisecond());
        let until = next.min(end_ms).max(from);
        days.push((date.to_string(), until - from));
        if until >= end_ms {
            break;
        }
        from = until;
    }
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    const LICENSE: &str = "license:0123456789abcdef0123456789abcdef01234567";

    fn fingerprints() -> Fingerprints {
        Fingerprints::new([7; 32])
    }

    fn shown(identifiers: &[&str], name: &str) -> Shown {
        Shown {
            identifiers: identifiers.iter().map(|&text| text.to_owned()).collect(),
            name: Some(name.to_owned()),
            ..Shown::default()
        }
    }

    fn listed(identifiers: &[&str], name: &str) -> Listed {
        Listed {
            id: 1,
            identifiers: identifiers.iter().map(|&text| text.to_owned()).collect(),
            name: Some(name.to_owned()),
        }
    }

    fn ms(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }

    #[test]
    fn a_claim_matches_the_list_whatever_the_order_and_case() {
        let (claim, dropped) = Claim::of(
            &shown(&[LICENSE, "discord:123456789012345678"], "Ada"),
            &fingerprints(),
        );
        let claim = claim.unwrap();

        assert_eq!(dropped, 0);
        assert!(claim.matches(&listed(&["DISCORD:123456789012345678", LICENSE], "Ada")));
        assert!(!claim.matches(&listed(&[LICENSE, "discord:123456789012345678"], "Ada ")));
        assert!(!claim.matches(&listed(&[LICENSE], "Ada")));
        assert!(!claim.matches(&listed(
            &[
                "license:ffffffffffffffffffffffffffffffffffffffff",
                "discord:123456789012345678"
            ],
            "Ada"
        )));
    }

    #[test]
    fn a_player_without_a_licence_has_no_claim() {
        let (claim, dropped) = Claim::of(
            &shown(&["discord:123456789012345678", "steam:x"], "Ada"),
            &fingerprints(),
        );

        assert_eq!(claim, None);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn one_network_gives_one_fingerprint_and_the_key_matters() {
        let shown = |address: &str| Shown {
            identifiers: vec![LICENSE.to_owned()],
            address: Some(test_address(address)),
            ..Shown::default()
        };
        let fingerprint = |address: &str, keys: &Fingerprints| {
            Claim::of(&shown(address), keys).0.unwrap().others[0].1.clone()
        };

        let plain = fingerprint("ip:192.0.2.7", &fingerprints());

        assert_eq!(plain.len(), 32);
        assert_eq!(fingerprint("ip:[::ffff:192.0.2.7]:1", &fingerprints()), plain);
        assert_ne!(fingerprint("ip:192.0.2.8", &fingerprints()), plain);
        assert_ne!(fingerprint("ip:192.0.2.7", &Fingerprints::new([8; 32])), plain);
        assert!(!plain.contains("192"));
    }

    fn test_address(text: &str) -> Address {
        Address::new(text)
    }

    #[test]
    fn a_session_across_midnight_is_split_between_two_days() {
        let paris = TimeZone::get("Europe/Paris").unwrap();

        let days = split_days(ms("2026-10-30T21:00:00Z"), ms("2026-10-31T01:00:00Z"), &paris);

        assert_eq!(
            days,
            [
                ("2026-10-30".to_owned(), 2 * 3_600_000),
                ("2026-10-31".to_owned(), 2 * 3_600_000)
            ]
        );
    }

    #[test]
    fn a_day_with_a_daylight_saving_change_counts_its_real_length() {
        let paris = TimeZone::get("Europe/Paris").unwrap();

        // 25 October 2026 has 25 hours in Paris.
        let days = split_days(ms("2026-10-24T22:00:00Z"), ms("2026-10-25T23:00:00Z"), &paris);

        assert_eq!(days, [("2026-10-25".to_owned(), 25 * 3_600_000)]);
    }

    #[test]
    fn an_empty_session_still_has_its_day() {
        let days = split_days(
            ms("2026-10-31T10:00:00Z"),
            ms("2026-10-31T10:00:00Z"),
            &TimeZone::UTC,
        );

        assert_eq!(days, [("2026-10-31".to_owned(), 0)]);
    }
}
