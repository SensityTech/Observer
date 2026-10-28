use jiff::Timestamp;
use observer_domain::artifacts::{
    ArtifactNotice, ArtifactSettings, BuildPlatform, HealthyBuild, NoticeKind, NoticeReason, PendingBuild,
};
use rusqlite::types::{FromSqlError, Type};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

/// Which build a server runs, and what Observer keeps about its builds between two runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactState {
    pub selected: Option<u32>,
    pub pending: Option<PendingBuild>,
    pub last_healthy: Option<HealthyBuild>,
    pub probation: Option<OpenProbation>,
    pub highest_run: Option<u32>,
    pub notice: Option<ArtifactNotice>,
    pub legacy_build_path: Option<String>,
    /// Bumped by every save; zero before the first.
    pub version: i64,
}

/// A build on probation, and the last healthy build when it opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenProbation {
    pub build: u32,
    pub from: Option<u32>,
}

/// A downloadable build a catalog listing named, as the catalog gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenBuild {
    pub build: u32,
    pub commit: String,
    pub branch: String,
    pub url: String,
    pub published_at: Option<Timestamp>,
    pub size: Option<u64>,
}

/// A server's artifact state, or the empty one until something is saved.
pub fn state(conn: &Connection, server_id: &str) -> Result<ArtifactState, StoreError> {
    Ok(conn
        .query_row(
            "SELECT selected_build, pending_build, pending_by, pending_at, last_healthy, last_healthy_at,
                 probation_build, probation_from, highest_run, notice_kind, notice_build, notice_other,
                 notice_reason, notice_at, legacy_build_path, version
             FROM artifact_state WHERE server_id = ?1",
            [server_id],
            read_state,
        )
        .optional()?
        .unwrap_or_default())
}

/// Stores the state with its version moved on, and returns it as saved.
pub fn save_state(
    conn: &Connection,
    server_id: &str,
    state: &ArtifactState,
    now: Timestamp,
) -> Result<ArtifactState, StoreError> {
    let saved = ArtifactState {
        version: state.version + 1,
        ..state.clone()
    };
    let pending = saved.pending.as_ref();
    let notice = saved.notice.as_ref();
    conn.execute(
        "INSERT INTO artifact_state (server_id, selected_build, pending_build, pending_by, pending_at,
             last_healthy, last_healthy_at, probation_build, probation_from, highest_run, notice_kind,
             notice_build, notice_other, notice_reason, notice_at, legacy_build_path, version, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT (server_id) DO UPDATE SET selected_build = excluded.selected_build,
             pending_build = excluded.pending_build, pending_by = excluded.pending_by,
             pending_at = excluded.pending_at, last_healthy = excluded.last_healthy,
             last_healthy_at = excluded.last_healthy_at, probation_build = excluded.probation_build,
             probation_from = excluded.probation_from, highest_run = excluded.highest_run,
             notice_kind = excluded.notice_kind, notice_build = excluded.notice_build,
             notice_other = excluded.notice_other, notice_reason = excluded.notice_reason,
             notice_at = excluded.notice_at, legacy_build_path = excluded.legacy_build_path,
             version = excluded.version, updated_at = excluded.updated_at",
        params![
            server_id,
            saved.selected,
            pending.map(|pending| pending.build),
            pending.map(|pending| &pending.by),
            pending.map(|pending| Utc(pending.at)),
            saved.last_healthy.map(|healthy| healthy.build),
            saved.last_healthy.map(|healthy| Utc(healthy.since)),
            saved.probation.map(|probation| probation.build),
            saved.probation.and_then(|probation| probation.from),
            saved.highest_run,
            notice.map(|notice| notice.kind.as_str()),
            notice.map(|notice| notice.build),
            notice.and_then(|notice| notice.other),
            notice.map(|notice| notice.reason.as_str()),
            notice.map(|notice| Utc(notice.at)),
            saved.legacy_build_path,
            saved.version,
            Utc(now),
        ],
    )?;
    Ok(saved)
}

/// A server's artifact settings and their version, or `None` until they are first saved.
pub fn settings(conn: &Connection, server_id: &str) -> Result<Option<(ArtifactSettings, i64)>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT keep_builds, auto_rollback, branches, version FROM artifact_settings WHERE server_id = ?1",
            [server_id],
            |row| {
                let branches: String = row.get(2)?;
                let settings = ArtifactSettings {
                    keep_builds: row.get(0)?,
                    auto_rollback: row.get(1)?,
                    branches: serde_json::from_str(&branches).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
                    })?,
                };
                Ok((settings, row.get(3)?))
            },
        )
        .optional()?)
}

/// Stores the settings under their version, which the caller has already moved on.
pub fn save_settings(
    conn: &Connection,
    server_id: &str,
    settings: &ArtifactSettings,
    version: i64,
    now: Timestamp,
) -> Result<(), StoreError> {
    let branches = serde_json::to_string(&settings.branches)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    conn.execute(
        "INSERT INTO artifact_settings (server_id, keep_builds, auto_rollback, branches, version, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (server_id) DO UPDATE SET keep_builds = excluded.keep_builds,
             auto_rollback = excluded.auto_rollback, branches = excluded.branches,
             version = excluded.version, updated_at = excluded.updated_at",
        params![
            server_id,
            settings.keep_builds,
            settings.auto_rollback,
            branches,
            version,
            Utc(now)
        ],
    )?;
    Ok(())
}

/// Every downloadable build of `platform` a listing named, newest first.
pub fn seen(conn: &Connection, platform: BuildPlatform) -> Result<Vec<SeenBuild>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT build, commit_sha, branch, url, published_at, size FROM artifact_builds
         WHERE platform = ?1 ORDER BY build DESC",
    )?;
    let rows = statement.query_map([platform.as_str()], |row| {
        Ok(SeenBuild {
            build: row.get(0)?,
            commit: row.get(1)?,
            branch: row.get(2)?,
            url: row.get(3)?,
            published_at: row.get::<_, Option<Utc>>(4)?.map(|at| at.0),
            size: row
                .get::<_, Option<i64>>(5)?
                .and_then(|size| u64::try_from(size).ok()),
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Records the builds a listing named now. A build seen before keeps when it was first seen, and
/// takes the date and size a later listing learnt.
pub fn remember_seen(
    conn: &Connection,
    platform: BuildPlatform,
    builds: &[SeenBuild],
    now: Timestamp,
) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "INSERT INTO artifact_builds (platform, build, commit_sha, branch, url, published_at, size,
             first_seen_at, last_listed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT (platform, build) DO UPDATE SET commit_sha = excluded.commit_sha,
             branch = excluded.branch, url = excluded.url,
             published_at = coalesce(excluded.published_at, published_at),
             size = coalesce(excluded.size, size), last_listed_at = excluded.last_listed_at",
    )?;
    for build in builds {
        statement.execute(params![
            platform.as_str(),
            build.build,
            build.commit,
            build.branch,
            build.url,
            build.published_at.map(Utc),
            build.size.and_then(|size| i64::try_from(size).ok()),
            Utc(now),
        ])?;
    }
    Ok(())
}

/// The SHA-256 of a build's archive the first time it was installed.
pub fn fingerprint(
    conn: &Connection,
    platform: BuildPlatform,
    build: u32,
) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT archive_sha256 FROM artifact_fingerprints WHERE platform = ?1 AND build = ?2",
            params![platform.as_str(), build],
            |row| row.get(0),
        )
        .optional()?)
}

/// Keeps the first fingerprint of a build: a later one never replaces it.
pub fn remember_fingerprint(
    conn: &Connection,
    platform: BuildPlatform,
    build: u32,
    archive_sha256: &str,
    now: Timestamp,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO artifact_fingerprints (platform, build, archive_sha256, first_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (platform, build) DO NOTHING",
        params![platform.as_str(), build, archive_sha256, Utc(now)],
    )?;
    Ok(())
}

fn named<T>(row: &Row<'_>, index: usize, from_name: fn(&str) -> Option<T>) -> rusqlite::Result<Option<T>> {
    let Some(text) = row.get::<_, Option<String>>(index)? else {
        return Ok(None);
    };
    from_name(&text).map(Some).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Text,
            Box::new(FromSqlError::Other("unknown name".into())),
        )
    })
}

fn read_state(row: &Row<'_>) -> rusqlite::Result<ArtifactState> {
    let at = |index: usize| -> rusqlite::Result<Option<Timestamp>> {
        Ok(row.get::<_, Option<Utc>>(index)?.map(|at| at.0))
    };
    let pending = match (row.get(1)?, row.get::<_, Option<String>>(2)?, at(3)?) {
        (Some(build), Some(by), Some(at)) => Some(PendingBuild { build, by, at }),
        _ => None,
    };
    let last_healthy = match (row.get(4)?, at(5)?) {
        (Some(build), Some(since)) => Some(HealthyBuild { build, since }),
        _ => None,
    };
    let probation = row.get::<_, Option<u32>>(6)?.map(|build| OpenProbation {
        build,
        from: row.get(7).ok().flatten(),
    });
    let kind = named(row, 9, NoticeKind::from_name)?;
    let reason = named(row, 12, NoticeReason::from_name)?;
    let notice = match (kind, row.get(10)?, reason, at(13)?) {
        (Some(kind), Some(build), Some(reason), Some(at)) => Some(ArtifactNotice {
            kind,
            build,
            other: row.get(11)?,
            reason,
            at,
        }),
        _ => None,
    };
    Ok(ArtifactState {
        selected: row.get(0)?,
        pending,
        last_healthy,
        probation,
        highest_run: row.get(8)?,
        notice,
        legacy_build_path: row.get(14)?,
        version: row.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn full() -> ArtifactState {
        ArtifactState {
            selected: Some(37900),
            pending: Some(PendingBuild {
                build: 37950,
                by: "ada".to_owned(),
                at: at("2026-10-28T09:00:00.123Z"),
            }),
            last_healthy: Some(HealthyBuild {
                build: 37872,
                since: at("2026-10-28T02:28:00Z"),
            }),
            probation: Some(OpenProbation {
                build: 37900,
                from: Some(37872),
            }),
            highest_run: Some(37900),
            notice: Some(ArtifactNotice {
                kind: NoticeKind::RolledBack,
                build: 37800,
                other: Some(37872),
                reason: NoticeReason::Crashed,
                at: at("2026-10-27T22:00:00Z"),
            }),
            legacy_build_path: Some(r"C:\FXServer\build".to_owned()),
            version: 0,
        }
    }

    #[test]
    fn the_state_round_trips_and_its_version_moves_on_with_each_save() {
        let conn = memory();
        assert_eq!(state(&conn, "default").unwrap(), ArtifactState::default());

        let saved = save_state(&conn, "default", &full(), at("2026-10-28T09:00:00Z")).unwrap();
        assert_eq!(saved.version, 1);
        assert_eq!(state(&conn, "default").unwrap(), saved);

        let cleared = ArtifactState {
            pending: None,
            probation: None,
            notice: None,
            legacy_build_path: None,
            ..saved
        };
        let again = save_state(&conn, "default", &cleared, at("2026-10-28T09:01:00Z")).unwrap();
        assert_eq!(again.version, 2);
        assert_eq!(state(&conn, "default").unwrap(), again);
    }

    #[test]
    fn the_schema_refuses_half_a_pending_build_or_a_number_that_is_not_a_build() {
        let conn = memory();
        let insert = |columns: &str, values: &str| {
            conn.execute(
                &format!(
                    "INSERT INTO artifact_state (server_id, {columns}, updated_at)
                     VALUES ('default', {values}, '2026-10-28T09:00:00.000Z')"
                ),
                [],
            )
        };

        assert!(insert("pending_build", "37900").is_err());
        assert!(insert("selected_build", "12").is_err());
        assert!(
            insert(
                "notice_kind, notice_build, notice_reason, notice_at",
                "'lost', 37900, 'crashed', 'x'"
            )
            .is_err()
        );
        assert!(insert("probation_from", "37872").is_err());
        insert("selected_build", "37900").unwrap();
    }

    #[test]
    fn settings_round_trip_and_none_means_the_defaults_were_never_changed() {
        let conn = memory();
        assert_eq!(settings(&conn, "default").unwrap(), None);
        let changed = ArtifactSettings {
            keep_builds: 5,
            auto_rollback: false,
            branches: vec!["feature/network-improvements-clean".to_owned()],
        };

        save_settings(&conn, "default", &changed, 1, at("2026-10-28T09:00:00Z")).unwrap();

        assert_eq!(settings(&conn, "default").unwrap(), Some((changed, 1)));
    }

    #[test]
    fn seen_builds_keep_their_first_sighting_and_what_later_listings_learnt() {
        let conn = memory();
        let build = |published_at: Option<Timestamp>| {
            SeenBuild {
            build: 37872,
            commit: "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607".to_owned(),
            branch: "master".to_owned(),
            url: "https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/37872-0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607/server.zip".to_owned(),
            published_at,
            size: None,
        }
        };

        remember_seen(
            &conn,
            BuildPlatform::Windows,
            &[build(Some(at("2026-10-26T12:24:47Z")))],
            at("2026-10-28T09:00:00Z"),
        )
        .unwrap();
        remember_seen(
            &conn,
            BuildPlatform::Windows,
            &[build(None)],
            at("2026-10-29T09:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            seen(&conn, BuildPlatform::Windows).unwrap(),
            [build(Some(at("2026-10-26T12:24:47Z")))]
        );
        assert_eq!(seen(&conn, BuildPlatform::Linux).unwrap(), []);
        let first: String = conn
            .query_row("SELECT first_seen_at FROM artifact_builds", [], |row| row.get(0))
            .unwrap();
        assert_eq!(first, "2026-10-28T09:00:00.000Z");
    }

    #[test]
    fn the_first_fingerprint_of_a_build_is_never_replaced() {
        let conn = memory();
        let first = "a".repeat(64);

        remember_fingerprint(
            &conn,
            BuildPlatform::Linux,
            37872,
            &first,
            at("2026-10-28T09:00:00Z"),
        )
        .unwrap();
        remember_fingerprint(
            &conn,
            BuildPlatform::Linux,
            37872,
            &"b".repeat(64),
            at("2026-10-29T09:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            fingerprint(&conn, BuildPlatform::Linux, 37872).unwrap(),
            Some(first)
        );
        assert_eq!(fingerprint(&conn, BuildPlatform::Windows, 37872).unwrap(), None);
        assert!(
            remember_fingerprint(
                &conn,
                BuildPlatform::Linux,
                37873,
                "ABC",
                at("2026-10-28T09:00:00Z")
            )
            .is_err()
        );
    }
}
