use std::fs;
use std::io;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{Pid, Signal};

const KILL_TIMEOUT: Duration = Duration::from_secs(5);

/// Remembers the running server's process group on disk. Without systemd nothing kills that group
/// when Observer itself is killed (`PR_SET_PDEATHSIG` only reaches FXServer, not its children), so
/// the next Observer sweeps it before it starts a server of its own.
pub struct PidFile {
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    /// No pid file: the previous run stopped its server.
    Clean,
    /// The recorded group is gone, or its id now belongs to someone else.
    Gone,
    /// The recorded group was still running and is now killed.
    Killed { group: u32, processes: usize },
}

impl PidFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Records the server just spawned, which leads its own process group.
    pub fn record(&self, pid: u32) -> io::Result<()> {
        let leader =
            stat(pid).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the process is gone"))?;
        let partial = self.path.with_extension("pid.partial");
        fs::write(&partial, format!("{pid} {} {}\n", leader.group, leader.start))?;
        fs::rename(&partial, &self.path)
    }

    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Kills what a previous Observer left running, if the recorded group is still the same one,
    /// then forgets it. Blocks for up to a few seconds.
    pub fn sweep(&self) -> io::Result<Sweep> {
        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Sweep::Clean),
            Err(error) => return Err(error),
        };
        let sweep = match Record::parse(&content) {
            Some(record) => kill_leftovers(&record)?,
            None => Sweep::Gone,
        };
        self.clear()?;
        Ok(sweep)
    }
}

struct Record {
    pid: u32,
    group: u32,
    start: u64,
}

impl Record {
    fn parse(content: &str) -> Option<Self> {
        let mut fields = content.split_whitespace().map(str::parse::<u64>);
        let pid = u32::try_from(fields.next()?.ok()?).ok()?;
        let group = u32::try_from(fields.next()?.ok()?).ok()?;
        let start = fields.next()?.ok()?;
        Some(Self { pid, group, start })
    }
}

fn kill_leftovers(record: &Record) -> io::Result<Sweep> {
    if stat(record.pid).is_some_and(|leader| leader.start != record.start || leader.group != record.group) {
        // The pid was reused, which the kernel only does once nothing uses the number as a group id:
        // the old group is gone.
        return Ok(Sweep::Gone);
    }
    // A group id stays reserved while any member lives, even after its leader died. Members of the
    // recorded group were started after its leader and share the session the leader created.
    let members = processes()?
        .filter_map(stat)
        .filter(|process| {
            process.group == record.group && process.session == record.group && process.start >= record.start
        })
        .count();
    let Some(group) = i32::try_from(record.group).ok().and_then(Pid::from_raw) else {
        return Ok(Sweep::Gone);
    };
    if members == 0 {
        return Ok(Sweep::Gone);
    }

    match rustix::process::kill_process_group(group, Signal::KILL) {
        Ok(()) | Err(Errno::SRCH) => {}
        Err(error) => return Err(error.into()),
    }
    let deadline = Instant::now() + KILL_TIMEOUT;
    while rustix::process::test_kill_process_group(group).is_ok() {
        if Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("process group {} survived SIGKILL", record.group),
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(Sweep::Killed {
        group: record.group,
        processes: members,
    })
}

fn processes() -> io::Result<impl Iterator<Item = u32>> {
    Ok(fs::read_dir("/proc")?.filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stat {
    pub group: u32,
    pub session: u32,
    /// Clock ticks after boot: with the pid, it tells a process from a later one that reused the pid.
    pub start: u64,
}

pub(crate) fn stat(pid: u32) -> Option<Stat> {
    parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Reads `/proc/<pid>/stat`. The command name may hold spaces and parentheses, so the fields are
/// counted from the last `)`.
fn parse_stat(line: &str) -> Option<Stat> {
    let mut fields = line.get(line.rfind(')')? + 1..)?.split_whitespace();
    let group = fields.nth(2)?.parse().ok()?;
    let session = fields.next()?.parse().ok()?;
    let start = fields.nth(15)?.parse().ok()?;
    Some(Stat {
        group,
        session,
        start,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{Launch, spawn};

    fn sleeper(script: &str) -> Launch {
        Launch {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
        }
    }

    #[test]
    fn stat_fields_are_counted_from_the_end_of_the_command_name() {
        let line =
            "4242 (fx (server) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 12 0 98765 1000 77\n";

        assert_eq!(
            parse_stat(line),
            Some(Stat {
                group: 4242,
                session: 4242,
                start: 98765
            })
        );
        assert_eq!(parse_stat("4242 (cut short) S 1"), None);
    }

    #[test]
    fn without_a_pid_file_there_is_nothing_to_sweep() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            PidFile::new(dir.path().join("fxserver.pid")).sweep().unwrap(),
            Sweep::Clean
        );
    }

    #[tokio::test]
    async fn a_recorded_group_still_running_is_killed_with_its_children() {
        let dir = tempfile::tempdir().unwrap();
        let file = PidFile::new(dir.path().join("fxserver.pid"));
        let mut spawned = spawn(&sleeper("sleep 30 & echo ready; wait")).unwrap();
        assert!(spawned.stdout.next().await.is_some());
        file.record(spawned.pid).unwrap();

        let sweep = file.sweep().unwrap();

        assert_eq!(
            sweep,
            Sweep::Killed {
                group: spawned.pid,
                processes: 2
            }
        );
        assert!(spawned.tree.is_empty().unwrap());
        assert!(!dir.path().join("fxserver.pid").exists());
    }

    #[tokio::test]
    async fn a_record_whose_start_time_differs_belongs_to_someone_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fxserver.pid");
        let mut spawned = spawn(&sleeper("echo ready; exec sleep 30")).unwrap();
        assert!(spawned.stdout.next().await.is_some());
        let leader = stat(spawned.pid).unwrap();
        fs::write(
            &path,
            format!("{} {} {}\n", spawned.pid, leader.group, leader.start + 1),
        )
        .unwrap();

        assert_eq!(PidFile::new(&path).sweep().unwrap(), Sweep::Gone);
        assert!(!spawned.tree.is_empty().unwrap());
        assert!(!path.exists());
    }

    #[test]
    fn an_unreadable_record_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fxserver.pid");
        fs::write(&path, "not a record").unwrap();

        assert_eq!(PidFile::new(&path).sweep().unwrap(), Sweep::Gone);
        assert!(!path.exists());
    }
}
