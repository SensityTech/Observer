use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use observer_domain::error::{ApiError, ErrorCode};
use tokio::sync::Semaphore;

use crate::{Error, secrets};

pub(crate) const MIN_CHARS: usize = 12;
pub(crate) const MAX_BYTES: usize = 1024;

/// Two hashes at 64 MiB each bound the memory a burst of logins can take.
const RUNNING: usize = 2;
const QUEUED: usize = 16;

/// What one Argon2id hash costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordCost {
    pub(crate) memory_kib: u32,
    pub(crate) passes: u32,
}

impl PasswordCost {
    /// 64 MiB and three passes, one lane.
    pub const STANDARD: Self = Self {
        memory_kib: 64 * 1024,
        passes: 3,
    };
    /// The cheapest cost Argon2 accepts. It protects nothing: tests only.
    pub const TEST: Self = Self {
        memory_kib: 8,
        passes: 1,
    };
}

/// Hashes and verifies passwords off the async runtime, a few at a time.
pub(crate) struct Passwords {
    params: Params,
    slots: Arc<Semaphore>,
    pending: Arc<AtomicUsize>,
    /// Checked when the username is unknown, so that it takes as long as a known one.
    dummy: String,
}

pub(crate) struct Verdict {
    pub matches: bool,
    /// The hash was made with other parameters and should be replaced on success.
    pub outdated: bool,
}

impl Passwords {
    pub(crate) async fn new(cost: PasswordCost) -> Result<Self, Error> {
        let params = Params::new(cost.memory_kib, cost.passes, 1, None).map_err(hashing)?;
        let mut passwords = Self {
            params,
            slots: Arc::new(Semaphore::new(RUNNING)),
            pending: Arc::default(),
            dummy: String::new(),
        };
        passwords.dummy = passwords.hash(secrets::temporary_password()?).await?;
        Ok(passwords)
    }

    pub(crate) async fn hash(&self, password: String) -> Result<String, Error> {
        let params = self.params.clone();
        self.run(move || {
            argon2(params)
                .hash_password(password.as_bytes())
                .map(|hash| hash.to_string())
                .map_err(hashing)
        })
        .await?
    }

    /// Checks `password` against `hash`, or against the dummy hash when there is no account: that
    /// costs the same and never matches.
    pub(crate) async fn verify(&self, password: String, hash: Option<String>) -> Result<Verdict, Error> {
        let known = hash.is_some();
        let hash = hash.unwrap_or_else(|| self.dummy.clone());
        let params = self.params.clone();
        self.run(move || {
            let parsed = PasswordHash::new(&hash).map_err(hashing)?;
            let matches = argon2(params.clone())
                .verify_password(password.as_bytes(), &parsed)
                .is_ok();
            let outdated = parsed.algorithm != Algorithm::Argon2id.ident()
                || Params::try_from(&parsed).map_or(true, |stored| {
                    (stored.m_cost(), stored.t_cost(), stored.p_cost())
                        != (params.m_cost(), params.t_cost(), params.p_cost())
                });
            Ok(Verdict {
                matches: known && matches,
                outdated,
            })
        })
        .await?
    }

    #[cfg(test)]
    pub(crate) fn dummy(&self) -> &str {
        &self.dummy
    }

    /// Stops every hash from starting until the permit drops, so a test can act while a check waits.
    #[cfg(test)]
    pub(crate) async fn hold(&self) -> tokio::sync::OwnedSemaphorePermit {
        Arc::clone(&self.slots)
            .acquire_many_owned(u32::try_from(RUNNING).unwrap())
            .await
            .unwrap()
    }

    /// How many hashes are queued or running.
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    /// Runs a hash on the blocking pool once a slot is free, or refuses when too many wait already.
    async fn run<R, F>(&self, job: F) -> Result<R, Error>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        let pending = Pending::enter(&self.pending)?;
        let slot = Arc::clone(&self.slots).acquire_owned().await.map_err(hashing)?;
        // Both are held by the job, so an abandoned request still counts until its hash ends.
        tokio::task::spawn_blocking(move || {
            let _held = (pending, slot);
            job()
        })
        .await
        .map_err(hashing)
    }
}

fn argon2(params: Params) -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hashing(error: impl std::fmt::Display) -> Error {
    Error::Hashing(error.to_string())
}

/// One request counted against the queue until it is dropped.
struct Pending(Arc<AtomicUsize>);

impl Pending {
    fn enter(counter: &Arc<AtomicUsize>) -> Result<Self, Error> {
        let ahead = counter.fetch_add(1, Ordering::AcqRel);
        let pending = Self(Arc::clone(counter));
        if ahead < RUNNING + QUEUED {
            Ok(pending)
        } else {
            Err(ApiError::new(ErrorCode::Busy).with("retryAfter", 1).into())
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Whether a password someone chooses fits the policy: no composition rules, only length.
pub(crate) fn check_new(password: &str) -> Result<(), Error> {
    if password.chars().count() < MIN_CHARS {
        Err(ApiError::new(ErrorCode::PasswordTooShort)
            .with("min", MIN_CHARS)
            .into())
    } else if password.len() > MAX_BYTES {
        Err(ApiError::new(ErrorCode::PasswordTooLong)
            .with("max", MAX_BYTES)
            .into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_password_verifies_against_its_own_hash_only() {
        let passwords = Passwords::new(PasswordCost::TEST).await.unwrap();
        let hash = passwords.hash("correct horse battery".to_owned()).await.unwrap();

        let right = passwords
            .verify("correct horse battery".to_owned(), Some(hash.clone()))
            .await
            .unwrap();
        let wrong = passwords
            .verify("correct horse battery!".to_owned(), Some(hash.clone()))
            .await
            .unwrap();

        assert!(hash.starts_with("$argon2id$v=19$m=8,t=1,p=1$"));
        assert!(right.matches && !right.outdated);
        assert!(!wrong.matches);
    }

    #[tokio::test]
    async fn an_unknown_account_is_checked_against_a_dummy_hash_of_the_same_cost() {
        let passwords = Passwords::new(PasswordCost::TEST).await.unwrap();

        let verdict = passwords
            .verify("anything at all".to_owned(), None)
            .await
            .unwrap();

        assert!(!verdict.matches);
        assert!(!verdict.outdated, "the dummy must cost what a real hash costs");
        assert!(passwords.dummy().starts_with("$argon2id$v=19$m=8,t=1,p=1$"));
    }

    #[tokio::test]
    async fn a_hash_made_with_other_parameters_is_outdated() {
        let old = Passwords::new(PasswordCost::TEST).await.unwrap();
        let current = Passwords::new(PasswordCost {
            memory_kib: 16,
            passes: 2,
        })
        .await
        .unwrap();
        let hash = old.hash("correct horse battery".to_owned()).await.unwrap();

        let verdict = current
            .verify("correct horse battery".to_owned(), Some(hash))
            .await
            .unwrap();

        assert!(verdict.matches && verdict.outdated);
    }

    #[test]
    fn new_passwords_are_judged_by_length_alone() {
        assert_eq!(
            check_new("elevenchars").unwrap_err().code(),
            Some(ErrorCode::PasswordTooShort)
        );
        // Twelve characters, though more than twelve bytes.
        check_new("ééééééééééé1").unwrap();
        check_new("aaaaaaaaaaaa").unwrap();
        assert_eq!(
            check_new(&"a".repeat(1025)).unwrap_err().code(),
            Some(ErrorCode::PasswordTooLong)
        );
    }
}
