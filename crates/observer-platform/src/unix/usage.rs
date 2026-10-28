use std::fs;
use std::io;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use super::ProcessTree;
use crate::usage::{CpuTimes, HostUsage, Interface, Member, Memory, Space, TreeUsage, Usage};

impl ProcessTree {
    /// What the group leader and each other member of its group use now. A process that exits
    /// meanwhile is left out, and so is one that left the group.
    pub fn usage(&self) -> io::Result<TreeUsage> {
        let leader = self.group();
        let mut main = None;
        let mut children = Vec::new();
        for pid in self.members()? {
            let Ok(usage) = measure(pid) else {
                continue;
            };
            if pid == leader {
                main = Some(usage);
                continue;
            }
            let name = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
            children.push(Member {
                pid,
                name: name.trim_end_matches('\n').to_owned(),
                usage,
            });
        }
        Ok(TreeUsage { main, children })
    }
}

pub(crate) fn own_usage() -> io::Result<Usage> {
    measure(std::process::id())
}

pub(crate) fn host_usage() -> HostUsage {
    let read = |path: &str| fs::read_to_string(path).ok();
    let stat = read("/proc/stat");
    HostUsage {
        cpu: stat.as_deref().and_then(parse_cpu),
        processors: stat.as_deref().and_then(parse_processors),
        memory: read("/proc/meminfo").as_deref().and_then(parse_meminfo),
        interfaces: read("/proc/net/dev").map(|text| {
            physical(parse_net_dev(&text), |name| {
                Path::new("/sys/class/net").join(name).join("device").exists()
            })
        }),
    }
}

pub(crate) fn volume_space(dir: &Path) -> io::Result<Space> {
    let stat = rustix::fs::statvfs(dir)?;
    Ok(Space {
        available: stat.f_bavail.saturating_mul(stat.f_frsize),
        total: stat.f_blocks.saturating_mul(stat.f_frsize),
    })
}

fn measure(pid: u32) -> io::Result<Usage> {
    let read = |file: &str| fs::read_to_string(format!("/proc/{pid}/{file}"));
    let invalid = |file: &str| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("/proc/{pid}/{file} is unreadable"),
        )
    };
    let stat = parse_stat(&read("stat")?).ok_or_else(|| invalid("stat"))?;
    // A zombie keeps its stat but has no memory left to report.
    let memory = parse_status(&read("status")?).ok_or_else(|| invalid("status"))?;
    // Only the owner of a process, or root, may read its I/O counters and descriptors.
    let io = read("io").ok().as_deref().and_then(parse_io);
    let handles = fs::read_dir(format!("/proc/{pid}/fd"))
        .ok()
        .map(|entries| u32::try_from(entries.count()).unwrap_or(u32::MAX));
    Ok(Usage {
        cpu: ticks(stat.cpu_ticks),
        working_set: memory.resident,
        private: memory.private,
        threads: Some(stat.threads),
        handles,
        read: io.map(|io| io.0),
        written: io.map(|io| io.1),
        started: stat.start,
    })
}

fn ticks(count: u64) -> Duration {
    static PER_SECOND: OnceLock<u64> = OnceLock::new();
    let per_second = *PER_SECOND.get_or_init(|| rustix::param::clock_ticks_per_second().max(1));
    Duration::from_secs(count / per_second)
        + Duration::from_nanos((count % per_second) * 1_000_000_000 / per_second)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stat {
    /// `utime` plus `stime`, in clock ticks.
    cpu_ticks: u64,
    threads: u32,
    /// Clock ticks after boot.
    start: u64,
}

/// `/proc/<pid>/stat`, counted from the last `)` since the command name may hold spaces and
/// parentheses: the state is field 3, `utime` 14, `stime` 15, `num_threads` 20, `starttime` 22.
fn parse_stat(text: &str) -> Option<Stat> {
    let fields: Vec<&str> = text.get(text.rfind(')')? + 1..)?.split_whitespace().collect();
    let field = |number: usize| fields.get(number - 3)?.parse::<u64>().ok();
    Some(Stat {
        cpu_ticks: field(14)?.checked_add(field(15)?)?,
        threads: u32::try_from(field(20)?).ok()?,
        start: field(22)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Resident {
    resident: u64,
    private: u64,
}

/// `/proc/<pid>/status`: `VmRSS`, and `RssAnon` with `VmSwap` for the memory no other process
/// shares, all in kibibytes.
fn parse_status(text: &str) -> Option<Resident> {
    let resident = kibibytes(text, "VmRSS:")?;
    let private = kibibytes(text, "RssAnon:")?.checked_add(kibibytes(text, "VmSwap:")?)?;
    Some(Resident { resident, private })
}

/// `/proc/<pid>/io`: `rchar` and `wchar`.
fn parse_io(text: &str) -> Option<(u64, u64)> {
    let counter = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.trim().parse::<u64>().ok())
    };
    Some((counter("rchar:")?, counter("wchar:")?))
}

/// The first line of `/proc/stat`: `cpu user nice system idle iowait irq softirq steal guest
/// guest_nice`. Guest time is already counted in user time, so the total stops at steal; waiting
/// for I/O is idle time.
fn parse_cpu(text: &str) -> Option<CpuTimes> {
    let line = text.lines().next()?.strip_prefix("cpu ")?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [user, nice, system, idle, iowait, irq, softirq, steal] = fields.try_into().ok()?;
    let total = user + nice + system + idle + iowait + irq + softirq + steal;
    Some(CpuTimes {
        busy: total - idle - iowait,
        total,
    })
}

/// The `cpuN` lines of `/proc/stat`, one per online processor. The CPUs a container is limited
/// to change neither them nor the first line.
fn parse_processors(text: &str) -> Option<u32> {
    let count = text
        .lines()
        .filter_map(|line| line.split_whitespace().next()?.strip_prefix("cpu"))
        .filter(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        .count();
    u32::try_from(count).ok().filter(|&count| count > 0)
}

fn parse_meminfo(text: &str) -> Option<Memory> {
    let total = kibibytes(text, "MemTotal:")?;
    let available = kibibytes(text, "MemAvailable:")?;
    Some(Memory {
        used: total.saturating_sub(available),
        total,
    })
}

/// `/proc/net/dev`: two header lines, then `name: rx_bytes … (8 receive columns) tx_bytes …`.
fn parse_net_dev(text: &str) -> Vec<Interface> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, counters) = line.split_once(':')?;
            let mut counters = counters.split_whitespace();
            let received = counters.next()?.parse().ok()?;
            let sent = counters.nth(7)?.parse().ok()?;
            Some(Interface {
                key: name.trim().to_owned(),
                received,
                sent,
            })
        })
        .collect()
}

/// The adapters backed by a device. In a container none is, and then every one but the loopback
/// counts.
fn physical(interfaces: Vec<Interface>, has_device: impl Fn(&str) -> bool) -> Vec<Interface> {
    let backed: Vec<Interface> = interfaces
        .iter()
        .filter(|interface| has_device(&interface.key))
        .cloned()
        .collect();
    if backed.is_empty() {
        interfaces
            .into_iter()
            .filter(|interface| interface.key != "lo")
            .collect()
    } else {
        backed
    }
}

fn kibibytes(text: &str, name: &str) -> Option<u64> {
    let value = text.lines().find_map(|line| line.strip_prefix(name))?;
    let number = value.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
    number.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use super::*;
    use crate::{Launch, spawn};

    #[test]
    fn stat_fields_are_counted_after_a_command_name_with_spaces_and_parentheses() {
        let line = "4242 (FXServer (main) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 512 128 0 0 20 0 37 0 98765 1000 77\n";

        assert_eq!(
            parse_stat(line),
            Some(Stat {
                cpu_ticks: 640,
                threads: 37,
                start: 98765
            })
        );
        assert_eq!(parse_stat("4242 (cut short) S 1 4242"), None);
    }

    #[test]
    fn status_gives_the_resident_and_private_bytes() {
        let status = "Name:\tFXServer\nVmPeak:\t 9000 kB\nVmRSS:\t  204800 kB\nRssAnon:\t  150000 kB\nRssFile:\t   54800 kB\nVmSwap:\t     512 kB\nThreads:\t37\n";

        assert_eq!(
            parse_status(status),
            Some(Resident {
                resident: 204_800 * 1024,
                private: 150_512 * 1024
            })
        );
        assert_eq!(parse_status("Name:\tzombie\nThreads:\t1\n"), None);
        assert_eq!(
            parse_status("VmRSS:\t 10 kB\nVmSwap:\t 0 kB\n"),
            None,
            "RssAnon is missing"
        );
    }

    #[test]
    fn io_gives_every_byte_read_and_written() {
        let io = "rchar: 323934931\nwchar: 323929600\nsyscr: 632687\nsyscw: 632675\nread_bytes: 0\nwrite_bytes: 323932160\ncancelled_write_bytes: 0\n";

        assert_eq!(parse_io(io), Some((323_934_931, 323_929_600)));
        assert_eq!(parse_io("rchar: 1\n"), None);
    }

    #[test]
    fn the_host_cpu_counts_steal_but_not_guest_time_and_iowait_as_idle() {
        let stat = "cpu  100 10 50 800 40 5 5 2 30 0\ncpu0 50 5 25 400 20 2 3 1 15 0\nintr 1\n";

        assert_eq!(
            parse_cpu(stat),
            Some(CpuTimes {
                busy: 172,
                total: 1012
            })
        );
        assert_eq!(parse_cpu("cpu  1 2 3\n"), None);
        assert_eq!(parse_cpu("intr 1\n"), None);
    }

    #[test]
    fn the_host_s_processors_are_the_numbered_cpu_lines() {
        let stat = "cpu  100 10 50 800 40 5 5 2 30 0\ncpu0 50 5 25 400 20 2 3 1 15 0\n\
                    cpu1 50 5 25 400 20 3 2 1 15 0\ncpufreq 1\nintr 1\nctxt 2\n";

        assert_eq!(parse_processors(stat), Some(2));
        assert_eq!(parse_processors("cpu  1 2 3\nintr 1\n"), None);
    }

    #[test]
    fn meminfo_gives_the_memory_in_use_as_total_minus_available() {
        let meminfo =
            "MemTotal:       16303428 kB\nMemFree:         1200000 kB\nMemAvailable:    9303428 kB\n";

        assert_eq!(
            parse_meminfo(meminfo),
            Some(Memory {
                used: 7_000_000 * 1024,
                total: 16_303_428 * 1024
            })
        );
        assert_eq!(parse_meminfo("MemTotal: 10 kB\n"), None);
    }

    #[test]
    fn net_dev_lines_give_the_receive_and_send_bytes_whatever_columns_follow() {
        let dev = "Inter-|   Receive                                                |  Transmit\n \
                   face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    \
                   lo: 2776770   11307    0    0    0     0          0         0  2776770   11307    0    0    0     0       0          0\n  \
                   eth0:1215645    2751    0    0    0     0          0         0  1782404    4324    0    0    0   427       0          0 99 98\n  \
                   bad: 12 13\n";

        assert_eq!(
            parse_net_dev(dev),
            vec![
                Interface {
                    key: "lo".to_owned(),
                    received: 2_776_770,
                    sent: 2_776_770
                },
                Interface {
                    key: "eth0".to_owned(),
                    received: 1_215_645,
                    sent: 1_782_404
                },
            ]
        );
    }

    #[test]
    fn only_adapters_backed_by_a_device_count_unless_none_is() {
        let interface = |key: &str| Interface {
            key: key.to_owned(),
            received: 1,
            sent: 1,
        };
        let all = vec![interface("lo"), interface("eth0"), interface("docker0")];
        let keys = |interfaces: Vec<Interface>| {
            interfaces
                .into_iter()
                .map(|interface| interface.key)
                .collect::<Vec<_>>()
        };

        assert_eq!(keys(physical(all.clone(), |name| name == "eth0")), ["eth0"]);
        assert_eq!(keys(physical(all, |_| false)), ["eth0", "docker0"]);
    }

    fn shell(script: &str) -> Launch {
        Launch {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
        }
    }

    #[tokio::test]
    async fn the_tree_measures_its_leader_and_its_child_apart() {
        let mut spawned = spawn(&shell("sleep 30 & echo ready; wait")).unwrap();
        assert!(spawned.stdout.next().await.is_some());
        // The child is a copy of the shell until it runs `sleep`.
        let deadline = Instant::now() + Duration::from_secs(5);
        let usage = loop {
            let usage = spawned.tree.usage().unwrap();
            let named = usage.children.iter().any(|child| child.name == "sleep");
            if named || Instant::now() > deadline {
                break usage;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };

        let main = usage.main.expect("sh still runs");
        assert!(main.working_set > 0);
        assert_eq!(main.threads, Some(1));
        assert!(main.handles.is_some_and(|handles| handles >= 3));
        assert!(
            main.written.is_some_and(|written| written >= 6),
            "sh wrote `ready`"
        );
        assert_eq!(usage.children.len(), 1, "{:?}", usage.children);
        let sleep = &usage.children[0];
        assert_eq!(sleep.name, "sleep");
        assert_ne!(sleep.pid, spawned.pid);
        assert!(sleep.usage.started >= main.started);

        spawned.tree.kill().unwrap();
        spawned.exit.wait().await;
        assert!(spawned.tree.wait_empty(Duration::from_secs(5)).await.unwrap());
        assert_eq!(
            spawned.tree.usage().unwrap(),
            TreeUsage {
                main: None,
                children: Vec::new()
            }
        );
    }

    #[test]
    fn observer_measures_itself() {
        let before = own_usage().unwrap();
        let spin = Instant::now();
        while spin.elapsed() < Duration::from_millis(50) {
            std::hint::black_box(spin.elapsed());
        }
        let after = own_usage().unwrap();

        assert!(after.cpu > before.cpu);
        assert!(after.working_set > 1024 * 1024);
        assert!(after.threads.is_some_and(|threads| threads >= 1));
        assert_eq!(after.started, before.started);
    }

    #[test]
    fn the_host_reports_its_processors_memory_and_adapters() {
        let host = host_usage();

        let cpu = host.cpu.unwrap();
        assert!(cpu.total > cpu.busy && cpu.busy > 0);
        let usable = std::thread::available_parallelism().unwrap().get();
        assert!(host.processors.unwrap() as usize >= usable);
        let memory = host.memory.unwrap();
        assert!(memory.total > memory.used && memory.used > 0);
        assert!(host.interfaces.is_some());
    }

    #[test]
    fn a_volume_reports_its_free_and_total_bytes() {
        let space = volume_space(&std::env::temp_dir()).unwrap();

        assert!(space.total >= space.available && space.available > 0);
    }
}
