//! The listing runtime.fivem.net still serves for branches other than master: an HTML page with one
//! link per build, newest first, each with the date its archive was published, in UTC.

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;

use super::Source;
use crate::archive::ArchiveLocation;
use crate::error::Error;
use crate::{Branch, Platform};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedBuild {
    pub location: ArchiveLocation,
    pub published_at: Option<Timestamp>,
}

pub(crate) fn url(platform: Platform, branch: &Branch) -> String {
    format!(
        "https://runtime.fivem.net/artifacts/fivem/{}/{branch}/",
        platform.runtime_folder()
    )
}

pub(crate) fn parse(html: &str, platform: Platform, branch: &Branch) -> Result<Vec<ListedBuild>, Error> {
    // The heading proves this is the listing asked for, and not a page some redirect led to.
    let heading = format!("Builds - fivem/{}/{branch}", platform.runtime_folder());
    if !html.contains(&heading) {
        return Err(Error::FormatChanged {
            origin: Source::Branch(platform, branch.clone()),
            reason: format!("it has no {heading:?} heading"),
        });
    }
    let base = url(platform, branch);
    let mut builds = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<a ") {
        let anchor = &rest[start..];
        let (Some(open_end), Some(close)) = (anchor.find('>'), anchor.find("</a>")) else {
            break;
        };
        rest = &anchor[close + "</a>".len()..];
        if open_end > close {
            continue;
        }
        let Some(href) = attribute(&anchor[..open_end], "href") else {
            continue;
        };
        let Some(relative) = href.strip_prefix("./") else {
            continue;
        };
        let file = if relative.ends_with('/') {
            platform.archive().file_name()
        } else {
            ""
        };
        let Some(location) = ArchiveLocation::parse(&format!("{base}{relative}{file}")) else {
            continue;
        };
        builds.push(ListedBuild {
            location,
            published_at: listed_date(&anchor[open_end..close]),
        });
    }
    Ok(builds)
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let length = tag[start..].find('"')?;
    Some(&tag[start..start + length])
}

/// The first `YYYY-MM-DD HH:MM:SS` in the link's text.
fn listed_date(text: &str) -> Option<Timestamp> {
    const SHAPE: &[u8; 19] = b"0000-00-00 00:00:00";
    let window = text.as_bytes().windows(SHAPE.len()).find(|window| {
        window.iter().zip(SHAPE).all(|(byte, shape)| {
            if *shape == b'0' {
                byte.is_ascii_digit()
            } else {
                byte == shape
            }
        })
    })?;
    let civil = DateTime::strptime("%Y-%m-%d %H:%M:%S", std::str::from_utf8(window).ok()?).ok()?;
    civil.to_zoned(TimeZone::UTC).ok().map(|zoned| zoned.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: &str = include_str!("../../tests/fixtures/branch-windows.html");
    const LINUX: &str = include_str!("../../tests/fixtures/branch-linux.html");

    fn branch() -> Branch {
        "feature/network-improvements-clean".parse().unwrap()
    }

    #[test]
    fn branch_listing_rows_are_parsed_with_utc_dates() {
        let builds = parse(WINDOWS, Platform::Windows, &branch()).unwrap();

        let numbers: Vec<u32> = builds.iter().map(|listed| listed.location.build).collect();
        assert_eq!(numbers, [37862, 37576, 37264, 37240, 37133]);
        let newest = &builds[0];
        assert_eq!(newest.location.branch, branch());
        assert_eq!(
            newest.location.commit.to_string(),
            "358a6175a19018b0b8c951562fcb0ab8caf203fd"
        );
        assert_eq!(newest.published_at, Some("2026-10-25T16:56:54Z".parse().unwrap()));
        assert_eq!(
            newest.location.url().as_str(),
            "https://runtime.fivem.net/artifacts/fivem/build_server_windows/feature/network-improvements-clean/37862-358a6175a19018b0b8c951562fcb0ab8caf203fd/server.zip"
        );
    }

    #[test]
    fn the_linux_listing_links_the_linux_archives() {
        let builds = parse(LINUX, Platform::Linux, &branch()).unwrap();

        assert_eq!(builds.len(), 5);
        assert!(
            builds
                .iter()
                .all(|listed| listed.location.platform == Platform::Linux)
        );
        assert!(builds[0].location.url().as_str().ends_with("/fx.tar.xz"));
    }

    #[test]
    fn a_page_for_another_branch_is_reported_as_a_format_change() {
        let other: Branch = "feature/other".parse().unwrap();

        let error = parse(WINDOWS, Platform::Windows, &other).unwrap_err();

        assert!(
            matches!(
                error,
                Error::FormatChanged {
                    origin: Source::Branch(..),
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn the_docs_page_is_not_mistaken_for_a_listing() {
        let docs = include_str!("../../tests/fixtures/docs-page.html");

        assert!(parse(docs, Platform::Windows, &Branch::master()).is_err());
    }
}
