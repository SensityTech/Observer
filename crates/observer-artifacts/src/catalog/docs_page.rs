//! The Cfx.re docs download page, a Next.js page whose `__NEXT_DATA__` script carries the build
//! lists. It names the recommended build and the latest master builds of both platforms at once,
//! without dates.

use serde::Deserialize;

use super::Source;
use crate::Platform;
use crate::archive::ArchiveLocation;
use crate::error::Error;

/// The master builds the page lists for one platform, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocsLists {
    pub recommended: Vec<ArchiveLocation>,
    pub latest: Vec<ArchiveLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocsPage {
    windows: DocsLists,
    linux: DocsLists,
}

impl DocsPage {
    pub fn lists(&self, platform: Platform) -> &DocsLists {
        match platform {
            Platform::Windows => &self.windows,
            Platform::Linux => &self.linux,
        }
    }
}

#[derive(Deserialize)]
struct NextData {
    props: Props,
}

#[derive(Deserialize)]
struct Props {
    #[serde(rename = "pageProps")]
    page_props: PageProps,
}

/// `enhanced` sits next to `legacy`: FiveM for GTAV Enhanced is another server, which Observer does
/// not run.
#[derive(Deserialize)]
struct PageProps {
    legacy: Legacy,
}

#[derive(Deserialize)]
struct Legacy {
    recommended: PerPlatform,
    latest: PerPlatform,
}

#[derive(Deserialize)]
struct PerPlatform {
    windows: Vec<Link>,
    linux: Vec<Link>,
}

#[derive(Deserialize)]
struct Link {
    #[serde(rename = "downloadURL")]
    download_url: String,
}

pub(crate) fn parse(html: &str) -> Result<DocsPage, Error> {
    let changed = |reason: String| Error::FormatChanged {
        origin: Source::DocsPage,
        reason,
    };
    let script = html
        .find(r#"id="__NEXT_DATA__""#)
        .map(|at| &html[at..])
        .and_then(|rest| Some(&rest[rest.find('>')? + 1..]))
        .and_then(|rest| Some(&rest[..rest.find("</script>")?]))
        .ok_or_else(|| changed("it has no __NEXT_DATA__ script".to_owned()))?;
    let data: NextData = serde_json::from_str(script)
        .map_err(|error| changed(format!("its build lists do not parse: {error}")))?;
    let legacy = data.props.page_props.legacy;

    let page = DocsPage {
        windows: DocsLists {
            recommended: locations(&legacy.recommended.windows, Platform::Windows),
            latest: locations(&legacy.latest.windows, Platform::Windows),
        },
        linux: DocsLists {
            recommended: locations(&legacy.recommended.linux, Platform::Linux),
            latest: locations(&legacy.latest.linux, Platform::Linux),
        },
    };
    for platform in [Platform::Windows, Platform::Linux] {
        if page.lists(platform).latest.is_empty() {
            return Err(changed(format!(
                "it links no {platform} build on runtime.fivem.net"
            )));
        }
    }
    Ok(page)
}

/// Links elsewhere than a master folder of this platform on runtime.fivem.net are left out.
fn locations(links: &[Link], platform: Platform) -> Vec<ArchiveLocation> {
    links
        .iter()
        .filter_map(|link| ArchiveLocation::parse(&link.download_url))
        .filter(|location| location.platform == platform && location.branch.is_master())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../../tests/fixtures/docs-page.html");
    const REDESIGNED: &str = include_str!("../../tests/fixtures/docs-page-redesigned.html");

    fn builds(locations: &[ArchiveLocation]) -> Vec<u32> {
        locations.iter().map(|location| location.build).collect()
    }

    #[test]
    fn the_docs_page_lists_recommended_and_latest_per_platform() {
        let page = parse(PAGE).unwrap();

        let windows = page.lists(Platform::Windows);
        assert_eq!(builds(&windows.recommended), [35245]);
        assert_eq!(
            windows.recommended[0].commit.to_string(),
            "6efb47dff473c0e2a12fb50b08d74c0eb24a50d5"
        );
        assert_eq!(windows.latest.len(), 32);
        assert_eq!(windows.latest[0].build, 37872);
        assert_eq!(windows.latest[31].build, 34629);
        assert!(builds(&windows.latest).contains(&36727));

        let linux = page.lists(Platform::Linux);
        assert_eq!(builds(&linux.recommended), [35245]);
        assert_eq!(linux.latest.len(), 32);
        assert!(!builds(&linux.latest).contains(&36727));
        assert_eq!(linux.latest[31].build, 33864);
        assert!(
            linux
                .latest
                .iter()
                .all(|location| location.platform == Platform::Linux)
        );
    }

    #[test]
    fn enhanced_builds_are_left_out() {
        let page = parse(PAGE).unwrap();

        for platform in [Platform::Windows, Platform::Linux] {
            assert!(!builds(&page.lists(platform).latest).contains(&170));
        }
    }

    #[test]
    fn a_redesigned_page_is_reported_as_a_format_change() {
        let error = parse(REDESIGNED).unwrap_err();

        assert!(
            matches!(&error, Error::FormatChanged { origin: Source::DocsPage, reason } if reason.contains("missing field `legacy`")),
            "{error}"
        );
    }

    #[test]
    fn a_page_without_next_data_is_reported_as_a_format_change() {
        let maintenance = "<!DOCTYPE html><html><body><h1>Down for maintenance</h1></body></html>";

        let error = parse(maintenance).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the docs download page changed format: it has no __NEXT_DATA__ script"
        );
    }

    #[test]
    fn a_page_whose_links_moved_to_another_host_is_reported_as_a_format_change() {
        let moved = PAGE.replace(
            "https://runtime.fivem.net/",
            "https://downloads.cfx-services.net/",
        );

        let error = parse(&moved).unwrap_err();

        assert!(matches!(error, Error::FormatChanged { .. }), "{error}");
    }
}
