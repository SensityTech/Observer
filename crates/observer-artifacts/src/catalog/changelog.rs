//! The changelogs API: the recommended and latest builds with their archive links, and the support
//! policy, an end of support and an end of life for every build Cfx.re still tracks. `critical` and
//! `optional` are left out: they have been frozen at 7290 for years.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use serde::Deserialize;

use super::{PolicyDate, Source};
use crate::Platform;
use crate::archive::{ArchiveLocation, build_number};
use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Changelog {
    pub recommended: ArchiveLocation,
    pub latest: ArchiveLocation,
    pub end_of_support: BTreeMap<u32, PolicyDate>,
    pub end_of_life: BTreeMap<u32, PolicyDate>,
}

#[derive(Deserialize)]
struct Answer {
    recommended: String,
    latest: String,
    recommended_download: String,
    latest_download: String,
    support_policy: BTreeMap<String, String>,
    support_policy_eol: BTreeMap<String, String>,
}

pub(crate) fn parse(json: &str, platform: Platform) -> Result<Changelog, Error> {
    let changed = |reason: String| Error::FormatChanged {
        origin: Source::Changelog(platform),
        reason,
    };
    // The Linux API has answered `null` for hours before; that lands here too.
    let answer: Answer =
        serde_json::from_str(json).map_err(|error| changed(format!("its answer does not parse: {error}")))?;

    let download = |field: &str, build: &str, url: &str| {
        let location = ArchiveLocation::parse(url)
            .filter(|location| location.platform == platform)
            .ok_or_else(|| {
                changed(format!(
                    "{field}_download is not a {platform} archive on runtime.fivem.net"
                ))
            })?;
        if build_number(build) != Some(location.build) {
            return Err(changed(format!("{field}_download is not build {build}")));
        }
        Ok(location)
    };
    let dates = |field: &str, map: BTreeMap<String, String>| {
        map.into_iter()
            .map(|(build, date)| {
                let number = build_number(&build)
                    .ok_or_else(|| changed(format!("{field} has a key {build:?} that is not a build")))?;
                let date = policy_date(&date)
                    .ok_or_else(|| changed(format!("{field} has a date {date:?} that does not parse")))?;
                Ok((number, date))
            })
            .collect::<Result<BTreeMap<_, _>, Error>>()
    };

    Ok(Changelog {
        recommended: download("recommended", &answer.recommended, &answer.recommended_download)?,
        latest: download("latest", &answer.latest, &answer.latest_download)?,
        end_of_support: dates("support_policy", answer.support_policy)?,
        end_of_life: dates("support_policy_eol", answer.support_policy_eol)?,
    })
}

/// `2026-09-24T11:31:17Z`, an offset instead of `Z`, or `9999-12-31T23:59:59.9999999` with seven
/// decimals and no zone. Any year from 9000 on stands for never.
pub(crate) fn policy_date(text: &str) -> Option<PolicyDate> {
    let year: u16 = text.get(..4)?.parse().ok()?;
    if year >= 9000 {
        return Some(PolicyDate::Never);
    }
    if let Ok(at) = text.parse::<Timestamp>() {
        return Some(PolicyDate::At(at));
    }
    let civil: DateTime = text.parse().ok()?;
    let at = civil.to_zoned(TimeZone::UTC).ok()?.timestamp();
    Some(PolicyDate::At(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: &str = include_str!("../../tests/fixtures/changelog-win32.json");
    const LINUX: &str = include_str!("../../tests/fixtures/changelog-linux.json");

    fn at(text: &str) -> PolicyDate {
        PolicyDate::At(text.parse().unwrap())
    }

    #[test]
    fn the_api_gives_recommended_latest_and_the_policy() {
        let changelog = parse(WINDOWS, Platform::Windows).unwrap();

        assert_eq!(changelog.recommended.build, 35245);
        assert_eq!(
            changelog.recommended.commit.to_string(),
            "6efb47dff473c0e2a12fb50b08d74c0eb24a50d5"
        );
        assert_eq!(changelog.latest.build, 37872);
        assert_eq!(changelog.end_of_support[&37944], at("2026-11-10T13:39:56Z"));
        assert_eq!(changelog.end_of_support[&37872], PolicyDate::Never);
        assert_eq!(changelog.end_of_life[&37872], at("2027-01-26T12:08:14Z"));
        assert_eq!(changelog.end_of_life[&35245], PolicyDate::Never);
        assert_eq!(changelog.end_of_support.keys().next(), Some(&14164));
    }

    #[test]
    fn each_platform_has_its_own_policy() {
        let linux = parse(LINUX, Platform::Linux).unwrap();

        assert_eq!(linux.latest.platform, Platform::Linux);
        assert!(!linux.end_of_support.contains_key(&37944));
        assert!(linux.end_of_support.contains_key(&37727));
    }

    #[test]
    fn the_windows_answer_is_not_read_as_linux() {
        let error = parse(WINDOWS, Platform::Linux).unwrap_err();

        assert!(
            matches!(
                error,
                Error::FormatChanged {
                    origin: Source::Changelog(Platform::Linux),
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_null_answer_is_reported_as_a_format_change() {
        let error = parse("null", Platform::Linux).unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("the linux changelogs API changed format"),
            "{error}"
        );
    }

    #[test]
    fn a_policy_date_in_a_new_shape_is_reported_as_a_format_change() {
        let changed = WINDOWS.replace("\"2026-11-10T13:39:56Z\"", "\"10/11/2026\"");

        let error = parse(&changed, Platform::Windows).unwrap_err();

        assert!(error.to_string().contains("\"10/11/2026\""), "{error}");
    }

    #[test]
    fn policy_dates_are_read_in_utc_and_the_far_future_means_never() {
        assert_eq!(
            policy_date("2026-09-24T11:31:17Z"),
            Some(at("2026-09-24T11:31:17Z"))
        );
        assert_eq!(
            policy_date("2026-09-24T13:31:17+02:00"),
            Some(at("2026-09-24T11:31:17Z"))
        );
        assert_eq!(
            policy_date("2026-09-24T11:31:17.5"),
            Some(at("2026-09-24T11:31:17.5Z"))
        );
        assert_eq!(
            policy_date("9999-12-31T23:59:59.9999999"),
            Some(PolicyDate::Never)
        );
        assert_eq!(policy_date("tomorrow"), None);
    }
}
