use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec};
use rustix::fs::inotify::{self, CreateFlags, WatchFlags};

use super::leftovers::{self, all};
use super::{ProcessTree, TREE_VARIABLE};
use crate::processes::{ChangeSender, ProcessInfo, TreeChange};

/// How often `/proc` is listed when nothing hints at a new process.
const SCAN_EVERY: Duration = Duration::from_millis(250);
/// After a loader was opened, the wait for the `execve` that opened it to finish.
const AFTER_HINT: Duration = Duration::from_millis(5);
/// Hints bring scans at most this close together: 20 a second.
const HINTED_GAP: Duration = Duration::from_millis(50);
/// A process that still has its parent's image and arguments after this long forked without
/// running anything else, and is recorded as it is.
const FORK_GRACE: Duration = Duration::from_secs(1);
/// The dynamic loaders whose opening means a program is being started on the host.
const LOADERS: [&str; 3] = [
    "/lib64/ld-linux-x86-64.so.2",
    "/lib/ld-linux-x86-64.so.2",
    "/lib/ld-musl-x86_64.so.1",
];

/// Starts the thread that lists `/proc` for the tree's new and ended processes. Without a
/// notification Linux offers an unprivileged process, a program that lives less than a scan can
/// be missed; opening a dynamic loader, which every dynamically linked start does, wakes the scan
/// early.
pub(crate) fn watch(main: u32, marker: &str, changes: ChangeSender, stop: Arc<AtomicBool>) -> io::Result<()> {
    let mut poller = Poller::new(main, marker);
    // FXServer runs through its build's own loader: its executable is that loader.
    let own_loader = fs::read_link(format!("/proc/{main}/exe")).ok();
    let hint = hint(LOADERS.iter().map(PathBuf::from).chain(own_loader));
    thread::Builder::new()
        .name("process-tree".to_owned())
        .spawn(move || {
            let mut last_scan: Option<Instant> = None;
            let mut next_scan = Instant::now();
            let mut buffer = [std::mem::MaybeUninit::uninit(); 4096];
            while !stop.load(Ordering::Relaxed) {
                let now = Instant::now();
                if now >= next_scan {
                    for change in poller.scan(now) {
                        if !changes.send(change) {
                            return;
                        }
                    }
                    last_scan = Some(now);
                    next_scan = now + SCAN_EVERY;
                    continue;
                }
                let wait = next_scan - now;
                let Some(fd) = &hint else {
                    thread::sleep(wait);
                    continue;
                };
                if hinted(fd, wait, &mut buffer) {
                    let earliest = last_scan.map_or(now, |at| at + HINTED_GAP);
                    next_scan = next_scan.min((now + AFTER_HINT).max(earliest));
                }
            }
        })?;
    Ok(())
}

/// An inotify descriptor told of every opening of the loaders that exist.
fn hint(loaders: impl Iterator<Item = PathBuf>) -> Option<OwnedFd> {
    let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK).ok()?;
    let mut watched = HashSet::new();
    for loader in loaders {
        let Ok(resolved) = fs::canonicalize(&loader) else {
            continue;
        };
        if watched.insert(resolved.clone()) {
            let _ = inotify::add_watch(&fd, &resolved, WatchFlags::OPEN);
        }
    }
    (!watched.is_empty()).then_some(fd)
}

/// Waits up to `wait` for a loader to be opened, then drains the events.
fn hinted(fd: &OwnedFd, wait: Duration, buffer: &mut [std::mem::MaybeUninit<u8>]) -> bool {
    let timeout = Timespec::try_from(wait).unwrap_or(Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    });
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    if !rustix::event::poll(&mut fds, Some(&timeout)).is_ok_and(|ready| ready > 0) {
        return false;
    }
    let mut reader = inotify::Reader::new(fd, buffer);
    while reader.next().is_ok() {}
    true
}

struct Member {
    start: u64,
    parent: u32,
    image: Option<PathBuf>,
    args: Vec<String>,
    /// Set once reported; until then the process may still be a fork about to run something else.
    reported: bool,
    seen: Instant,
}

struct Poller {
    main: u32,
    marker: Vec<u8>,
    main_image: Option<PathBuf>,
    main_args: Vec<String>,
    members: HashMap<u32, Member>,
    /// Processes outside the tree, with their start, never read again while they live.
    strangers: HashMap<u32, u64>,
}

impl Poller {
    fn new(main: u32, marker: &str) -> Self {
        Self {
            main,
            marker: format!("{TREE_VARIABLE}={marker}").into_bytes(),
            main_image: fs::read_link(format!("/proc/{main}/exe")).ok(),
            main_args: args(main),
            members: HashMap::new(),
            strangers: HashMap::new(),
        }
    }

    fn scan(&mut self, now: Instant) -> Vec<TreeChange> {
        let Ok(listed) = all() else {
            return Vec::new();
        };
        let listed: HashSet<u32> = listed.filter(|&pid| pid != self.main).collect();
        let mut changes = Vec::new();

        self.strangers.retain(|pid, _| listed.contains(pid));
        let ended: Vec<u32> = self
            .members
            .iter()
            .filter(|(pid, member)| {
                !listed.contains(pid) || leftovers::stat(**pid).is_none_or(|stat| stat.start != member.start)
            })
            .map(|(pid, _)| *pid)
            .collect();
        for pid in ended {
            if self.members.remove(&pid).is_some_and(|member| member.reported) {
                changes.push(TreeChange::Ended { pid });
            }
        }

        let mut new: Vec<(u32, leftovers::Stat)> = listed
            .iter()
            .filter(|pid| !self.members.contains_key(pid) && !self.strangers.contains_key(pid))
            .filter_map(|&pid| Some((pid, leftovers::stat(pid)?)))
            .collect();
        // A parent may be listed after its child: members are found until none is left to find.
        loop {
            let before = new.len();
            new.retain(|&(pid, stat)| {
                let inside = stat.session == self.main
                    || stat.parent == self.main
                    || self.members.contains_key(&stat.parent);
                if inside {
                    self.join(pid, stat, now);
                }
                !inside
            });
            if new.len() == before {
                break;
            }
        }
        for (pid, stat) in new {
            if marked(pid, &self.marker) {
                self.join(pid, stat, now);
            } else {
                self.strangers.insert(pid, stat.start);
            }
        }

        let (main_image, main_args) = (self.main_image.clone(), self.main_args.clone());
        let mut started: Vec<u32> = Vec::new();
        for (&pid, member) in &mut self.members {
            if member.reported {
                continue;
            }
            member.image = fs::read_link(format!("/proc/{pid}/exe")).ok();
            member.args = args(pid);
            started.push(pid);
        }
        for pid in started {
            let Some(member) = self.members.get(&pid) else {
                continue;
            };
            let (parent_image, parent_args) = if member.parent == self.main {
                (main_image.as_ref(), main_args.as_slice())
            } else {
                self.members
                    .get(&member.parent)
                    .map_or((None, &[][..]), |parent| {
                        (parent.image.as_ref(), parent.args.as_slice())
                    })
            };
            let copy =
                member.image.is_some() && member.image.as_ref() == parent_image && member.args == parent_args;
            if copy && now.saturating_duration_since(member.seen) < FORK_GRACE {
                continue;
            }
            let info = ProcessInfo {
                pid,
                parent: Some(member.parent),
                image: member.image.clone(),
                command_line: command_line(&member.args),
                args: member.args.clone(),
                created_ms: created_ms(member.start),
                stamp: Some(member.start),
            };
            if let Some(member) = self.members.get_mut(&pid) {
                member.reported = true;
            }
            changes.push(TreeChange::Started(info));
        }
        changes
    }

    fn join(&mut self, pid: u32, stat: leftovers::Stat, now: Instant) {
        self.members.insert(
            pid,
            Member {
                start: stat.start,
                parent: stat.parent,
                image: None,
                args: Vec::new(),
                reported: false,
                seen: now,
            },
        );
    }
}

impl ProcessTree {
    /// What the tree's process `pid` is, or `None` when it is gone or not in the tree: in
    /// FXServer's session, in its group, or carrying its marker.
    pub fn describe(&self, pid: u32) -> Option<ProcessInfo> {
        let stat = leftovers::stat(pid)?;
        let group = self.group();
        let inside = pid == group
            || stat.session == group
            || stat.group == group
            || marked(pid, format!("{TREE_VARIABLE}={}", self.marker()).as_bytes());
        if !inside {
            return None;
        }
        let args = args(pid);
        Some(ProcessInfo {
            pid,
            parent: Some(stat.parent),
            image: fs::read_link(format!("/proc/{pid}/exe")).ok(),
            command_line: command_line(&args),
            args,
            created_ms: created_ms(stat.start),
            stamp: Some(stat.start),
        })
    }

    /// Every process in the tree: FXServer's session and group, and whatever carries its marker
    /// after leaving them.
    pub fn all_members(&self) -> io::Result<Vec<u32>> {
        let group = self.group();
        let entry = format!("{TREE_VARIABLE}={}", self.marker());
        Ok(all()?
            .filter(|&pid| {
                leftovers::stat(pid).is_some_and(|stat| stat.session == group || stat.group == group)
                    || marked(pid, entry.as_bytes())
            })
            .collect())
    }
}

fn marked(pid: u32, entry: &[u8]) -> bool {
    fs::read(format!("/proc/{pid}/environ"))
        .is_ok_and(|environ| environ.split(|&byte| byte == 0).any(|variable| variable == entry))
}

fn args(pid: u32) -> Vec<String> {
    let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return Vec::new();
    };
    let cmdline = cmdline.strip_suffix(&[0]).unwrap_or(&cmdline);
    if cmdline.is_empty() {
        return Vec::new();
    }
    cmdline
        .split(|&byte| byte == 0)
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect()
}

/// argv joined with blanks, an argument holding a blank or a quote in double quotes.
fn command_line(args: &[String]) -> Option<String> {
    if args.is_empty() {
        return None;
    }
    let quoted: Vec<String> = args
        .iter()
        .map(|arg| {
            if arg.is_empty() || arg.contains([' ', '\t', '"']) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect();
    Some(quoted.join(" "))
}

/// The start of a process in milliseconds since the Unix epoch, from its start in clock ticks
/// after boot.
fn created_ms(start: u64) -> Option<i64> {
    static BOOT_MS: OnceLock<Option<i64>> = OnceLock::new();
    let boot = (*BOOT_MS.get_or_init(|| boot_ms(Path::new("/proc/stat"))))?;
    let per_second = rustix::param::clock_ticks_per_second().max(1);
    let after_boot = i64::try_from(start.checked_mul(1000)? / per_second).ok()?;
    boot.checked_add(after_boot)
}

fn boot_ms(stat: &Path) -> Option<i64> {
    let text = fs::read_to_string(stat).ok()?;
    let seconds: i64 = text
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    seconds.checked_mul(1000)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::{Launch, TreeChanges, spawn};

    fn shell(script: &str) -> Launch {
        Launch {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
        }
    }

    fn name(info: &ProcessInfo) -> String {
        info.image
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    async fn changes_until(
        changes: &mut TreeChanges,
        done: impl Fn(&[TreeChange]) -> bool,
    ) -> Vec<TreeChange> {
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(&seen) && Instant::now() < deadline {
            if let Ok(Some(change)) = tokio::time::timeout(Duration::from_millis(100), changes.next()).await {
                seen.push(change);
            }
        }
        seen
    }

    fn started<'a>(seen: &'a [TreeChange], program: &str) -> Option<&'a ProcessInfo> {
        seen.iter().find_map(|change| match change {
            TreeChange::Started(info) if name(info) == program => Some(info),
            _ => None,
        })
    }

    #[test]
    fn an_argument_with_a_blank_is_quoted_in_the_command_line() {
        let args = ["sh", "-c", "sleep 1", ""].map(String::from);

        assert_eq!(command_line(&args).unwrap(), r#"sh -c "sleep 1" """#);
        assert_eq!(command_line(&[]), None);
    }

    #[tokio::test]
    async fn a_child_is_reported_with_its_argv_and_parent_then_as_it_ends() {
        let mut spawned = spawn(&shell("sleep 0.6 & wait")).unwrap();

        let seen = changes_until(&mut spawned.changes, |seen| {
            started(seen, "sleep").is_some_and(|sleep| seen.contains(&TreeChange::Ended { pid: sleep.pid }))
        })
        .await;

        let sleep = started(&seen, "sleep").unwrap_or_else(|| panic!("no sleep in {seen:#?}"));
        assert_eq!(sleep.args, ["sleep", "0.6"]);
        assert_eq!(sleep.command_line.as_deref(), Some("sleep 0.6"));
        assert_eq!(sleep.parent, Some(spawned.pid));
        assert!(sleep.created_ms.is_some_and(|ms| ms > 1_700_000_000_000));
        assert!(seen.contains(&TreeChange::Ended { pid: sleep.pid }), "{seen:#?}");
        assert!(
            seen.iter()
                .all(|change| !matches!(change, TreeChange::Started(info) if info.pid == spawned.pid))
        );
    }

    #[tokio::test]
    async fn a_process_in_a_session_of_its_own_is_still_reported() {
        let mut spawned = spawn(&shell("(setsid sleep 1 </dev/null &); sleep 0.5")).unwrap();

        let seen = changes_until(&mut spawned.changes, |seen| started(seen, "sleep").is_some()).await;

        assert!(started(&seen, "sleep").is_some(), "{seen:#?}");
        spawned.tree.kill().unwrap();
    }

    #[tokio::test]
    async fn a_subshell_is_not_recorded_as_a_copy_of_its_parent() {
        let seen = changes_of("(sleep 0.4; true); sleep 0.3").await;

        assert!(
            seen.iter().all(
                |change| !matches!(change, TreeChange::Started(info) if info.args == ["/bin/sh", "-c", "(sleep 0.4; true); sleep 0.3"])
            ),
            "{seen:#?}"
        );
        assert!(started(&seen, "sleep").is_some(), "{seen:#?}");
    }

    #[tokio::test]
    async fn a_process_outside_the_tree_is_not_described() {
        let spawned = spawn(&shell("sleep 1")).unwrap();

        assert_eq!(spawned.tree.describe(std::process::id()), None);
        assert_eq!(
            spawned.tree.describe(spawned.pid).map(|info| info.pid),
            Some(spawned.pid)
        );
        assert!(spawned.tree.all_members().unwrap().contains(&spawned.pid));
        spawned.tree.kill().unwrap();
    }

    /// The changes of a run of `script`, until it has ended and the last scan after it is in.
    async fn changes_of(script: &str) -> Vec<TreeChange> {
        let mut spawned = spawn(&shell(script)).unwrap();
        let _ = spawned.exit.wait().await;
        let mut seen = Vec::new();
        while let Ok(Some(change)) = tokio::time::timeout(SCAN_EVERY * 2, spawned.changes.next()).await {
            seen.push(change);
        }
        seen
    }

    /// How many of `runs` starts of a shell living about 50 ms the poller sees.
    async fn short_shells_seen(runs: usize) -> usize {
        let mut seen = 0;
        for _ in 0..runs {
            // Well after the start, as a resource would, so that the first scan does not count.
            let changes = changes_of("sleep 0.3; sh -c 'sleep 0.05'; sleep 0.3").await;
            let inner = changes.iter().any(|change| {
                matches!(change, TreeChange::Started(info)
                    if info.command_line.as_deref().is_some_and(|line| line.contains("0.05")))
            });
            seen += usize::from(inner);
        }
        seen
    }

    #[tokio::test]
    async fn a_shell_that_lives_fifty_milliseconds_is_caught_with_the_loader_hint() {
        let seen = short_shells_seen(20).await;

        assert!(seen >= 18, "{seen} of 20");
    }
}
