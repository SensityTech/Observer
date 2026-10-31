//! What a reverse proxy that terminates TLS says of the requests it forwards. Only the proxies the
//! owner listed are heard: any client could write the same headers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use serde::Deserialize;

use crate::AppState;

/// A year, and never `includeSubDomains`: the other names of the owner's domain are not
/// Observer's to decide.
const STRICT_TRANSPORT_SECURITY: &str = "max-age=31536000";

/// An address or a CIDR block whose `X-Forwarded-For` and `X-Forwarded-Proto` Observer believes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct TrustedProxy {
    network: IpAddr,
    prefix: u8,
}

#[derive(Debug, thiserror::Error)]
#[error("`{0}` is not an address or a CIDR block such as 10.0.0.0/8")]
pub struct InvalidProxy(String);

impl FromStr for TrustedProxy {
    type Err = InvalidProxy;

    /// Bits past the prefix are dropped, as Traefik and nginx do: `10.0.0.1/8` is `10.0.0.0/8`.
    fn from_str(entry: &str) -> Result<Self, InvalidProxy> {
        let invalid = || InvalidProxy(entry.to_owned());
        let (address, prefix) = match entry.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (entry, None),
        };
        let address = address.parse::<IpAddr>().map_err(|_| invalid())?;
        let bits = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => prefix.parse::<u8>().map_err(|_| invalid())?,
            None => bits,
        };
        if prefix > bits {
            return Err(invalid());
        }
        Ok(Self {
            network: masked(address, prefix),
            prefix,
        })
    }
}

impl TryFrom<String> for TrustedProxy {
    type Error = InvalidProxy;

    fn try_from(entry: String) -> Result<Self, InvalidProxy> {
        entry.parse()
    }
}

impl TrustedProxy {
    /// An IPv4 peer of a dual-stack listener arrives mapped into IPv6, and still matches its
    /// IPv4 entry.
    fn contains(self, ip: IpAddr) -> bool {
        match (self.network, ip.to_canonical()) {
            (IpAddr::V4(_), ip @ IpAddr::V4(_)) | (IpAddr::V6(_), ip @ IpAddr::V6(_)) => {
                masked(ip, self.prefix) == self.network
            }
            _ => false,
        }
    }
}

/// `ip` with every bit past the first `prefix` cleared. The prefix fits the address's family.
fn masked(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(ip) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            Ipv4Addr::from_bits(ip.to_bits() & mask).into()
        }
        IpAddr::V6(ip) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            Ipv6Addr::from_bits(ip.to_bits() & mask).into()
        }
    }
}

/// Where a request comes from once the trusted proxies are heard: the client's address, and
/// whether the browser reached the proxy over HTTPS. Without a trusted proxy in front, it is the
/// TCP peer over plain HTTP, since Observer serves no TLS itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Remote {
    pub ip: IpAddr,
    pub https: bool,
}

impl Remote {
    pub(crate) fn resolve(peer: IpAddr, headers: &HeaderMap, proxies: &[TrustedProxy]) -> Self {
        if !trusted(peer, proxies) {
            return Self {
                ip: peer,
                https: false,
            };
        }
        Self {
            ip: forwarded_for(headers, proxies).unwrap_or(peer),
            https: forwarded_https(headers),
        }
    }

    /// The scheme of the page the browser shows, which its `Origin` names.
    pub(crate) fn scheme(self) -> &'static str {
        if self.https { "https" } else { "http" }
    }
}

fn trusted(ip: IpAddr, proxies: &[TrustedProxy]) -> bool {
    proxies.iter().any(|proxy| proxy.contains(ip))
}

/// Each proxy appends the address it took the request from, so walking the list from the right,
/// the first address that is not a trusted proxy is the client. What lies further left came from
/// the client itself and is never read. `None` when the header is missing or an entry the walk
/// reaches is not an address.
fn forwarded_for(headers: &HeaderMap, proxies: &[TrustedProxy]) -> Option<IpAddr> {
    let mut leftmost = None;
    for entry in entries(headers, "x-forwarded-for").rev() {
        let ip = std::str::from_utf8(entry).ok()?.parse::<IpAddr>().ok()?;
        if !trusted(ip, proxies) {
            return Some(ip);
        }
        leftmost = Some(ip);
    }
    // Every hop is a trusted proxy: the request started on one of their hosts.
    leftmost
}

/// The last value is the one the nearest proxy wrote. Traefik names the scheme of a websocket
/// opening `wss` where it would write `https`.
fn forwarded_https(headers: &HeaderMap) -> bool {
    entries(headers, "x-forwarded-proto")
        .next_back()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(b"https") || scheme.eq_ignore_ascii_case(b"wss"))
}

/// The comma-separated entries of every `name` header, in order, trimmed.
fn entries<'a>(headers: &'a HeaderMap, name: &'static str) -> impl DoubleEndedIterator<Item = &'a [u8]> {
    headers
        .get_all(name)
        .iter()
        .flat_map(|value| value.as_bytes().split(|&byte| byte == b','))
        .map(<[u8]>::trim_ascii)
}

/// Hears the trusted proxy before anything reads the request's client or scheme. Behind HTTPS,
/// every response also tells the browser to come back over HTTPS only.
pub(crate) async fn hear(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut request: Request,
    next: Next,
) -> Response {
    let remote = Remote::resolve(peer.ip(), request.headers(), &state.proxies);
    request.extensions_mut().insert(remote);
    let mut response = next.run(request).await;
    if remote.https {
        response.headers_mut().insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(STRICT_TRANSPORT_SECURITY),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: [u8; 4] = [10, 0, 0, 2];

    fn proxies(entries: &[&str]) -> Vec<TrustedProxy> {
        entries.iter().map(|entry| entry.parse().unwrap()).collect()
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for &(name, value) in pairs {
            headers.append(name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    fn client(pairs: &[(&'static str, &str)]) -> Remote {
        Remote::resolve(PEER.into(), &headers(pairs), &proxies(&["10.0.0.0/8"]))
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn an_entry_is_an_address_or_a_cidr_block_of_either_family() {
        for valid in [
            "127.0.0.1",
            "10.0.0.0/8",
            "0.0.0.0/0",
            "::1",
            "fd00::/8",
            "2001:db8::/32",
        ] {
            assert!(valid.parse::<TrustedProxy>().is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "localhost",
            "10.0.0.0/",
            "10.0.0.0/33",
            "fd00::/129",
            "10.0.0.0/8/8",
            "10.0.0.0/eight",
            "10.0.0",
            "fe80::1%eth0",
            "[::1]",
            "127.0.0.1:443",
        ] {
            let error = invalid.parse::<TrustedProxy>().unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("`{invalid}` is not an address or a CIDR block such as 10.0.0.0/8")
            );
        }
    }

    #[test]
    fn a_block_holds_the_addresses_under_its_prefix() {
        let block = |entry: &str| entry.parse::<TrustedProxy>().unwrap();

        assert!(block("10.0.0.0/8").contains(ip("10.255.3.4")));
        assert!(!block("10.0.0.0/8").contains(ip("11.0.0.1")));
        assert!(
            block("10.0.0.1/8").contains(ip("10.9.9.9")),
            "the host bits are dropped"
        );
        assert!(block("172.18.0.2").contains(ip("172.18.0.2")));
        assert!(!block("172.18.0.2").contains(ip("172.18.0.3")));
        assert!(block("fd00::/8").contains(ip("fd12:3456::1")));
        assert!(!block("fd00::/8").contains(ip("fe80::1")));
        assert!(block("127.0.0.1").contains(ip("::ffff:127.0.0.1")));
        assert!(!block("0.0.0.0/0").contains(ip("::2")));
        assert!(!block("::/0").contains(ip("192.0.2.1")));
    }

    #[test]
    fn headers_from_a_peer_that_is_not_a_trusted_proxy_are_ignored() {
        let forged = headers(&[
            ("x-forwarded-for", "198.51.100.20"),
            ("x-forwarded-proto", "https"),
        ]);

        for listed in [&[][..], &["10.0.0.0/8"][..]] {
            let remote = Remote::resolve(ip("203.0.113.7"), &forged, &proxies(listed));
            assert_eq!(
                remote,
                Remote {
                    ip: ip("203.0.113.7"),
                    https: false
                }
            );
        }
    }

    #[test]
    fn the_client_is_the_first_address_from_the_right_that_is_not_a_trusted_proxy() {
        assert_eq!(
            client(&[("x-forwarded-for", "198.51.100.20")]).ip,
            ip("198.51.100.20")
        );
        assert_eq!(
            client(&[("x-forwarded-for", "198.51.100.20, 10.0.0.9")]).ip,
            ip("198.51.100.20"),
            "behind a chain of trusted proxies"
        );
        assert_eq!(
            client(&[("x-forwarded-for", "192.0.2.66, 198.51.100.20,10.0.0.9")]).ip,
            ip("198.51.100.20"),
            "what the client wrote itself is left of the address its proxy saw"
        );
        assert_eq!(
            client(&[("x-forwarded-for", "not an address, 198.51.100.20")]).ip,
            ip("198.51.100.20"),
            "nor is it read"
        );
        assert_eq!(
            client(&[
                ("x-forwarded-for", "192.0.2.66"),
                ("x-forwarded-for", "2001:db8::7, 10.0.0.9"),
            ])
            .ip,
            ip("2001:db8::7"),
            "the header may come as several lines"
        );
    }

    #[test]
    fn when_every_hop_is_a_trusted_proxy_the_client_is_the_leftmost() {
        let remote = client(&[("x-forwarded-for", "10.1.0.5, 10.0.0.9")]);

        assert_eq!(remote.ip, ip("10.1.0.5"));
    }

    #[test]
    fn a_missing_or_malformed_header_leaves_the_peer() {
        for pairs in [
            &[][..],
            &[("x-forwarded-for", "")][..],
            &[("x-forwarded-for", "198.51.100.20, unknown")][..],
            &[("x-forwarded-for", "198.51.100.20:50000")][..],
            &[("x-forwarded-for", "[2001:db8::7]")][..],
        ] {
            assert_eq!(client(pairs).ip, ip("10.0.0.2"), "{pairs:?}");
        }
    }

    #[test]
    fn the_scheme_is_the_last_forwarded_proto_and_plain_http_otherwise() {
        let https = |pairs: &[(&'static str, &str)]| client(pairs).https;

        assert!(https(&[("x-forwarded-proto", "https")]));
        assert!(https(&[("x-forwarded-proto", "HTTPS")]));
        assert!(https(&[("x-forwarded-proto", "http, https")]));
        assert!(!https(&[("x-forwarded-proto", "https, http")]));
        assert!(!https(&[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-proto", "http")
        ]));
        assert!(https(&[("x-forwarded-proto", "wss")]), "Traefik's websocket");
        assert!(!https(&[("x-forwarded-proto", "ws")]));
        assert!(!https(&[]));
        assert_eq!(client(&[("x-forwarded-proto", "https")]).scheme(), "https");
        assert_eq!(client(&[]).scheme(), "http");
    }
}
