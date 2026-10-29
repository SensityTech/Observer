//! What the About page shows: this build of Observer, and the third-party software in it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a binary knows of the source it was built from. `cargo xtask dist` sets the commit and
/// its date; a plain `cargo build` has neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Build {
    pub version: &'static str,
    pub commit: Option<&'static str>,
    /// `2026-10-28`, the commit's date rather than the build's, so one commit always builds the
    /// same binary.
    pub commit_date: Option<&'static str>,
}

/// `GET /api/about`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct About {
    pub version: String,
    pub commit: Option<String>,
    pub commit_date: Option<String>,
    #[ts(type = "\"windows\" | \"linux\"")]
    pub os: String,
    /// `x86_64`, as Rust names it.
    pub arch: String,
    pub profile: Profile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Profile {
    Release,
    Debug,
}

impl About {
    pub fn of(build: Build) -> Self {
        Self {
            version: build.version.to_owned(),
            commit: build.commit.map(str::to_owned),
            commit_date: build.commit_date.map(str::to_owned),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            profile: if cfg!(debug_assertions) {
                Profile::Debug
            } else {
                Profile::Release
            },
        }
    }
}

/// `GET /api/about/licences`: every package Observer is built from, with the licence texts it
/// ships. `cargo xtask licences` writes it for a target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThirdPartyLicences {
    /// The target triple the list was made for: dependencies differ between systems.
    pub target: String,
    pub packages: Vec<ThirdPartyPackage>,
    /// Each text once, by the key its files name.
    #[ts(type = "Record<string, string>")]
    pub texts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThirdPartyPackage {
    pub ecosystem: Ecosystem,
    pub name: String,
    pub version: String,
    /// The licence as the package declares it, an SPDX expression as a rule.
    pub licence: Option<String>,
    pub repository: Option<String>,
    pub files: Vec<LicenceFile>,
    /// The package ships no licence file: the texts come from the SPDX licence list.
    pub from_spdx: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Ecosystem {
    Crate,
    Npm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LicenceFile {
    /// Its path in the package, such as `LICENSE-MIT`.
    pub path: String,
    /// The key of its text in [`ThirdPartyLicences::texts`].
    pub text: String,
}
