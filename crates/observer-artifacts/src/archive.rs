use std::fmt;
use std::str::FromStr;

use reqwest::Url;
use serde::{Deserialize, Serialize};

const RUNTIME: &str = "https://runtime.fivem.net/artifacts/fivem/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
}

impl Platform {
    pub const fn host() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Linux }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }

    pub const fn archive(self) -> ArchiveKind {
        match self {
            Self::Windows => ArchiveKind::Zip,
            Self::Linux => ArchiveKind::TarXz,
        }
    }

    pub(crate) const fn other(self) -> Self {
        match self {
            Self::Windows => Self::Linux,
            Self::Linux => Self::Windows,
        }
    }

    /// The folder runtime.fivem.net keeps this platform's builds in. `proot` is historical: the
    /// Linux builds no longer use it.
    pub(crate) const fn runtime_folder(self) -> &'static str {
        match self {
            Self::Windows => "build_server_windows",
            Self::Linux => "build_proot_linux",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "windows" => Ok(Self::Windows),
            "linux" => Ok(Self::Linux),
            _ => Err(format!("{text:?} is not a platform; use windows or linux")),
        }
    }
}

/// The archive Observer downloads for a platform. Windows builds also come as `server.7z`, which the
/// docs page links, but `server.zip` sits next to it and needs no 7-Zip decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveKind {
    Zip,
    TarXz,
}

impl ArchiveKind {
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Zip => "server.zip",
            Self::TarXz => "fx.tar.xz",
        }
    }
}

/// A branch of runtime.fivem.net, such as `master` or `feature/network-improvements-clean`. The name
/// goes into URL paths, so each `/`-separated segment is 1 to 40 of `a-z`, `0-9`, `_` and `-`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Branch(String);

impl Branch {
    const MASTER: &str = "master";

    pub fn master() -> Self {
        Self(Self::MASTER.to_owned())
    }

    pub fn is_master(&self) -> bool {
        self.0 == Self::MASTER
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Branch {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let valid_segment = |segment: &str| {
            (1..=40).contains(&segment.len())
                && segment
                    .bytes()
                    .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
        };
        let segments = text.split('/').count();
        if (1..=4).contains(&segments) && text.split('/').all(valid_segment) {
            Ok(Self(text.to_owned()))
        } else {
            Err(format!(
                "{text:?} is not a branch name: up to 4 segments joined by `/`, each 1 to 40 of a-z, 0-9, `_` and `-`"
            ))
        }
    }
}

impl TryFrom<String> for Branch {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Branch> for String {
    fn from(branch: Branch) -> Self {
        branch.0
    }
}

impl fmt::Display for Branch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The citizenfx/fivem commit a build was made from. It names the build's folder on
/// runtime.fivem.net; it says nothing about the archive's content.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommitSha([u8; 20]);

impl FromStr for CommitSha {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("{text:?} is not a 40-character commit sha");
        let (pairs, rest) = text.as_bytes().as_chunks::<2>();
        if pairs.len() != 20 || !rest.is_empty() {
            return Err(invalid());
        }
        // Only what `Display` writes back: `from_str_radix` would also take a sign or upper case.
        let nibble = |digit: u8| match digit {
            b'0'..=b'9' => Some(digit - b'0'),
            b'a'..=b'f' => Some(digit - b'a' + 10),
            _ => None,
        };
        let mut bytes = [0; 20];
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = (nibble(*high).ok_or_else(invalid)? << 4) | nibble(*low).ok_or_else(invalid)?;
        }
        Ok(Self(bytes))
    }
}

impl TryFrom<String> for CommitSha {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<CommitSha> for String {
    fn from(sha: CommitSha) -> Self {
        sha.to_string()
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitSha({self})")
    }
}

/// A build's folder on runtime.fivem.net: `<platform folder>/<branch>/<build>-<commit>/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveLocation {
    pub platform: Platform,
    pub branch: Branch,
    pub build: u32,
    pub commit: CommitSha,
}

impl ArchiveLocation {
    /// Reads a link to any archive of a build, `server.7z` included, as the docs page gives them.
    pub fn parse(url: &str) -> Option<Self> {
        let path = url.strip_prefix(RUNTIME)?;
        let segments: Vec<&str> = path.split('/').collect();
        let [folder, branch @ .., folder_name, file] = segments.as_slice() else {
            return None;
        };
        let platform = [Platform::Windows, Platform::Linux]
            .into_iter()
            .find(|platform| platform.runtime_folder() == *folder)?;
        let known_file = match platform {
            Platform::Windows => matches!(*file, "server.zip" | "server.7z"),
            Platform::Linux => *file == "fx.tar.xz",
        };
        if !known_file {
            return None;
        }
        let (build, commit) = folder_name.split_once('-')?;
        Some(Self {
            platform,
            branch: branch.join("/").parse().ok()?,
            build: build_number(build)?,
            commit: commit.parse().ok()?,
        })
    }

    /// The archive Observer downloads from this folder.
    pub fn url(&self) -> Url {
        let text = format!(
            "{RUNTIME}{}/{}/{}-{}/{}",
            self.platform.runtime_folder(),
            self.branch,
            self.build,
            self.commit,
            self.platform.archive().file_name(),
        );
        Url::parse(&text).expect("a runtime.fivem.net URL made of validated parts parses")
    }

    pub fn on(&self, platform: Platform) -> Self {
        Self {
            platform,
            ..self.clone()
        }
    }
}

/// Build numbers have 4 to 7 digits on every source Cfx.re publishes.
pub(crate) fn build_number(text: &str) -> Option<u32> {
    if (4..=7).contains(&text.len()) && text.bytes().all(|digit| digit.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607";

    #[test]
    fn a_docs_page_link_to_server_7z_downloads_server_zip() {
        let url = format!("{RUNTIME}build_server_windows/master/37872-{SHA}/server.7z");

        let location = ArchiveLocation::parse(&url).unwrap();

        assert_eq!(location.platform, Platform::Windows);
        assert!(location.branch.is_master());
        assert_eq!(location.build, 37872);
        assert_eq!(location.commit.to_string(), SHA);
        assert_eq!(
            location.url().as_str(),
            format!("{RUNTIME}build_server_windows/master/37872-{SHA}/server.zip")
        );
    }

    #[test]
    fn a_branch_folder_keeps_its_segments() {
        let url =
            format!("{RUNTIME}build_proot_linux/feature/network-improvements-clean/37862-{SHA}/fx.tar.xz");

        let location = ArchiveLocation::parse(&url).unwrap();

        assert_eq!(location.branch.as_str(), "feature/network-improvements-clean");
        assert_eq!(location.url().as_str(), url);
    }

    #[test]
    fn links_elsewhere_or_to_the_wrong_archive_are_not_build_folders() {
        for url in [
            format!("https://downloads.cfx-services.net/prod/37872-{SHA}/server.zip"),
            format!(
                "http://runtime.fivem.net/artifacts/fivem/build_server_windows/master/37872-{SHA}/server.zip"
            ),
            format!("{RUNTIME}build_server_windows/master/37872-{SHA}/fx.tar.xz"),
            format!(
                "{RUNTIME}build_server_windows/master/37872-{}/server.zip",
                SHA.to_uppercase()
            ),
            format!("{RUNTIME}build_server_windows/master/378-{SHA}/server.zip"),
            format!("{RUNTIME}build_server_windows/../37872-{SHA}/server.zip"),
            format!("{RUNTIME}build_server_windows/37872-{SHA}/server.zip"),
        ] {
            assert_eq!(ArchiveLocation::parse(&url), None, "{url}");
        }
    }

    #[test]
    fn a_commit_sha_is_forty_lower_case_hexadecimal_digits() {
        assert_eq!(SHA.parse::<CommitSha>().unwrap().to_string(), SHA);
        let signed = "+0".repeat(20);
        for invalid in [
            signed.as_str(),
            &SHA.to_uppercase(),
            &SHA[..39],
            &format!("{SHA}0"),
            &SHA.replace('c', "g"),
        ] {
            assert!(invalid.parse::<CommitSha>().is_err(), "{invalid}");
        }
        let url = format!("{RUNTIME}build_server_windows/master/37872-{signed}/server.zip");
        assert_eq!(ArchiveLocation::parse(&url), None);
    }

    #[test]
    fn branch_names_only_take_url_safe_segments() {
        for valid in ["master", "feature/network-improvements-clean", "a_b-9"] {
            assert!(valid.parse::<Branch>().is_ok(), "{valid}");
        }
        let long = "a".repeat(41);
        for invalid in [
            "",
            "Feature",
            "a/../b",
            "a//b",
            "a/",
            "a.b",
            "a b",
            "a/b/c/d/e",
            long.as_str(),
        ] {
            assert!(invalid.parse::<Branch>().is_err(), "{invalid}");
        }
    }
}
