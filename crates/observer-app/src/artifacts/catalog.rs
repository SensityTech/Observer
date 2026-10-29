//! The catalog as the panel reads it: one fetch at a time, answered from memory while it is
//! fresh, and the downloadable builds it lists remembered as seen.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use jiff::Timestamp;
use observer_artifacts::catalog::{Availability, Catalog, CatalogBuild, Listing, SeenBuild, Support};
use observer_artifacts::{Branch, Platform};
use observer_domain::artifacts::BuildPlatform;
use observer_domain::error::ErrorCode;
use observer_store::Database;
use observer_store::artifacts as stored;
use tokio::sync::Mutex as AsyncMutex;
use tracing::warn;

use super::ArtifactTimings;
use crate::{Clock, Error};

pub(crate) struct CatalogCache {
    catalog: Catalog,
    db: Database,
    clock: Clock,
    timings: ArtifactTimings,
    /// Held during a fetch, so that requests arriving meanwhile get its answer.
    fetching: AsyncMutex<()>,
    latest: Mutex<Option<Arc<Fetched>>>,
}

pub(crate) struct Fetched {
    pub listing: Listing,
    pub at: Instant,
    /// When it was fetched, as the panel shows it.
    pub taken_at: Timestamp,
    /// The test branches it was fetched with.
    pub branches: Vec<String>,
}

impl Fetched {
    /// Past its end of life by the support policy, when the catalog knows the build.
    pub fn end_of_life(&self, build: u32, now: Timestamp) -> bool {
        self.listing
            .find(build)
            .is_some_and(|entry| entry.support(now) == Support::EndOfLife)
    }

    pub fn reinstallable(&self, build: u32) -> bool {
        self.listing
            .find(build)
            .is_some_and(|entry| matches!(entry.availability, Availability::Downloadable { .. }))
    }
}

impl CatalogCache {
    pub fn new(catalog: Catalog, db: Database, clock: Clock, timings: ArtifactTimings) -> Self {
        Self {
            catalog,
            db,
            clock,
            timings,
            fetching: AsyncMutex::new(()),
            latest: Mutex::new(None),
        }
    }

    /// The last listing fetched, whatever its age.
    pub fn latest(&self) -> Option<Arc<Fetched>> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// A listing younger than ten minutes for these branches, or a new one. `refresh` fetches
    /// again once the last listing is thirty seconds old.
    pub async fn listing(&self, branches: &[String], refresh: bool) -> Result<Arc<Fetched>, Error> {
        let _one = self.fetching.lock().await;
        if let Some(current) = self.latest().filter(|current| current.branches == branches) {
            let age = current.at.elapsed();
            let limit = if refresh {
                self.timings.refresh_after
            } else {
                self.timings.catalog_fresh_for
            };
            if age < limit {
                return Ok(current);
            }
        }
        let platform = BuildPlatform::host();
        let rows = self.db.read(move |conn| stored::seen(conn, platform)).await?;
        let seen: Vec<SeenBuild> = rows.iter().filter_map(parse_seen).collect();
        let parsed: Vec<Branch> = branches.iter().filter_map(|name| name.parse().ok()).collect();
        let listing = self
            .catalog
            .list(Platform::host(), &parsed, &seen)
            .await
            .map_err(|error| {
                warn!(%error, "the catalog has no source to list builds from");
                Error::from(ErrorCode::CatalogUnavailable)
            })?;
        let now = self.clock.now();
        let listed: Vec<stored::SeenBuild> = listing
            .builds
            .iter()
            .filter(|entry| entry.listed)
            .filter_map(CatalogBuild::seen)
            .map(|seen| stored::SeenBuild {
                build: seen.build,
                commit: seen.commit.to_string(),
                branch: seen.branch.to_string(),
                url: seen.url.to_string(),
                published_at: seen.published_at,
                size: seen.size,
            })
            .collect();
        if let Err(error) = self
            .db
            .write(move |tx| stored::remember_seen(tx, platform, &listed, now))
            .await
        {
            warn!(%error, "cannot remember the builds the catalog listed");
        }
        let fetched = Arc::new(Fetched {
            listing,
            at: Instant::now(),
            taken_at: now,
            branches: branches.to_vec(),
        });
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&fetched));
        Ok(fetched)
    }
}

/// A stored build, as the catalog takes it; one the store kept in a shape this Observer refuses
/// is left out.
fn parse_seen(row: &stored::SeenBuild) -> Option<SeenBuild> {
    Some(SeenBuild {
        build: row.build,
        commit: row.commit.parse().ok()?,
        branch: row.branch.parse().ok()?,
        url: row.url.parse().ok()?,
        published_at: row.published_at,
        size: row.size,
    })
}
