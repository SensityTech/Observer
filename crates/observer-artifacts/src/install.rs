use reqwest::Url;

use crate::{Branch, CommitSha, Platform};

/// A build to install, as the catalog describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    pub build: u32,
    pub commit: CommitSha,
    pub branch: Branch,
    pub platform: Platform,
    pub url: Url,
}
