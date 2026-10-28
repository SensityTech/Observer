use std::sync::{Arc, Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};

/// Where the services read the time. Tests drive a manual clock instead of waiting for expiries.
#[derive(Debug, Clone, Default)]
pub struct Clock {
    manual: Option<Arc<Mutex<Timestamp>>>,
}

impl Clock {
    pub fn system() -> Self {
        Self::default()
    }

    /// A clock that stands still at `start` until it is advanced.
    pub fn manual(start: Timestamp) -> Self {
        Self {
            manual: Some(Arc::new(Mutex::new(start))),
        }
    }

    pub fn now(&self) -> Timestamp {
        match &self.manual {
            Some(now) => *now.lock().unwrap_or_else(PoisonError::into_inner),
            None => Timestamp::now(),
        }
    }

    /// Moves a manual clock forward. The system clock moves on its own and ignores this.
    pub fn advance(&self, by: SignedDuration) {
        if let Some(now) = &self.manual {
            let mut now = now.lock().unwrap_or_else(PoisonError::into_inner);
            *now += by;
        }
    }
}
