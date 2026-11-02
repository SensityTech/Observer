//! AWS Signature Version 4, as S3 and the stores compatible with it check it: the canonical
//! request, the string to sign under the day's scope, and the key derived from the secret for that
//! day, region and service. Only what object requests need: no query string, the payload's digest
//! signed in `x-amz-content-sha256`.

use std::fmt::Write as _;

use hmac::{Hmac, KeyInit, Mac};
use jiff::Timestamp;
use sha2::{Digest, Sha256};

/// The digest of an empty payload.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Clone, Copy)]
pub struct Credentials<'a> {
    pub access_key_id: &'a str,
    pub secret: &'a str,
    pub region: &'a str,
}

/// One request to sign.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub method: &'a str,
    /// As the request line sends it, every segment already encoded by [`encode_path`].
    pub path: &'a str,
    /// `host` or `host:port`, as the `Host` header carries it.
    pub host: &'a str,
    /// The other headers signed, names in lower case: `x-amz-date` and `x-amz-content-sha256`
    /// always, and any the request adds.
    pub headers: &'a [(&'a str, &'a str)],
    pub payload_sha256: &'a str,
    pub at: Timestamp,
}

/// `20130524T000000Z`, as `x-amz-date` carries it.
pub fn amz_date(at: Timestamp) -> String {
    at.strftime("%Y%m%dT%H%M%SZ").to_string()
}

/// A key as S3 expects it in a path: every byte but the unreserved ones and `/` percent-encoded.
pub fn encode_path(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len());
    for byte in key.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// The `Authorization` header for the request.
pub fn authorization(credentials: Credentials<'_>, request: &Request<'_>) -> String {
    let mut headers: Vec<(&str, String)> = request
        .headers
        .iter()
        .map(|(name, value)| (*name, value.trim().to_owned()))
        .chain([("host", request.host.to_owned())])
        .collect();
    headers.sort_by(|a, b| a.0.cmp(b.0));
    let signed: Vec<&str> = headers.iter().map(|(name, _)| *name).collect();
    let signed = signed.join(";");
    let mut canonical = format!("{}\n{}\n\n", request.method, request.path);
    for (name, value) in &headers {
        let _ = writeln!(canonical, "{name}:{value}");
    }
    let _ = write!(canonical, "\n{signed}\n{}", request.payload_sha256);

    let day = request.at.strftime("%Y%m%d").to_string();
    let scope = format!("{day}/{}/s3/aws4_request", credentials.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        amz_date(request.at),
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let key = [day.as_str(), credentials.region, "s3", "aws4_request"]
        .into_iter()
        .fold(format!("AWS4{}", credentials.secret).into_bytes(), |key, part| {
            hmac(&key, part.as_bytes()).to_vec()
        });
    let signature = hex(&hmac(&key, to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
        credentials.access_key_id
    )
}

fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

pub fn hex(bytes: &[u8]) -> String {
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

    // The examples of "Authenticating Requests: Using the Authorization Header", in the S3 API
    // reference.
    const CREDENTIALS: Credentials<'static> = Credentials {
        access_key_id: "AKIAIOSFODNN7EXAMPLE",
        secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        region: "us-east-1",
    };

    fn at() -> Timestamp {
        "2013-05-24T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn a_get_with_a_range_signs_as_aws_documents_it() {
        let headers = [
            ("range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let request = Request {
            method: "GET",
            path: "/test.txt",
            host: "examplebucket.s3.amazonaws.com",
            headers: &headers,
            payload_sha256: EMPTY_SHA256,
            at: at(),
        };

        assert_eq!(
            authorization(CREDENTIALS, &request),
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn a_put_with_an_escaped_key_signs_as_aws_documents_it() {
        let payload = hex(&Sha256::digest(b"Welcome to Amazon S3."));
        let headers = [
            ("date", "Fri, 24 May 2013 00:00:00 GMT"),
            ("x-amz-content-sha256", payload.as_str()),
            ("x-amz-date", "20130524T000000Z"),
            ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
        ];
        let path = format!("/{}", encode_path("test$file.text"));
        let request = Request {
            method: "PUT",
            path: &path,
            host: "examplebucket.s3.amazonaws.com",
            headers: &headers,
            payload_sha256: &payload,
            at: at(),
        };

        assert_eq!(path, "/test%24file.text");
        assert_eq!(
            authorization(CREDENTIALS, &request),
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class, \
             Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    #[test]
    fn keys_keep_their_slashes_and_lose_everything_else() {
        assert_eq!(
            encode_path("database/es extended+1.sql.gz"),
            "database/es%20extended%2B1.sql.gz"
        );
        assert_eq!(encode_path("é"), "%C3%A9");
    }
}
