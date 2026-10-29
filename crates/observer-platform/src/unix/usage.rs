use std::fs;
use std::io;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use super::ProcessTree;
use crate::usage::{
    Clock, CpuTimes, DiskCounters, HostUsage, Interface, Member, Memory, Pressure, Space, ThreadUsage,
    TreeTotals, TreeUsage, Usage,
};

impl ProcessTree {
    /// What the group leader, its threads and each other member of its group use now, and the
    /// group's totals. A process or a thread that exits meanwhile is left out, and so is a process
    /// that left the group.
    pub fn usage(&self) -> io::Result<TreeUsage> {
        let leader = self.group();
        let mut main = None;
        let mut children = Vec::new();
        let mut total = None;
        for pid in self.members()? {
            let Ok(stat) = stat_of(pid) else {
                continue;
            };
            // A process that ended keeps its times until its parent waits for it and takes them
            // into its own `waited`: the total holds them all along.
            *total.get_or_insert(Duration::ZERO) +=
                ticks(stat.user.saturating_add(stat.system).saturating_add(stat.waited));
            let Ok(usage) = measure(pid, &stat) else {
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
        let threads = main.and_then(|_| threads_from(Path::new("/proc"), leader).ok());
        Ok(TreeUsage {
            main,
            children,
            threads,
            totals: total.map(|cpu| TreeTotals {
                cpu,
                processes_started: None,
                private_peak: None,
            }),
        })
    }
}

pub(crate) fn own_usage() -> io::Result<Usage> {
    let pid = std::process::id();
    measure(pid, &stat_of(pid)?)
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
        clock: clock_from(Path::new("/sys/devices/system/cpu")),
        pressure: pressure_from(Path::new("/proc/pressure")),
    }
}

pub(crate) fn volume_space(dir: &Path) -> io::Result<Space> {
    let stat = rustix::fs::statvfs(dir)?;
    Ok(Space {
        available: stat.f_bavail.saturating_mul(stat.f_frsize),
        total: stat.f_blocks.saturating_mul(stat.f_frsize),
    })
}

/// The block device holding `dir`, as `/proc/diskstats` counts it. A file system without one of
/// its own (overlay, tmpfs, a btrfs subvolume, a 9p share such as WSL's `/mnt/c`) is not found.
pub(crate) fn disk_counters(dir: &Path) -> io::Result<DiskCounters> {
    let device = rustix::fs::stat(dir)?.st_dev;
    let (major, minor) = (rustix::fs::major(device), rustix::fs::minor(device));
    parse_diskstats(&fs::read_to_string("/proc/diskstats")?, major, minor).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("device {major}:{minor} has no disk statistics"),
        )
    })
}

fn unreadable(pid: u32, file: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("/proc/{pid}/{file} is unreadable"),
    )
}

fn stat_of(pid: u32) -> io::Result<Stat> {
    parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat"))?).ok_or_else(|| unreadable(pid, "stat"))
}

/// What process `pid` uses, from its `stat` and its other files.
fn measure(pid: u32, stat: &Stat) -> io::Result<Usage> {
    let read = |file: &str| fs::read_to_string(format!("/proc/{pid}/{file}"));
    // A zombie keeps its stat but has no memory left to report.
    let memory = parse_status(&read("status")?).ok_or_else(|| unreadable(pid, "status"))?;
    // Only the owner of a process, or root, may read its I/O counters and descriptors.
    let io = read("io").ok().as_deref().and_then(parse_io);
    let handles = fs::read_dir(format!("/proc/{pid}/fd"))
        .ok()
        .map(|entries| u32::try_from(entries.count()).unwrap_or(u32::MAX));
    Ok(Usage {
        user: ticks(stat.user),
        kernel: ticks(stat.system),
        working_set: memory.resident,
        working_set_peak: memory.peak,
        private: memory.private,
        faults: stat.minor_faults.checked_add(stat.major_faults),
        hard_faults: Some(stat.major_faults),
        threads: Some(stat.threads),
        handles,
        read: io.map(|io| io.0),
        written: io.map(|io| io.1),
        started: stat.start,
    })
}

/// The tasks of process `pid` under `proc_root`. One that ends while they are read is left out.
fn threads_from(proc_root: &Path, pid: u32) -> io::Result<Vec<ThreadUsage>> {
    let tasks = proc_root.join(pid.to_string()).join("task");
    let mut threads = Vec::new();
    for entry in fs::read_dir(&tasks)? {
        let Some(id) = entry?
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(text) = fs::read_to_string(tasks.join(id.to_string()).join("stat")) else {
            continue;
        };
        let (Some(name), Some(stat)) = (parse_comm(&text), parse_stat(&text)) else {
            continue;
        };
        threads.push(ThreadUsage {
            id,
            started: stat.start,
            name: name.to_owned(),
            cpu: ticks(stat.user.saturating_add(stat.system)),
        });
    }
    Ok(threads)
}

fn ticks(count: u64) -> Duration {
    static PER_SECOND: OnceLock<u64> = OnceLock::new();
    let per_second = *PER_SECOND.get_or_init(|| rustix::param::clock_ticks_per_second().max(1));
    Duration::from_secs(count / per_second)
        + Duration::from_nanos((count % per_second) * 1_000_000_000 / per_second)
}

/// The fields of `stat` the statistics use, times in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stat {
    minor_faults: u64,
    major_faults: u64,
    user: u64,
    system: u64,
    /// `cutime` plus `cstime`: the children it waited for.
    waited: u64,
    threads: u32,
    /// Clock ticks after boot.
    start: u64,
}

/// `/proc/<pid>/stat`, or a task's, counted from the last `)` since the command name may hold
/// spaces and parentheses: the state is field 3, `minflt` 10, `majflt` 12, `utime` 14, `stime` 15,
/// `cutime` 16, `cstime` 17, `num_threads` 20, `starttime` 22.
fn parse_stat(text: &str) -> Option<Stat> {
    let fields: Vec<&str> = text.get(text.rfind(')')? + 1..)?.split_whitespace().collect();
    let field = |number: usize| fields.get(number - 3)?.parse::<u64>().ok();
    Some(Stat {
        minor_faults: field(10)?,
        major_faults: field(12)?,
        user: field(14)?,
        system: field(15)?,
        waited: field(16)?.checked_add(field(17)?)?,
        threads: u32::try_from(field(20)?).ok()?,
        start: field(22)?,
    })
}

/// The command name of a `stat` line: what lies between the first `(` and the last `)`.
fn parse_comm(text: &str) -> Option<&str> {
    text.get(text.find('(')? + 1..text.rfind(')')?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Status {
    resident: u64,
    private: u64,
    peak: Option<u64>,
}

/// `/proc/<pid>/status`: `VmRSS`, `RssAnon` with `VmSwap` for the memory no other process shares,
/// and the resident peak `VmHWM`, all in kibibytes.
fn parse_status(text: &str) -> Option<Status> {
    let resident = kibibytes(text, "VmRSS:")?;
    let private = kibibytes(text, "RssAnon:")?.checked_add(kibibytes(text, "VmSwap:")?)?;
    Some(Status {
        resident,
        private,
        peak: kibibytes(text, "VmHWM:"),
    })
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
        steal: Some(steal),
    })
}

/// The `cpuN` lines of `/proc/stat`, one per online processor. The CPUs a container is limited
/// to change neither them nor the first line.
fn parse_processors(text: &str) -> Option<u32> {
    let count = text
        .lines()
        .filter_map(|line| line.split_whitespace().next()?.strip_prefix("cpu"))
        .filter(|number| is_number(number))
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

/// The clock of the processors under `root` (`/sys/devices/system/cpu`) that report all three of
/// `cpufreq`'s figures, in kHz there. Most virtual machines have no `cpufreq` at all.
fn clock_from(root: &Path) -> Option<Clock> {
    let (mut current, mut maximum, mut limit, mut counted) = (0.0, 0.0, 0.0, 0.0);
    for entry in fs::read_dir(root).ok()?.flatten() {
        let name = entry.file_name();
        if !name
            .to_str()
            .and_then(|name| name.strip_prefix("cpu"))
            .is_some_and(is_number)
        {
            continue;
        }
        let cpufreq = entry.path().join("cpufreq");
        let read =
            |file: &str| -> Option<f64> { fs::read_to_string(cpufreq.join(file)).ok()?.trim().parse().ok() };
        let (Some(now), Some(max), Some(allowed)) = (
            read("scaling_cur_freq"),
            read("cpuinfo_max_freq"),
            read("scaling_max_freq"),
        ) else {
            continue;
        };
        if max <= 0.0 {
            continue;
        }
        current += now;
        maximum += max;
        limit += allowed / max * 100.0;
        counted += 1.0;
    }
    (counted > 0.0).then(|| Clock {
        mhz: current / counted / 1000.0,
        max_mhz: maximum / counted / 1000.0,
        limit: Some(limit / counted),
    })
}

/// The `some` stall totals of `cpu`, `memory` and `io` under `root` (`/proc/pressure`), which a
/// kernel built without pressure stall information does not have.
fn pressure_from(root: &Path) -> Option<Pressure> {
    let read = |name: &str| {
        fs::read_to_string(root.join(name))
            .ok()
            .as_deref()
            .and_then(parse_pressure)
    };
    let pressure = Pressure {
        cpu: read("cpu"),
        memory: read("memory"),
        io: read("io"),
    };
    (pressure.cpu.is_some() || pressure.memory.is_some() || pressure.io.is_some()).then_some(pressure)
}

/// `some avg10=0.00 avg60=0.00 avg300=0.00 total=12345`, wherever it stands among the lines.
fn parse_pressure(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("some "))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("total="))?
        .parse()
        .ok()
}

/// The line of device `major:minor` in `/proc/diskstats`: reads completed (field 4), milliseconds
/// reading (7), writes completed (8), milliseconds writing (11). Newer kernels add fields after
/// those.
fn parse_diskstats(text: &str, major: u32, minor: u32) -> Option<DiskCounters> {
    text.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let number = |field: usize| fields.get(field - 1)?.parse::<u64>().ok();
        if number(1)? != u64::from(major) || number(2)? != u64::from(minor) {
            return None;
        }
        Some(DiskCounters {
            transfers: number(4)?.checked_add(number(8)?)?,
            busy: Duration::from_millis(number(7)?.checked_add(number(11)?)?),
        })
    })
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn kibibytes(text: &str, name: &str) -> Option<u64> {
    let value = text.lines().find_map(|line| line.strip_prefix(name))?;
    let number = value.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
    number.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;
    use crate::{Launch, spawn};

    fn write(root: &Path, file: &str, text: &str) {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn stat_fields_are_counted_after_a_command_name_with_spaces_and_parentheses() {
        let line = "4242 (FXServer (main) x) S 1 4242 4242 0 -1 4194560 1500 30 7 0 512 128 40 2 20 0 37 0 98765 1000 77\n";

        assert_eq!(
            parse_stat(line),
            Some(Stat {
                minor_faults: 1500,
                major_faults: 7,
                user: 512,
                system: 128,
                waited: 42,
                threads: 37,
                start: 98765
            })
        );
        assert_eq!(parse_comm(line), Some("FXServer (main) x"));
        assert_eq!(parse_stat("4242 (cut short) S 1 4242"), None);
    }

    #[test]
    fn status_gives_the_resident_private_and_peak_bytes() {
        let status = "Name:\tFXServer\nVmPeak:\t 9000 kB\nVmHWM:\t  250000 kB\nVmRSS:\t  204800 kB\nRssAnon:\t  150000 kB\nRssFile:\t   54800 kB\nVmSwap:\t     512 kB\nThreads:\t37\n";

        assert_eq!(
            parse_status(status),
            Some(Status {
                resident: 204_800 * 1024,
                private: 150_512 * 1024,
                peak: Some(250_000 * 1024),
            })
        );
        assert_eq!(parse_status("Name:\tzombie\nThreads:\t1\n"), None);
        assert_eq!(
            parse_status("VmRSS:\t 10 kB\nVmSwap:\t 0 kB\n"),
            None,
            "RssAnon is missing"
        );
        assert_eq!(
            parse_status("VmRSS:\t 10 kB\nRssAnon:\t 8 kB\nVmSwap:\t 0 kB\n").map(|status| status.peak),
            Some(None)
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
                total: 1012,
                steal: Some(2),
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

    #[test]
    fn the_tasks_of_a_process_are_read_with_their_names_and_a_vanished_one_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        let stat = |id: u32, comm: &str, user: u32, system: u32, start: u32| {
            format!(
                "{id} ({comm}) S 1 4242 4242 0 -1 4194560 10 0 0 0 {user} {system} 0 0 20 0 37 0 {start} 1000 77\n"
            )
        };
        write(
            root.path(),
            "4242/task/4242/stat",
            &stat(4242, "FXServer", 300, 100, 9000),
        );
        write(
            root.path(),
            "4242/task/4250/stat",
            &stat(4250, "UV loop: svNetw", 120, 30, 9010),
        );
        write(
            root.path(),
            "4242/task/4251/stat",
            &stat(4251, "a (b) c", 0, 0, 9011),
        );
        fs::create_dir_all(root.path().join("4242/task/4260")).unwrap();

        let mut threads = threads_from(root.path(), 4242).unwrap();
        threads.sort_by_key(|thread| thread.id);

        let named: Vec<(u32, &str, u64)> = threads
            .iter()
            .map(|thread| (thread.id, thread.name.as_str(), thread.started))
            .collect();
        assert_eq!(
            named,
            [
                (4242, "FXServer", 9000),
                (4250, "UV loop: svNetw", 9010),
                (4251, "a (b) c", 9011)
            ]
        );
        assert_eq!((threads[0].cpu, threads[1].cpu), (ticks(400), ticks(150)));
        assert!(threads_from(root.path(), 4243).is_err(), "no such process");
    }

    #[test]
    fn the_clock_averages_the_processors_that_report_all_three_figures() {
        let root = tempfile::tempdir().unwrap();
        let cpu = |number: u32, now: &str, max: &str, allowed: Option<&str>| {
            write(root.path(), &format!("cpu{number}/cpufreq/scaling_cur_freq"), now);
            write(root.path(), &format!("cpu{number}/cpufreq/cpuinfo_max_freq"), max);
            if let Some(allowed) = allowed {
                write(
                    root.path(),
                    &format!("cpu{number}/cpufreq/scaling_max_freq"),
                    allowed,
                );
            }
        };
        cpu(0, "4200000\n", "5000000\n", Some("5000000\n"));
        cpu(1, "3000000\n", "5000000\n", Some("2500000\n"));
        cpu(2, "1000000\n", "5000000\n", None);
        fs::create_dir_all(root.path().join("cpu3")).unwrap();
        write(root.path(), "cpufreq/boost", "1\n");
        write(root.path(), "online", "0-3\n");

        let clock = clock_from(root.path()).unwrap();

        assert!((clock.mhz - 3600.0).abs() < 1e-9, "{clock:?}");
        assert!((clock.max_mhz - 5000.0).abs() < 1e-9);
        assert!((clock.limit.unwrap() - 75.0).abs() < 1e-9);
        let empty = tempfile::tempdir().unwrap();
        fs::create_dir_all(empty.path().join("cpu0")).unwrap();
        assert_eq!(clock_from(empty.path()), None, "no cpufreq");
        assert_eq!(clock_from(&empty.path().join("missing")), None);
    }

    #[test]
    fn pressure_reads_each_resource_s_some_total_and_none_without_the_files() {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            "cpu",
            "some avg10=1.20 avg60=0.80 avg300=0.10 total=4400123\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
        );
        write(
            root.path(),
            "memory",
            "full avg10=0.00 avg60=0.00 avg300=0.00 total=90\nsome avg10=0.00 avg60=0.00 avg300=0.00 total=1234\n",
        );

        assert_eq!(
            pressure_from(root.path()),
            Some(Pressure {
                cpu: Some(4_400_123),
                memory: Some(1234),
                io: None,
            })
        );
        assert_eq!(pressure_from(&root.path().join("missing")), None);
        assert_eq!(parse_pressure("some avg10=1.00\n"), None);
    }

    #[test]
    fn diskstats_gives_the_transfers_and_their_time_for_a_device_whatever_its_length() {
        let stats = "   8       0 sda 9000 10 72000 4500 3000 20 24000 1500 0 6000 6000\n   \
                       8       1 sda1 8000 5 64000 4000 2000 10 16000 1000 0 5000 5000 0 0 0 0 0 0 0\n \
                     259       0 nvme0n1 120 0 960 30 80 0 640 25 0 50 55 0 0 0 0 40 7\n   \
                       7       0 loop0 1 2\n";

        assert_eq!(
            parse_diskstats(stats, 8, 1),
            Some(DiskCounters {
                transfers: 10_000,
                busy: Duration::from_millis(5000),
            }),
            "a partition, 20 fields"
        );
        assert_eq!(
            parse_diskstats(stats, 8, 0),
            Some(DiskCounters {
                transfers: 12_000,
                busy: Duration::from_millis(6000),
            }),
            "a disk, 14 fields"
        );
        assert_eq!(parse_diskstats(stats, 259, 0).unwrap().transfers, 200);
        assert_eq!(parse_diskstats(stats, 7, 0), None, "cut short");
        assert_eq!(parse_diskstats(stats, 0, 42), None);
    }

    #[test]
    fn the_disk_counters_are_those_of_the_device_holding_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let device = rustix::fs::stat(dir.path()).unwrap().st_dev;
        let (major, minor) = (rustix::fs::major(device), rustix::fs::minor(device));
        let listed = parse_diskstats(&fs::read_to_string("/proc/diskstats").unwrap(), major, minor);

        let counters = disk_counters(dir.path());

        match listed {
            Some(_) => assert!(counters.is_ok_and(|counters| counters.transfers > 0)),
            None => assert_eq!(counters.unwrap_err().kind(), io::ErrorKind::NotFound),
        }
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
        assert!(main.working_set_peak.is_some_and(|peak| peak >= main.working_set));
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
        let threads = usage.threads.expect("sh runs");
        assert_eq!(threads.len(), 1);
        assert_eq!((threads[0].id, threads[0].name.as_str()), (spawned.pid, "sh"));

        spawned.tree.kill().unwrap();
        spawned.exit.wait().await;
        assert!(spawned.tree.wait_empty(Duration::from_secs(5)).await.unwrap());
        assert_eq!(
            spawned.tree.usage().unwrap(),
            TreeUsage {
                main: None,
                children: Vec::new(),
                threads: None,
                totals: None,
            }
        );
    }

    #[tokio::test]
    async fn the_leader_s_threads_keep_their_ids_and_starts() {
        let mut spawned = spawn(&shell("echo ready; sleep 30")).unwrap();
        assert!(spawned.stdout.next().await.is_some());

        let first = spawned.tree.usage().unwrap().threads.expect("sh runs");
        let second = spawned.tree.usage().unwrap().threads.expect("sh runs");

        assert_eq!(first.len(), 1);
        assert_eq!((first[0].id, first[0].started), (second[0].id, second[0].started));
        spawned.tree.kill().unwrap();
        spawned.exit.wait().await;
    }

    #[tokio::test]
    async fn the_tree_totals_keep_a_busy_child_its_parent_waited_for() {
        let busy = "i=0; while [ $i -lt 300000 ]; do i=$((i+1)); done";
        let mut spawned = spawn(&shell(&format!("sh -c '{busy}'; echo ready; sleep 30"))).unwrap();
        assert!(spawned.stdout.next().await.is_some());

        let usage = spawned.tree.usage().unwrap();

        let totals = usage.totals.expect("the group has members");
        let measured: Duration = usage.main.iter().map(Usage::cpu).sum::<Duration>()
            + usage
                .children
                .iter()
                .map(|child| child.usage.cpu())
                .sum::<Duration>();
        assert!(
            totals.cpu >= measured + Duration::from_millis(50),
            "{totals:?} against {measured:?}"
        );
        assert_eq!((totals.processes_started, totals.private_peak), (None, None));
        spawned.tree.kill().unwrap();
        spawned.exit.wait().await;
    }

    #[tokio::test]
    async fn the_tree_totals_keep_a_busy_child_that_ended_before_its_parent_waits_for_it() {
        let busy = "i=0; while [ $i -lt 300000 ]; do i=$((i+1)); done";
        // `sleep` never waits for the child it inherits, which stays a zombie until the end.
        let mut spawned = spawn(&shell(&format!("sh -c '{busy}' & exec sleep 30"))).unwrap();
        let zombie = |pid: &u32| {
            fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                stat.rsplit_once(") ")
                    .is_some_and(|(_, rest)| rest.starts_with('Z'))
            })
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let ended = loop {
            if let Some(pid) = spawned.tree.members().unwrap().into_iter().find(zombie) {
                break pid;
            }
            assert!(Instant::now() < deadline, "the child never ended");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };

        let usage = spawned.tree.usage().unwrap();

        let spun = stat_of(ended).unwrap();
        let spun = ticks(spun.user + spun.system);
        assert!(spun >= Duration::from_millis(50), "{spun:?}");
        let totals = usage.totals.expect("the group has members");
        assert!(totals.cpu >= spun, "{totals:?} against the child's {spun:?}");
        assert!(usage.children.is_empty(), "{:?}", usage.children);
        spawned.tree.kill().unwrap();
        spawned.exit.wait().await;
    }

    #[test]
    fn observer_measures_itself() {
        let before = own_usage().unwrap();
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(50) {
            std::hint::black_box(started.elapsed());
        }
        let after = own_usage().unwrap();

        assert!(after.cpu() > before.cpu());
        assert!(after.working_set > 1024 * 1024);
        assert!(after.threads.is_some_and(|threads| threads >= 1));
        assert_eq!(after.started, before.started);
    }

    #[test]
    fn user_and_kernel_time_are_measured_apart() {
        let before = own_usage().unwrap();
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(200) {
            std::hint::black_box(started.elapsed());
        }
        let spun = own_usage().unwrap();
        // Reading files from /proc spends its time in the kernel.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut after = spun;
        while after.kernel < spun.kernel + Duration::from_millis(20) && Instant::now() < deadline {
            for _ in 0..200 {
                let _ = fs::read_to_string("/proc/self/stat");
            }
            after = own_usage().unwrap();
        }

        assert!(
            spun.user >= before.user + Duration::from_millis(100),
            "{before:?} {spun:?}"
        );
        assert!(
            after.kernel >= spun.kernel + Duration::from_millis(20),
            "{spun:?} {after:?}"
        );
        assert_eq!(after.cpu(), after.user + after.kernel);
    }

    #[test]
    fn touching_64_mb_raises_the_page_faults_and_the_peak_follows() {
        // With huge pages a fault maps 2 MB at once.
        rustix::thread::disable_transparent_huge_pages(true).unwrap();
        let before = own_usage().unwrap();
        let mut memory = vec![0u8; 64 << 20];
        for page in memory.chunks_mut(4096) {
            page[0] = 1;
        }
        std::hint::black_box(&memory);
        let after = own_usage().unwrap();

        let faults = after.faults.unwrap() - before.faults.unwrap();
        assert!(faults >= 16_000, "{faults} faults");
        assert!(after.hard_faults.unwrap() >= before.hard_faults.unwrap());
        let peak = after.working_set_peak.unwrap();
        assert!(peak >= after.working_set && peak >= 64 << 20, "{after:?}");
    }

    #[test]
    fn a_named_thread_of_this_process_is_found_by_its_cut_name_with_its_cpu() {
        let (spun, spinning) = mpsc::channel();
        let (release, held) = mpsc::channel::<()>();
        let worker = std::thread::Builder::new()
            .name("UV loop: svNetwork".to_owned())
            .spawn(move || {
                let started = Instant::now();
                while started.elapsed() < Duration::from_millis(100) {
                    std::hint::black_box(started.elapsed());
                }
                spun.send(()).unwrap();
                let _ = held.recv();
            })
            .unwrap();
        spinning.recv().unwrap();

        let threads = threads_from(Path::new("/proc"), std::process::id()).unwrap();
        release.send(()).unwrap();
        worker.join().unwrap();

        let named = threads
            .iter()
            .find(|thread| thread.name == "UV loop: svNetw")
            .expect("the thread is listed by the first 15 bytes of its name");
        assert!(named.cpu >= Duration::from_millis(50), "{named:?}");
    }

    #[test]
    fn the_host_reports_its_processors_memory_and_adapters() {
        let host = host_usage();

        let cpu = host.cpu.unwrap();
        assert!(cpu.total > cpu.busy && cpu.busy > 0);
        assert!(cpu.steal.is_some());
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
