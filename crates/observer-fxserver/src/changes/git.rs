//! The head of a resource held in git: `HEAD`, then the branch's ref file or `packed-refs`. Nothing
//! else of the repository is read, and nothing is run.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use observer_domain::changes::GitHead;

const MAX_HEAD: u64 = 4096;
const MAX_REF: u64 = 256;
const MAX_PACKED_REFS: u64 = 4 * 1024 * 1024;
const MAX_BRANCH: usize = 255;

/// The branch and commit of the repository whose `.git` folder is `git`, or `None` when it has no
/// readable head.
pub fn head(git: &Path) -> Option<GitHead> {
    let text = read(&git.join("HEAD"), MAX_HEAD)?;
    let line = text.lines().next()?.trim();
    let Some(reference) = line.strip_prefix("ref:").map(str::trim) else {
        return commit(line).map(|commit| GitHead {
            branch: None,
            commit: Some(commit),
        });
    };
    let branch = reference.strip_prefix("refs/heads/")?;
    if !safe_ref(reference) || branch.len() > MAX_BRANCH {
        return None;
    }
    let commit = read(&git.join(reference), MAX_REF)
        .and_then(|text| commit(text.trim()))
        .or_else(|| packed(git, reference));
    Some(GitHead {
        branch: Some(branch.to_owned()),
        commit,
    })
}

fn packed(git: &Path, reference: &str) -> Option<String> {
    let text = read(&git.join("packed-refs"), MAX_PACKED_REFS)?;
    text.lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(_, name)| name.trim() == reference)
        .and_then(|(hash, _)| commit(hash))
}

/// A full object name, SHA-1 or SHA-256.
fn commit(text: &str) -> Option<String> {
    let valid = matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    valid.then(|| text.to_ascii_lowercase())
}

/// A ref that names a file below `refs/` and nowhere else.
fn safe_ref(reference: &str) -> bool {
    reference.starts_with("refs/heads/")
        && reference
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && !reference.contains(['\\', ':', '\0'])
        && !reference.chars().any(char::is_control)
}

fn read(path: &Path, limit: u64) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if u64::try_from(bytes.len()).ok()? > limit {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const COMMIT: &str = "3d2f7cd0e1a2b3c4d5e6f708192a3b4c5d6e7f80";

    fn repository(head: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("refs/heads/feature")).unwrap();
        fs::write(dir.path().join("HEAD"), head).unwrap();
        dir
    }

    #[test]
    fn a_branch_reads_its_ref_file() {
        let git = repository("ref: refs/heads/feature/drops\n");
        fs::write(git.path().join("refs/heads/feature/drops"), format!("{COMMIT}\n")).unwrap();

        let head = head(git.path()).unwrap();

        assert_eq!(head.branch.as_deref(), Some("feature/drops"));
        assert_eq!(head.commit.as_deref(), Some(COMMIT));
    }

    #[test]
    fn a_packed_branch_reads_packed_refs() {
        let git = repository("ref: refs/heads/main\n");
        fs::write(
            git.path().join("packed-refs"),
            format!("# pack-refs with: peeled fully-peeled sorted\n{COMMIT} refs/heads/main\n"),
        )
        .unwrap();

        assert_eq!(head(git.path()).unwrap().commit.as_deref(), Some(COMMIT));
    }

    #[test]
    fn a_detached_head_is_a_commit_alone() {
        let git = repository(&format!("{}\n", COMMIT.to_uppercase()));

        let head = head(git.path()).unwrap();

        assert_eq!(head.branch, None);
        assert_eq!(head.commit.as_deref(), Some(COMMIT));
    }

    #[test]
    fn a_ref_that_leaves_refs_is_never_read() {
        let git = repository("ref: refs/heads/../../../../etc/passwd\n");

        assert_eq!(head(git.path()), None);
    }

    #[test]
    fn a_new_branch_without_a_commit_has_no_commit() {
        let git = repository("ref: refs/heads/main\n");

        assert_eq!(
            head(git.path()),
            Some(GitHead {
                branch: Some("main".to_owned()),
                commit: None
            })
        );
    }
}
