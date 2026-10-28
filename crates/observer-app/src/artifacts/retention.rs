//! What a cleanup removes: every build but the ones in use and the few installed last. Pure.

use std::collections::BTreeMap;

use jiff::Timestamp;
use observer_domain::artifacts::Protection;

/// An installed build as a cleanup weighs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub build: u32,
    pub installed_at: Option<Timestamp>,
    pub valid: bool,
}

/// The builds a cleanup removes, newest number first, and those it keeps with why. Protected
/// builds do not count against `keep`; of the others, the `keep` valid builds installed last stay,
/// a tie going to the higher number, and everything else goes, broken builds included.
pub(crate) fn plan(
    builds: &[Candidate],
    protected: &BTreeMap<u32, Vec<Protection>>,
    keep: usize,
) -> (Vec<u32>, Vec<(u32, Vec<Protection>)>) {
    let mut kept: Vec<(u32, Vec<Protection>)> = Vec::new();
    let mut quota: Vec<&Candidate> = Vec::new();
    let mut removed = Vec::new();
    for candidate in builds {
        match protected.get(&candidate.build) {
            Some(reasons) if !reasons.is_empty() => kept.push((candidate.build, reasons.clone())),
            _ if candidate.valid => quota.push(candidate),
            _ => removed.push(candidate.build),
        }
    }
    quota.sort_by(|a, b| {
        b.installed_at
            .cmp(&a.installed_at)
            .then_with(|| b.build.cmp(&a.build))
    });
    for (index, candidate) in quota.into_iter().enumerate() {
        if index < keep {
            kept.push((candidate.build, Vec::new()));
        } else {
            removed.push(candidate.build);
        }
    }
    removed.sort_unstable_by_key(|build| std::cmp::Reverse(*build));
    kept.sort_by_key(|(build, _)| std::cmp::Reverse(*build));
    (removed, kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(build: u32, day: u8) -> Candidate {
        Candidate {
            build,
            installed_at: Some(format!("2026-10-{day:02}T10:00:00Z").parse().unwrap()),
            valid: true,
        }
    }

    fn protect(entries: &[(u32, Protection)]) -> BTreeMap<u32, Vec<Protection>> {
        let mut protected: BTreeMap<u32, Vec<Protection>> = BTreeMap::new();
        for (build, reason) in entries {
            protected.entry(*build).or_default().push(*reason);
        }
        protected
    }

    #[test]
    fn protected_builds_stay_outside_the_quota() {
        let builds = [
            installed(37000, 1),
            installed(37100, 2),
            installed(37200, 3),
            installed(37300, 4),
            installed(37400, 5),
        ];
        let protected = protect(&[
            (37000, Protection::LastHealthy),
            (37400, Protection::Selected),
            (37400, Protection::Running),
        ]);

        let (removed, kept) = plan(&builds, &protected, 2);

        assert_eq!(removed, [37100]);
        assert_eq!(
            kept,
            [
                (37400, vec![Protection::Selected, Protection::Running]),
                (37300, vec![]),
                (37200, vec![]),
                (37000, vec![Protection::LastHealthy]),
            ]
        );
    }

    #[test]
    fn the_last_installed_stay_and_a_tie_goes_to_the_higher_number() {
        // 36000 was installed after 37000, by an import of an older build.
        let builds = [installed(37000, 1), installed(36000, 5), installed(37500, 5)];

        let (removed, kept) = plan(&builds, &BTreeMap::new(), 2);

        assert_eq!(removed, [37000]);
        assert_eq!(kept, [(37500, vec![]), (36000, vec![])]);
    }

    #[test]
    fn broken_builds_go_whatever_the_quota() {
        let builds = [
            installed(37000, 1),
            Candidate {
                build: 37100,
                installed_at: None,
                valid: false,
            },
        ];

        let (removed, _) = plan(&builds, &BTreeMap::new(), 20);

        assert_eq!(removed, [37100]);
    }

    #[test]
    fn keeping_one_or_twenty_builds() {
        let builds: Vec<Candidate> = (1..=25)
            .map(|day| installed(37000 + u32::from(day), day))
            .collect();

        let (removed_one, kept_one) = plan(&builds, &BTreeMap::new(), 1);
        let (removed_twenty, kept_twenty) = plan(&builds, &BTreeMap::new(), 20);

        assert_eq!(kept_one, [(37025, vec![])]);
        assert_eq!(removed_one.len(), 24);
        assert_eq!(kept_twenty.len(), 20);
        assert_eq!(removed_twenty, [37005, 37004, 37003, 37002, 37001]);
    }
}
