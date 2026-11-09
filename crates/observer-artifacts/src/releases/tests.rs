use std::sync::Arc;

use observer_domain::updates::{SIGNATURE, SUMS};
use tokio_util::sync::CancellationToken;

use super::fake::{FakeRelease, latest_path, test_key};
use super::*;
use crate::test_server::{Reply, TestServer};

const EXECUTABLE: &[u8] = b"\x7fELF or MZ, the new Observer";

/// Serves `release` as GitHub would, the `ETag` `"v1"` making a repeated check a 304.
async fn serve(release: FakeRelease) -> TestServer {
    let release = Arc::new(release);
    TestServer::start(move |request, _| {
        if request.path == latest_path() && request.header("If-None-Match") == Some("\"v1\"") {
            return Reply::status(304);
        }
        let base = format!("http://{}", request.header("Host").unwrap_or_default());
        match release.answer(&request.path, &base) {
            Some(body) if request.path == latest_path() => Reply::ok(body).header("ETag", "\"v1\""),
            Some(body) => Reply::ok(body),
            None => Reply::status(404),
        }
    })
    .await
}

async fn found(releases: &Releases) -> Published {
    match releases.latest(None).await.unwrap() {
        Latest::Found { release, .. } => release,
        other => panic!("{other:?}"),
    }
}

async fn stage(release: FakeRelease) -> (Result<Staged, Error>, tempfile::TempDir) {
    let server = serve(release).await;
    let releases = Releases::local(server.port, Some(test_key()));
    let published = found(&releases).await;
    let dir = tempfile::tempdir().unwrap();
    let staged = releases
        .stage(&published, dir.path(), &CancellationToken::new(), &mut |_| {})
        .await;
    (staged, dir)
}

#[tokio::test]
async fn the_latest_release_is_read_with_its_assets_and_an_unchanged_one_costs_a_304() {
    let server = serve(FakeRelease::new("0.2.0", EXECUTABLE)).await;
    let releases = Releases::local(server.port, Some(test_key()));

    let Latest::Found { release, etag } = releases.latest(None).await.unwrap() else {
        panic!("no release");
    };

    assert_eq!(release.version, semver::Version::new(0, 2, 0));
    assert_eq!(release.name, "Observer 0.2.0");
    assert!(release.notes.contains("## What's Changed"));
    assert!(release.signed());
    let archive = release.archive().unwrap();
    assert!(archive.digest.is_some());
    assert_eq!(etag.as_deref(), Some("\"v1\""));
    assert_eq!(releases.latest(Some("\"v1\"")).await.unwrap(), Latest::Unchanged);
    let first = &server.requests()[0];
    assert_eq!(first.header("Accept"), Some("application/vnd.github+json"));
}

#[tokio::test]
async fn no_release_at_all_is_told_apart_from_a_failure() {
    let server = TestServer::start(|_, _| Reply::status(404)).await;
    let releases = Releases::local(server.port, Some(test_key()));

    assert_eq!(releases.latest(None).await.unwrap(), Latest::None);
}

#[tokio::test]
async fn a_spent_allowance_says_when_it_comes_back() {
    let server = TestServer::start(|_, _| {
        Reply::status(403)
            .header("x-ratelimit-remaining", "0")
            .header("x-ratelimit-reset", "1794204723")
    })
    .await;
    let releases = Releases::local(server.port, Some(test_key()));

    let error = releases.latest(None).await.unwrap_err();

    assert!(
        matches!(error, Error::RateLimited { until: Some(until) } if until.as_second() == 1_794_204_723),
        "{error}"
    );
}

#[tokio::test]
async fn a_tag_that_names_no_version_is_refused() {
    let server = TestServer::start(|_, _| {
        Reply::ok(br#"{"tag_name":"nightly","html_url":"https://github.com/x","assets":[]}"#.to_vec())
    })
    .await;
    let releases = Releases::local(server.port, Some(test_key()));

    let error = releases.latest(None).await.unwrap_err();

    assert!(matches!(error, Error::ReleaseFormat(_)), "{error}");
}

#[tokio::test]
async fn a_signed_release_is_staged_with_its_executable() {
    let release = FakeRelease::new("0.2.0", EXECUTABLE);

    let (staged, dir) = stage(release).await;

    let staged = staged.unwrap();
    assert_eq!(std::fs::read(&staged.executable).unwrap(), EXECUTABLE);
    assert!(staged.executable.starts_with(dir.path()));
    assert_eq!(staged.version, semver::Version::new(0, 2, 0));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&staged.executable)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}

#[tokio::test]
async fn a_release_without_a_signature_is_refused_before_its_archive_is_fetched() {
    let mut release = FakeRelease::new("0.2.0", EXECUTABLE);
    release.remove(SIGNATURE);
    let archive = release.archive_name();
    let server = serve(release).await;
    let releases = Releases::local(server.port, Some(test_key()));
    let published = found(&releases).await;
    let dir = tempfile::tempdir().unwrap();

    let error = releases
        .stage(&published, dir.path(), &CancellationToken::new(), &mut |_| {})
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Unsigned), "{error}");
    assert!(!server.paths().iter().any(|path| path.ends_with(&archive)));
}

#[tokio::test]
async fn checksums_signed_by_another_key_are_refused() {
    let mut release = FakeRelease::new("0.2.0", EXECUTABLE);
    release.set(SIGNATURE, vec![7; 64]);

    let (staged, _dir) = stage(release).await;

    assert!(matches!(staged, Err(Error::BadSignature)), "{staged:?}");
}

#[tokio::test]
async fn checksums_changed_after_they_were_signed_are_refused() {
    let mut release = FakeRelease::new("0.2.0", EXECUTABLE);
    let sums = String::from_utf8(release.file(SUMS).unwrap().to_vec()).unwrap();
    release.set(
        SUMS,
        format!("{sums}{}  extra.zip\n", "0".repeat(64)).into_bytes(),
    );

    let (staged, _dir) = stage(release).await;

    assert!(matches!(staged, Err(Error::BadSignature)), "{staged:?}");
}

#[tokio::test]
async fn an_archive_swapped_after_the_checksums_were_signed_is_refused_and_not_kept() {
    let mut release = FakeRelease::new("0.2.0", EXECUTABLE);
    let name = release.archive_name();
    let mut swapped = release.file(&name).unwrap().to_vec();
    swapped[0] ^= 0xff;
    release.set(&name, swapped);

    let (staged, dir) = stage(release).await;

    assert!(
        matches!(staged, Err(Error::ChecksumMismatch { .. })),
        "{staged:?}"
    );
    assert!(
        !dir.path()
            .join(updates::executable_name(std::env::consts::OS))
            .exists()
    );
}

#[tokio::test]
async fn a_build_without_the_release_key_stages_nothing() {
    let server = serve(FakeRelease::new("0.2.0", EXECUTABLE)).await;
    let releases = Releases::local(server.port, None);
    let published = found(&releases).await;
    let dir = tempfile::tempdir().unwrap();

    let error = releases
        .stage(&published, dir.path(), &CancellationToken::new(), &mut |_| {})
        .await
        .unwrap_err();

    assert!(matches!(error, Error::NoReleaseKey), "{error}");
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn an_archive_with_another_program_in_it_is_refused() {
    let release = FakeRelease::holding("0.2.0", "server", EXECUTABLE);

    let (staged, _dir) = stage(release).await;

    assert!(matches!(staged, Err(Error::Damaged(_))), "{staged:?}");
}

#[test]
fn a_checksum_is_read_as_sha256sum_writes_it_and_a_name_listed_twice_is_none() {
    let hash = "ab".repeat(32);
    let sums =
        format!("{hash}  observer-0.2.0-linux-x86_64.tar.gz\n{hash} *observer-0.2.0-windows-x86_64.zip\n");

    assert_eq!(
        key::checksum_of(&sums, "observer-0.2.0-linux-x86_64.tar.gz"),
        Some([0xab; 32])
    );
    assert_eq!(
        key::checksum_of(&sums, "observer-0.2.0-windows-x86_64.zip"),
        Some([0xab; 32])
    );
    assert_eq!(key::checksum_of(&sums, "observer-0.2.0.zip"), None);
    let twice = format!("{sums}{hash}  observer-0.2.0-linux-x86_64.tar.gz\n");
    assert_eq!(
        key::checksum_of(&twice, "observer-0.2.0-linux-x86_64.tar.gz"),
        None
    );
    assert_eq!(key::bytes32(&format!("+{}", "a".repeat(63))), None);
}

#[test]
fn a_weak_or_malformed_release_key_is_refused() {
    assert!(ReleaseKey::from_hex(&"00".repeat(32)).is_none());
    assert!(ReleaseKey::from_hex("not a key").is_none());
    assert_eq!(
        ReleaseKey::from_hex(&key::hex(&test_key().to_bytes())),
        Some(test_key())
    );
}
