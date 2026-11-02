//! Against a store on the loopback that checks what S3 checks: the signature, recomputed from the
//! request it got, and the body against its signed digest.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap as Headers, Method as Verb, StatusCode as Status, Uri};
use axum::response::{IntoResponse, Response as Answer};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use super::*;

const ACCESS_KEY: &str = "GKOBSERVERTEST";
const SECRET: &str = "s3cr3t/with+signs";

#[derive(Default)]
struct Store {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    /// Refuse every request as if the key were wrong.
    wrong_key: bool,
}

fn refusal(status: Status, code: &str) -> Answer {
    let body =
        format!("<?xml version=\"1.0\"?><Error><Code>{code}</Code><Message>{code} here</Message></Error>");
    (status, body).into_response()
}

async fn serve(
    State(store): State<Arc<Store>>,
    verb: Verb,
    uri: Uri,
    headers: Headers,
    body: Bytes,
) -> Answer {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    };
    let (date, payload) = (header("x-amz-date"), header("x-amz-content-sha256"));
    let at = jiff::fmt::strtime::parse("%Y%m%dT%H%M%SZ", date)
        .and_then(|parsed| parsed.to_datetime())
        .and_then(|time| time.to_zoned(jiff::tz::TimeZone::UTC))
        .map(|zoned| zoned.timestamp());
    let Ok(at) = at else {
        return refusal(Status::FORBIDDEN, "AccessDenied");
    };
    let signed = [("x-amz-content-sha256", payload), ("x-amz-date", date)];
    let expected = sigv4::authorization(
        Credentials {
            access_key_id: ACCESS_KEY,
            secret: SECRET,
            region: "garage",
        },
        &Request {
            method: verb.as_str(),
            path: uri.path(),
            host: header("host"),
            headers: &signed,
            payload_sha256: payload,
            at,
        },
    );
    if store.wrong_key || header("authorization") != expected {
        return refusal(Status::FORBIDDEN, "SignatureDoesNotMatch");
    }
    let key = uri.path().to_owned();
    let mut objects = store.objects.lock().unwrap();
    match verb {
        Verb::PUT => {
            if header("content-length").is_empty() {
                return refusal(Status::LENGTH_REQUIRED, "MissingContentLength");
            }
            if sigv4::hex(&Sha256::digest(&body)) != payload {
                return refusal(Status::BAD_REQUEST, "XAmzContentSHA256Mismatch");
            }
            objects.insert(key, body.to_vec());
            Status::OK.into_response()
        }
        Verb::GET => match objects.get(&key) {
            Some(object) => object.clone().into_response(),
            None => refusal(Status::NOT_FOUND, "NoSuchKey"),
        },
        Verb::DELETE => {
            objects.remove(&key);
            Status::NO_CONTENT.into_response()
        }
        _ => refusal(Status::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
    }
}

async fn start(store: Store) -> (Arc<Store>, String) {
    let store = Arc::new(store);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = axum::Router::new().fallback(serve).with_state(Arc::clone(&store));
    tokio::spawn(async move { axum::serve(listener, router).await });
    (store, format!("http://{address}"))
}

fn bucket(endpoint: &str) -> Bucket {
    Bucket::new(
        BucketSettings {
            endpoint,
            region: "garage",
            bucket: "sensity-backups",
            prefix: "database/",
            access_key_id: ACCESS_KEY,
            path_style: true,
        },
        &Secret::new(SECRET),
    )
    .unwrap()
}

fn backup(dir: &std::path::Path, bytes: &[u8]) -> (std::path::PathBuf, String) {
    let path = dir.join("es_extended-20261102T040000Z-7.sql.gz");
    std::fs::write(&path, bytes).unwrap();
    (path, sigv4::hex(&Sha256::digest(bytes)))
}

#[tokio::test]
async fn a_backup_goes_up_signed_comes_back_and_goes_away() {
    let (store, endpoint) = start(Store::default()).await;
    let bucket = bucket(&endpoint);
    let dir = tempfile::tempdir().unwrap();
    let (path, sha256) = backup(dir.path(), b"gzipped dump");
    let key = bucket.key("es_extended-20261102T040000Z-7.sql.gz");
    let progress = Arc::new(AtomicU64::new(0));

    bucket
        .put(
            &key,
            &path,
            12,
            &sha256,
            Arc::clone(&progress),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(progress.load(Ordering::Relaxed), 12);
    let stored = "/sensity-backups/database/es_extended-20261102T040000Z-7.sql.gz";
    assert_eq!(store.objects.lock().unwrap()[stored], b"gzipped dump");
    let body = bucket.get(&key).await.unwrap().bytes().await.unwrap();
    assert_eq!(body.as_ref(), b"gzipped dump");

    bucket.delete(&key).await.unwrap();
    bucket.delete(&key).await.unwrap();
    assert!(store.objects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_body_that_is_not_the_signed_one_is_refused() {
    let (store, endpoint) = start(Store::default()).await;
    let bucket = bucket(&endpoint);
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = backup(dir.path(), b"gzipped dump");
    let wrong = sigv4::hex(&Sha256::digest(b"another dump"));

    let failure = bucket
        .put(
            "database/x.sql.gz",
            &path,
            12,
            &wrong,
            Arc::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert_eq!(failure.error, BackupError::VerifyFailed);
    assert!(failure.detail.unwrap().contains("XAmzContentSHA256Mismatch"));
    assert!(store.objects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_wrong_key_says_so_without_the_secret() {
    let (_, endpoint) = start(Store {
        wrong_key: true,
        ..Store::default()
    })
    .await;

    let failure = bucket(&endpoint).test().await.unwrap_err();

    assert_eq!(failure.error, BackupError::Authentication);
    let detail = failure.detail.unwrap();
    assert!(detail.starts_with("HTTP 403 SignatureDoesNotMatch"), "{detail}");
    assert!(!detail.contains(SECRET));
}

#[tokio::test]
async fn the_test_writes_then_deletes_its_own_object() {
    let (store, endpoint) = start(Store::default()).await;

    bucket(&endpoint).test().await.unwrap();

    assert!(store.objects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_endpoint_that_does_not_answer_is_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);

    let failure = bucket(&endpoint).test().await.unwrap_err();

    assert_eq!(failure.error, BackupError::Unreachable);
}

#[tokio::test]
async fn an_object_larger_than_one_upload_is_refused_before_sending() {
    let bucket = bucket("http://127.0.0.1:9");
    let dir = tempfile::tempdir().unwrap();
    let (path, sha256) = backup(dir.path(), b"x");

    let failure = bucket
        .put(
            "database/x",
            &path,
            MAX_SINGLE_PUT + 1,
            &sha256,
            Arc::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert_eq!(failure.error, BackupError::TooLarge);
}

#[test]
fn virtual_hosted_addresses_put_the_bucket_in_the_host() {
    let mut bucket = bucket("https://s3.eu-west-3.amazonaws.com");
    bucket.path_style = false;

    let url = bucket.url("database/es extended.sql.gz").unwrap();

    assert_eq!(
        url.as_str(),
        "https://sensity-backups.s3.eu-west-3.amazonaws.com/database/es%20extended.sql.gz"
    );
}
