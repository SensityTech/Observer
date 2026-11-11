//! Who follows a live view. The server reads positions and entities only while a panel looks at
//! them: each view counts its followers here, and each follower holds a guard that leaves with it.

use tokio::sync::watch;

/// The followers of one live view.
#[derive(Clone)]
pub(crate) struct Watchers(watch::Sender<usize>);

impl Watchers {
    pub(crate) fn new() -> Self {
        Self(watch::Sender::new(0))
    }

    pub(crate) fn join(&self) -> Watching {
        self.0.send_modify(|watchers| *watchers += 1);
        Watching {
            watchers: self.0.clone(),
        }
    }

    pub(crate) fn any(&self) -> bool {
        *self.0.borrow() > 0
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<usize> {
        self.0.subscribe()
    }
}

/// Held while a panel follows a live view: the server stops reading for it once none does.
pub struct Watching {
    watchers: watch::Sender<usize>,
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.watchers
            .send_modify(|watchers| *watchers = watchers.saturating_sub(1));
    }
}
