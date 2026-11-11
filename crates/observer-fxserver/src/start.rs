//! How far a start has come, read from what FXServer prints while it runs its cfg: each resource
//! once it is up, and nothing of the one it is starting. The command of the cfg it is at is worked
//! out from the resources each command starts.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::cfg::{ResourceVerb, Scan};
use crate::console::channel;
use crate::server_data::Resources;

/// The slowest resources kept of one start.
const SLOWEST: usize = 5;
/// Warnings of each kind kept of one run.
const MAX_WARNINGS: usize = 2000;
/// Resources of later commands started in a row, none of the command the start is taken to be at,
/// before it is taken to be past that command. FXServer starts a resource's dependencies ahead of
/// their own commands, a few in a row at most; a command whose resource never says how it ended
/// would hold the start there for ever.
const STRAYS: u32 = 3;

/// A command of the cfg that starts resources, and the resources it starts, in lower case.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    file: String,
    line: u32,
    target: String,
    members: Vec<String>,
}

impl Step {
    fn starts(&self, resource: &str) -> bool {
        self.members.iter().any(|member| member == resource)
    }
}

/// The commands of a cfg that start resources, in the order FXServer runs them. Empty when the
/// scan could not read them all: a start cannot be placed among commands that are missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    steps: Vec<Step>,
    resources: Option<u32>,
}

impl Plan {
    pub fn new(scan: &Scan, resources: &Resources) -> Self {
        if !scan.resource_commands_complete {
            return Self::default();
        }
        let steps: Vec<Step> = scan
            .resource_commands
            .iter()
            .filter(|command| matches!(command.verb, ResourceVerb::Start | ResourceVerb::Ensure))
            .map(|command| {
                let mut members = resources.started_by(&command.name);
                // A resource the folder does not hold, as one of the build's own.
                if members.is_empty() && !command.name.starts_with('[') {
                    members.push(command.name.to_ascii_lowercase());
                }
                Step {
                    file: command.file.clone(),
                    line: command.line,
                    target: command.name.clone(),
                    members,
                }
            })
            .collect();
        let distinct: HashSet<&str> = steps
            .iter()
            .flat_map(|step| step.members.iter().map(String::as_str))
            .collect();
        let resources = u32::try_from(distinct.len()).ok();
        Self { steps, resources }
    }
}

/// What a line of FXServer's console says of its start.
#[derive(Debug, PartialEq, Eq)]
enum Said<'a> {
    /// How many resources the folder holds, said once as the scan of it ends.
    Found(u32),
    /// What a `refresh` found: new resources, and how many of those it read again changed.
    Refreshed {
        added: u32,
        changed: u32,
        read: u32,
    },
    /// A resource with server scripts begins to start.
    Entering(&'a str),
    Started {
        resource: &'a str,
        warnings: u32,
    },
    /// A resource did not start, or no resource has that name.
    Failed(&'a str),
    /// A streamed file FXServer finds heavy, as `resource/file`, in tenths of a MiB.
    Heavy {
        path: &'a str,
        tenths: u32,
        memory: Memory,
        oversized: bool,
    },
    /// A file a manifest names that its resource does not hold.
    Missing {
        kind: &'a str,
        file: &'a str,
        manifest: &'a str,
        line: u32,
    },
}

/// The memory FXServer measures a streamed file by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Memory {
    Physical,
    Virtual,
}

impl<'a> Said<'a> {
    fn read(line: &'a str) -> Option<Self> {
        let (channel, text) = channel(line)?;
        // FXServer names the channel `resources:<resource>:stream`, cut at twenty characters.
        if channel.starts_with("resources:") {
            return Self::warning(text);
        }
        if channel == "resources" {
            if let Some(rest) = text.strip_prefix("Started resource ") {
                let (resource, counted) = rest.split_once(' ').unwrap_or((rest, ""));
                let warnings = counted
                    .strip_prefix('(')
                    .and_then(|counted| counted.split_once(" warning"))
                    .and_then(|(count, _)| count.parse().ok())
                    .unwrap_or(0);
                return (!resource.is_empty()).then_some(Self::Started { resource, warnings });
            }
            let found = text.strip_prefix("Found ")?.strip_suffix(" resources.")?;
            // `ServerResources.cpp` says it this way once it read again resources it knew.
            if let Some((added, read)) = found.split_once(" new resources, and refreshed ") {
                let (changed, read) = read.split_once('/')?;
                return Some(Self::Refreshed {
                    added: added.parse().ok()?,
                    changed: changed.parse().ok()?,
                    read: read.parse().ok()?,
                });
            }
            return found.parse().ok().map(Self::Found);
        }
        if channel == "c-scripting-core" {
            let resource = text.strip_prefix("Creating script environments for ")?;
            return (!resource.is_empty()).then_some(Self::Entering(resource));
        }
        if channel != "citizen-server-impl" {
            return None;
        }
        let named = text
            .strip_prefix("Couldn't start resource ")
            .or_else(|| text.strip_prefix("Couldn't find resource "))?;
        let resource = named.strip_suffix('.').unwrap_or(named);
        // A category that is not there starts nothing: its command has no resource to wait for.
        (!resource.is_empty() && !resource.starts_with("category ")).then_some(Self::Failed(resource))
    }

    /// What FXServer says of a resource's files as it starts it, before the line that says it
    /// started: `Asset maps/dock.ydr uses 49.5 MiB of physical memory.`, and
    /// ``Warning: could not find file `x.lua` (defined in fxmanifest.lua:12)``.
    fn warning(text: &'a str) -> Option<Self> {
        if let Some(rest) = text.strip_prefix("Asset ") {
            let (path, said) = rest.split_once(" uses ")?;
            let (size, of) = said.split_once(" MiB of ")?;
            let memory = if of.starts_with("physical memory") {
                Memory::Physical
            } else if of.starts_with("virtual memory") {
                Memory::Virtual
            } else {
                return None;
            };
            let (whole, tenth) = size.split_once('.').unwrap_or((size, "0"));
            let tenths = whole
                .parse::<u32>()
                .ok()?
                .saturating_mul(10)
                .saturating_add(tenth.get(..1)?.parse().ok()?);
            return path.contains('/').then_some(Self::Heavy {
                path,
                tenths,
                memory,
                oversized: of.contains("Oversized assets"),
            });
        }
        let rest = text.strip_prefix("Warning: could not find ")?;
        let (kind, rest) = rest.split_once(" `")?;
        let (file, rest) = rest.split_once("` (defined in ")?;
        let (manifest, line) = rest.strip_suffix(')')?.rsplit_once(':')?;
        Some(Self::Missing {
            kind,
            file,
            manifest,
            line: line.parse().ok()?,
        })
    }
}

/// A streamed file FXServer finds heavy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct HeavyAsset {
    pub resource: String,
    /// Within the resource, as FXServer names it.
    pub file: String,
    /// In tenths of a MiB.
    pub tenths: u32,
    pub memory: Memory,
    /// FXServer says it can keep models from loading.
    pub oversized: bool,
}

/// A file a manifest names that its resource does not hold.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct MissingFile {
    pub resource: String,
    /// What the manifest asks for it as: `client_script`, `file`.
    pub kind: String,
    pub file: String,
    pub manifest: String,
    pub line: u32,
}

/// What FXServer warned about as it started its resources, each once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Warnings {
    pub heavy: Vec<HeavyAsset>,
    pub missing: Vec<MissingFile>,
    /// FXServer warned of more than are kept.
    pub truncated: bool,
}

impl Warnings {
    fn heavy(&mut self, asset: HeavyAsset) {
        if self.heavy.contains(&asset) {
            return;
        }
        if self.heavy.len() < MAX_WARNINGS {
            self.heavy.push(asset);
        } else {
            self.truncated = true;
        }
    }

    fn missing(&mut self, file: MissingFile) {
        if self.missing.contains(&file) {
            return;
        }
        if self.missing.len() < MAX_WARNINGS {
            self.missing.push(file);
        } else {
            self.truncated = true;
        }
    }
}

/// What a `refresh` found, as FXServer said it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refreshed {
    pub at_ms: i64,
    /// Resources it did not know.
    pub added: u32,
    /// Resources whose manifest changed, of those it read again.
    pub changed: u32,
    pub read: u32,
}

/// What a run's console said of its resources, each by its name in lower case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    /// The resources FXServer could not start and has not started since, with when it said so.
    pub failed: HashMap<String, i64>,
    /// What FXServer counted as it last started each resource it warned about.
    pub warned: HashMap<String, u32>,
    /// The heavy files it named of each resource, and how many of them oversized.
    pub heavy: HashMap<String, (u32, u32)>,
    /// The files a manifest names that its resource does not hold.
    pub missing: HashMap<String, u32>,
    /// The latest `refresh` of the run.
    pub refreshed: Option<Refreshed>,
}

/// The last resource FXServer said it started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Started {
    /// As FXServer names it.
    pub name: String,
    pub at_ms: i64,
    /// What FXServer counted after its name.
    pub warnings: u32,
}

/// The command of the cfg a start is at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct At {
    pub file: String,
    pub line: u32,
    /// The command's argument as written: a resource, or a category in brackets.
    pub target: String,
    /// When FXServer came to it.
    pub since_ms: i64,
    /// How many of the resources it starts are up, failed ones counted with them.
    pub done: u32,
    pub of: u32,
    /// The resource whose server scripts FXServer is loading, when it said so.
    pub resource: Option<String>,
}

/// A resource and the time FXServer took from the resource before it to this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Slow {
    pub name: String,
    pub ms: u64,
}

/// A start as it stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// Resources FXServer said it started.
    pub started: u32,
    /// Resources it could not start.
    pub failed: u32,
    /// How many the folder holds, once FXServer has said.
    pub found: Option<u32>,
    /// How many the cfg's commands start, when Observer read them all.
    pub planned: Option<u32>,
    pub last: Option<Started>,
    /// None before the cfg's first command and after its last, or when Observer could not read
    /// every command.
    pub at: Option<At>,
    /// The slowest first.
    pub slowest: Vec<Slow>,
}

#[derive(Debug)]
struct State {
    plan: Plan,
    found: Option<u32>,
    started: HashSet<String>,
    /// When FXServer said it could not start each.
    failed: HashMap<String, i64>,
    /// What FXServer counted as it last started each resource it warned about.
    warned: HashMap<String, u32>,
    refreshed: Option<Refreshed>,
    last: Option<(Instant, Started)>,
    /// When FXServer last said anything of its start, which the next resource's time counts from.
    mark: Instant,
    moved: Option<Instant>,
    /// The step the start is at: every step before it is over.
    step: usize,
    step_since_ms: i64,
    /// The step the start was at before it was taken to be past it without its resources saying
    /// so: one of them starting proves it was not.
    left: Option<usize>,
    strays: u32,
    entering: Option<String>,
    slowest: Vec<Slow>,
    warnings: Warnings,
    /// The files FXServer could not find since the last resource started: it says them before
    /// the resource they belong to, which the line does not name.
    unplaced: Vec<MissingFile>,
}

impl State {
    /// Gives the files not found since the last resource to `resource`, which they are said before.
    fn place_missing(&mut self, resource: &str) {
        for mut file in std::mem::take(&mut self.unplaced) {
            resource.clone_into(&mut file.resource);
            self.warnings.missing(file);
        }
    }

    fn settled(&self, resource: &str) -> bool {
        self.started.contains(resource) || self.failed.contains_key(resource)
    }

    fn settle(&mut self, at_ms: i64) {
        let before = self.step;
        while self
            .plan
            .steps
            .get(self.step)
            .is_some_and(|step| step.members.iter().all(|member| self.settled(member)))
        {
            self.step += 1;
        }
        if self.step != before {
            self.step_since_ms = at_ms;
            self.strays = 0;
        }
    }

    /// Places the start after `resource` started: at its command, or still where it was when the
    /// resource is a dependency started ahead of its own command.
    fn place(&mut self, resource: &str, at_ms: i64) {
        let steps = &self.plan.steps;
        let skipped = self.left.unwrap_or(self.step)..self.step;
        if let Some(back) = skipped.clone().find(|&index| steps[index].starts(resource)) {
            self.step = back;
            self.left = None;
            self.strays = 0;
        } else if steps.get(self.step).is_some_and(|step| step.starts(resource)) {
            self.strays = 0;
        } else if let Some(later) = (self.step + 1..steps.len()).find(|&index| steps[index].starts(resource))
        {
            self.strays += 1;
            if self.strays >= STRAYS {
                self.left.get_or_insert(self.step);
                self.step = later;
                self.step_since_ms = at_ms;
                self.strays = 0;
            }
        }
    }

    fn timed(&mut self, name: &str, took: Duration) {
        let ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
        let rank = self.slowest.partition_point(|slow| slow.ms >= ms);
        if rank < SLOWEST {
            self.slowest.insert(
                rank,
                Slow {
                    name: name.to_owned(),
                    ms,
                },
            );
            self.slowest.truncate(SLOWEST);
        }
    }
}

/// One start, fed by the console's reader and read by the supervisor.
#[derive(Debug)]
pub struct Progress {
    state: Mutex<State>,
}

impl Progress {
    pub fn new(plan: Plan, spawned: Instant, spawned_ms: i64) -> Self {
        Self {
            state: Mutex::new(State {
                plan,
                found: None,
                started: HashSet::new(),
                failed: HashMap::new(),
                warned: HashMap::new(),
                refreshed: None,
                last: None,
                mark: spawned,
                moved: None,
                step: 0,
                step_since_ms: spawned_ms,
                left: None,
                strays: 0,
                entering: None,
                slowest: Vec::new(),
                warnings: Warnings::default(),
                unplaced: Vec::new(),
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a line FXServer printed, colours removed, at `at`.
    pub fn read(&self, line: &str, at: Instant, at_ms: i64) {
        let Some(said) = Said::read(line) else {
            return;
        };
        let mut state = self.state();
        match said {
            Said::Found(found) => state.found = Some(found),
            Said::Refreshed { added, changed, read } => {
                state.refreshed = Some(Refreshed {
                    at_ms,
                    added,
                    changed,
                    read,
                });
                return;
            }
            Said::Entering(resource) => {
                state.entering = Some(resource.to_owned());
                return;
            }
            Said::Heavy {
                path,
                tenths,
                memory,
                oversized,
            } => {
                let (resource, file) = path.split_once('/').unwrap_or((path, ""));
                state.warnings.heavy(HeavyAsset {
                    resource: resource.to_owned(),
                    file: file.to_owned(),
                    tenths,
                    memory,
                    oversized,
                });
                return;
            }
            Said::Missing {
                kind,
                file,
                manifest,
                line,
            } => {
                if state.unplaced.len() < MAX_WARNINGS {
                    state.unplaced.push(MissingFile {
                        resource: String::new(),
                        kind: kind.to_owned(),
                        file: file.to_owned(),
                        manifest: manifest.to_owned(),
                        line,
                    });
                }
                return;
            }
            Said::Started { resource, warnings } => {
                let lower = resource.to_ascii_lowercase();
                let took = at.saturating_duration_since(state.mark);
                if state.started.insert(lower.clone()) {
                    state.timed(resource, took);
                }
                state.failed.remove(&lower);
                if warnings == 0 {
                    state.warned.remove(&lower);
                } else {
                    state.warned.insert(lower.clone(), warnings);
                }
                state.last = Some((
                    at,
                    Started {
                        name: resource.to_owned(),
                        at_ms,
                        warnings,
                    },
                ));
                state.entering = None;
                state.place_missing(resource);
                state.place(&lower, at_ms);
            }
            Said::Failed(resource) => {
                state.place_missing(resource);
                let lower = resource.to_ascii_lowercase();
                if !state.started.contains(&lower) {
                    state.failed.insert(lower, at_ms);
                }
                state.entering = None;
            }
        }
        state.mark = at;
        state.moved = Some(at);
        state.settle(at_ms);
    }

    /// When FXServer last said its start went on.
    pub fn moved(&self) -> Option<Instant> {
        self.state().moved
    }

    /// The last resource FXServer said it started, and when.
    pub fn last(&self) -> Option<(Instant, String)> {
        let state = self.state();
        state
            .last
            .as_ref()
            .map(|(at, started)| (*at, started.name.clone()))
    }

    /// What FXServer warned about so far, in this run.
    pub fn warnings(&self) -> Warnings {
        self.state().warnings.clone()
    }

    /// What the console said of each resource so far, in this run.
    pub fn marks(&self) -> Marks {
        let state = self.state();
        let mut heavy: HashMap<String, (u32, u32)> = HashMap::new();
        for asset in &state.warnings.heavy {
            let (files, oversized) = heavy.entry(asset.resource.to_ascii_lowercase()).or_default();
            *files += 1;
            *oversized += u32::from(asset.oversized);
        }
        let mut missing: HashMap<String, u32> = HashMap::new();
        for file in &state.warnings.missing {
            *missing.entry(file.resource.to_ascii_lowercase()).or_default() += 1;
        }
        Marks {
            failed: state.failed.clone(),
            warned: state.warned.clone(),
            heavy,
            missing,
            refreshed: state.refreshed,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = self.state();
        let count = |resources: usize| u32::try_from(resources).unwrap_or(u32::MAX);
        let at = state.plan.steps.get(state.step).map(|step| At {
            file: step.file.clone(),
            line: step.line,
            target: step.target.clone(),
            since_ms: state.step_since_ms,
            done: u32::try_from(step.members.iter().filter(|member| state.settled(member)).count())
                .unwrap_or(u32::MAX),
            of: u32::try_from(step.members.len()).unwrap_or(u32::MAX),
            resource: state.entering.clone(),
        });
        Snapshot {
            started: count(state.started.len()),
            failed: count(state.failed.len()),
            found: state.found,
            planned: state.plan.resources,
            last: state.last.as_ref().map(|(_, started)| started.clone()),
            at,
            slowest: state.slowest.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use observer_domain::launch::OneSync;

    use super::*;
    use crate::{cfg, server_data};

    fn scanned(cfg_text: &str, folders: &[&str]) -> (Scan, Resources) {
        let dir = tempfile::tempdir().unwrap();
        for folder in folders {
            let path = dir.path().join("resources").join(folder);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("fxmanifest.lua"), "fx_version 'cerulean'\n").unwrap();
        }
        fs::write(dir.path().join("server.cfg"), cfg_text).unwrap();
        let resources = server_data::walk(dir.path());
        let context = cfg::Context {
            panel: None,
            onesync: OneSync::On,
            probe_ports: false,
            system_resources: None,
            follow_outside: false,
            convars: None,
        };
        let scan = cfg::scan(dir.path(), "server.cfg", cfg_text, &resources, &context);
        (scan, resources)
    }

    #[test]
    fn a_plan_holds_the_commands_that_start_resources_with_those_each_one_starts() {
        let (scan, resources) = scanned(
            "ensure chat\nstop chat\nensure [Maps]\nstart sessionmanager\nensure [gone]\nrestart chat\n",
            &["chat", "[maps]/docks", "[maps]/[interiors]/bank"],
        );

        let plan = Plan::new(&scan, &resources);

        assert_eq!(
            plan.steps,
            [
                step(1, "chat", &["chat"]),
                step(3, "[Maps]", &["bank", "docks"]),
                step(4, "sessionmanager", &["sessionmanager"]),
                step(5, "[gone]", &[]),
            ]
        );
        assert_eq!(plan.resources, Some(4));
    }

    #[test]
    fn a_cfg_whose_commands_could_not_all_be_read_gives_no_plan() {
        let (scan, resources) = scanned("ensure chat\nexec @chat/more.cfg\n", &["chat"]);
        assert!(!scan.resource_commands_complete);

        assert_eq!(Plan::new(&scan, &resources), Plan::default());
    }

    fn step(line: u32, target: &str, members: &[&str]) -> Step {
        Step {
            file: "server.cfg".to_owned(),
            line,
            target: target.to_owned(),
            members: members.iter().map(|&member| member.to_owned()).collect(),
        }
    }

    fn plan(steps: Vec<Step>) -> Plan {
        let resources = steps.iter().map(|step| step.members.len()).sum::<usize>();
        Plan {
            steps,
            resources: u32::try_from(resources).ok(),
        }
    }

    struct Start {
        progress: Progress,
        spawned: Instant,
    }

    impl Start {
        fn new(plan: Plan) -> Self {
            let spawned = Instant::now();
            Self {
                progress: Progress::new(plan, spawned, 0),
                spawned,
            }
        }

        fn says(&self, seconds: u64, line: &str) -> &Self {
            let at = self.spawned + Duration::from_secs(seconds);
            self.progress
                .read(line, at, i64::try_from(seconds * 1000).unwrap_or(i64::MAX));
            self
        }

        fn starts(&self, seconds: u64, resource: &str) -> &Self {
            self.says(
                seconds,
                &format!("[           resources] Started resource {resource}"),
            )
        }

        fn at(&self) -> Option<(u32, String)> {
            self.progress.snapshot().at.map(|at| (at.line, at.target))
        }
    }

    #[test]
    fn the_lines_of_a_start_are_read_from_fxservers_own_channels_only() {
        assert_eq!(
            Said::read("[           resources] Started resource Docks_Props"),
            Some(Said::Started {
                resource: "Docks_Props",
                warnings: 0
            })
        );
        assert_eq!(
            Said::read("[           resources] Started resource maps (1 warning)"),
            Some(Said::Started {
                resource: "maps",
                warnings: 1
            })
        );
        assert_eq!(
            Said::read("[           resources] Started resource maps (12 warnings)"),
            Some(Said::Started {
                resource: "maps",
                warnings: 12
            })
        );
        assert_eq!(
            Said::read("[           resources] Found 391 resources."),
            Some(Said::Found(391))
        );
        assert_eq!(
            Said::read("[    c-scripting-core] Creating script environments for chat"),
            Some(Said::Entering("chat"))
        );
        assert_eq!(
            Said::read("[ citizen-server-impl] Couldn't start resource nursery."),
            Some(Said::Failed("nursery"))
        );
        assert_eq!(
            Said::read("[ citizen-server-impl] Couldn't find resource typo."),
            Some(Said::Failed("typo"))
        );
        for line in [
            "[ citizen-server-impl] Couldn't find resource category [hotel].",
            "[      script:chat] Started resource maps",
            "[      script:chat] Couldn't start resource maps.",
            "Started resource maps",
            "[           resources] Scanning resources.",
            "[           resources] Stopping resource maps",
        ] {
            assert_eq!(Said::read(line), None, "{line}");
        }
    }

    #[test]
    fn what_fxserver_warns_of_is_kept_once_with_the_resource_it_was_starting() {
        let start = Start::new(plan(Vec::new()));
        let heavy = "[resources:maps:stream] Asset maps/dock.ydr uses 49.5 MiB of physical memory. \
            Oversized assets can and WILL lead to streaming issues (such as models not loading/rendering).";
        start
            .says(1, heavy)
            .says(1, "[  resources:maps:str] Asset maps/pier.ytd uses 16 MiB of virtual memory.")
            .says(1, "[resources:Long_Name] Warning: could not find file `data/**/x.meta` (defined in fxmanifest.lua:12)")
            .says(1, "[resources:Long_Name] Warning: could not find client_script `cl.lua` (defined in __resource.lua:3)")
            .says(1, "[resources:Long_Name] Warning: Long_Name1 does not have a resource manifest (fxmanifest.lua)")
            .starts(2, "Long_Name_Of_A_Resource (2 warnings)")
            .says(3, heavy)
            .says(3, "[         script:maps] Asset maps/fake.ydr uses 99.0 MiB of physical memory.")
            .says(3, "[resources:gone:strea] Warning: could not find file `a.lua` (defined in fxmanifest.lua:1)")
            .says(4, "[ citizen-server-impl] Couldn't start resource gone.");

        assert_eq!(
            start.progress.warnings(),
            Warnings {
                heavy: vec![
                    HeavyAsset {
                        resource: "maps".to_owned(),
                        file: "dock.ydr".to_owned(),
                        tenths: 495,
                        memory: Memory::Physical,
                        oversized: true,
                    },
                    HeavyAsset {
                        resource: "maps".to_owned(),
                        file: "pier.ytd".to_owned(),
                        tenths: 160,
                        memory: Memory::Virtual,
                        oversized: false,
                    },
                ],
                missing: vec![
                    MissingFile {
                        resource: "Long_Name_Of_A_Resource".to_owned(),
                        kind: "file".to_owned(),
                        file: "data/**/x.meta".to_owned(),
                        manifest: "fxmanifest.lua".to_owned(),
                        line: 12,
                    },
                    MissingFile {
                        resource: "Long_Name_Of_A_Resource".to_owned(),
                        kind: "client_script".to_owned(),
                        file: "cl.lua".to_owned(),
                        manifest: "__resource.lua".to_owned(),
                        line: 3,
                    },
                    MissingFile {
                        resource: "gone".to_owned(),
                        kind: "file".to_owned(),
                        file: "a.lua".to_owned(),
                        manifest: "fxmanifest.lua".to_owned(),
                        line: 1,
                    },
                ],
                truncated: false,
            }
        );
    }

    #[test]
    fn a_start_is_at_the_first_command_whose_resources_are_not_all_up() {
        let start = Start::new(plan(vec![
            step(3, "chat", &["chat"]),
            step(4, "[maps]", &["bridge", "docks", "pier"]),
            step(9, "jobs", &["jobs"]),
        ]));
        assert_eq!(start.at(), Some((3, "chat".to_owned())));

        start.starts(2, "chat");
        let snapshot = start.progress.snapshot();
        let at = snapshot.at.unwrap();
        assert_eq!((at.line, at.since_ms, at.done, at.of), (4, 2000, 0, 3));

        start.starts(5, "Docks").starts(6, "pier");
        let at = start.progress.snapshot().at.unwrap();
        assert_eq!((at.line, at.since_ms, at.done, at.of), (4, 2000, 2, 3));

        start.starts(9, "bridge");
        assert_eq!(start.at(), Some((9, "jobs".to_owned())));
        start.starts(10, "jobs");
        assert_eq!(start.at(), None);
        let snapshot = start.progress.snapshot();
        assert_eq!((snapshot.started, snapshot.planned), (5, Some(5)));
        assert_eq!(snapshot.last.unwrap().name, "jobs");
    }

    #[test]
    fn a_resource_that_does_not_start_does_not_hold_the_start_at_its_command() {
        let start = Start::new(plan(vec![
            step(3, "typo", &["typo"]),
            step(4, "[props]", &["nursery", "toys"]),
            step(5, "[hotel]", &[]),
            step(6, "jobs", &["jobs"]),
        ]));

        start.says(1, "[ citizen-server-impl] Couldn't find resource typo.");
        assert_eq!(start.at(), Some((4, "[props]".to_owned())));
        start
            .starts(2, "toys")
            .says(3, "[ citizen-server-impl] Couldn't start resource nursery.");

        assert_eq!(start.at(), Some((6, "jobs".to_owned())));
        let snapshot = start.progress.snapshot();
        assert_eq!((snapshot.started, snapshot.failed), (1, 2));
    }

    #[test]
    fn a_dependency_started_ahead_of_its_command_leaves_the_start_where_it_is() {
        let start = Start::new(plan(vec![
            step(3, "framework", &["framework"]),
            step(4, "database", &["database"]),
            step(5, "jobs", &["jobs"]),
        ]));

        start.starts(1, "database");
        assert_eq!(start.at(), Some((3, "framework".to_owned())));

        start.starts(4, "framework");
        assert_eq!(
            start.at(),
            Some((5, "jobs".to_owned())),
            "the database is up already"
        );
    }

    #[test]
    fn a_command_whose_resource_never_says_how_it_ended_is_left_after_three_later_ones() {
        let start = Start::new(plan(vec![
            step(3, "[props]", &["silent", "toys"]),
            step(4, "a", &["a"]),
            step(5, "b", &["b"]),
            step(6, "c", &["c"]),
            step(7, "d", &["d"]),
        ]));

        start.starts(1, "toys").starts(2, "a").starts(3, "b");
        assert_eq!(start.at(), Some((3, "[props]".to_owned())));
        start.starts(4, "c");
        assert_eq!(start.at(), Some((7, "d".to_owned())));

        start.starts(9, "silent");
        assert_eq!(
            start.at(),
            Some((7, "d".to_owned())),
            "what was left is over, and so are a, b and c"
        );
    }

    #[test]
    fn a_command_left_too_soon_is_come_back_to_when_its_resource_starts() {
        let start = Start::new(plan(vec![
            step(3, "framework", &["framework"]),
            step(4, "a", &["a"]),
            step(5, "b", &["b"]),
            step(6, "c", &["c"]),
            step(7, "[maps]", &["docks", "pier"]),
        ]));

        start.starts(1, "a").starts(2, "b").starts(3, "c");
        assert_eq!(start.at(), Some((7, "[maps]".to_owned())));

        start.starts(60, "framework");
        assert_eq!(
            start.at(),
            Some((7, "[maps]".to_owned())),
            "its three dependencies were up"
        );
        let start = Start::new(plan(vec![
            step(3, "[core]", &["framework", "hud"]),
            step(4, "a", &["a"]),
            step(5, "b", &["b"]),
            step(6, "c", &["c"]),
            step(7, "d", &["d"]),
        ]));
        start.starts(1, "a").starts(2, "b").starts(3, "c");
        assert_eq!(start.at(), Some((7, "d".to_owned())));
        start.starts(60, "framework");
        assert_eq!(
            start.at(),
            Some((3, "[core]".to_owned())),
            "hud is still to start"
        );
    }

    #[test]
    fn the_slowest_resources_are_timed_from_the_one_before_them() {
        let start = Start::new(plan(Vec::new()));
        start.says(4, "[           resources] Found 12 resources.");
        for (seconds, resource) in [
            (5, "a"),
            (11, "b"),
            (12, "c"),
            (20, "d"),
            (22, "e"),
            (25, "f"),
            (29, "g"),
        ] {
            start.starts(seconds, resource);
        }
        start.starts(40, "b");

        let snapshot = start.progress.snapshot();
        let slowest: Vec<(&str, u64)> = snapshot
            .slowest
            .iter()
            .map(|slow| (slow.name.as_str(), slow.ms))
            .collect();
        assert_eq!(
            slowest,
            [("d", 8000), ("b", 6000), ("g", 4000), ("f", 3000), ("e", 2000)]
        );
        assert_eq!(
            (snapshot.found, snapshot.started, snapshot.at),
            (Some(12), 7, None)
        );
    }

    #[test]
    fn what_the_console_says_of_a_resource_is_kept_by_its_name_until_it_starts_clean() {
        let start = Start::new(plan(Vec::new()));
        start
            .says(1, "[  resources:maps:str] Asset maps/dock.ydr uses 49.5 MiB of physical memory. Oversized assets can and WILL lead to streaming issues (such as models not loading/rendering).")
            .says(1, "[  resources:maps:str] Asset maps/pier.ytd uses 16 MiB of virtual memory.")
            .says(1, "[resources:Maps:strea] Warning: could not find file `x.meta` (defined in fxmanifest.lua:12)")
            .starts(2, "Maps (3 warnings)")
            .starts(3, "chat")
            .says(4, "[ citizen-server-impl] Couldn't start resource Nursery.")
            .says(5, "[ citizen-server-impl] Couldn't start resource jobs.");

        let marks = start.progress.marks();
        assert_eq!(marks.warned, HashMap::from([("maps".to_owned(), 3)]));
        assert_eq!(marks.heavy, HashMap::from([("maps".to_owned(), (2, 1))]));
        assert_eq!(marks.missing, HashMap::from([("maps".to_owned(), 1)]));
        assert_eq!(
            marks.failed,
            HashMap::from([("nursery".to_owned(), 4000), ("jobs".to_owned(), 5000)])
        );
        assert_eq!(marks.refreshed, None);

        start.starts(6, "maps").starts(7, "jobs");
        let marks = start.progress.marks();
        assert_eq!(marks.warned, HashMap::new(), "it started without a warning since");
        assert_eq!(marks.failed, HashMap::from([("nursery".to_owned(), 4000)]));
    }

    #[test]
    fn a_refresh_says_what_it_found_and_a_start_only_how_many() {
        assert_eq!(
            Said::read("[           resources] Found 2 new resources, and refreshed 3/391 resources."),
            Some(Said::Refreshed {
                added: 2,
                changed: 3,
                read: 391
            })
        );
        assert_eq!(
            Said::read("[           resources] Found 0 new resources, and refreshed 0/7 resources."),
            Some(Said::Refreshed {
                added: 0,
                changed: 0,
                read: 7
            })
        );
        for line in [
            "[           resources] Found 2 new resources, and refreshed 3 resources.",
            "[           resources] Found x new resources, and refreshed 3/391 resources.",
            "[         script:evil] Found 2 new resources, and refreshed 3/391 resources.",
        ] {
            assert_eq!(Said::read(line), None, "{line}");
        }
        let start = Start::new(plan(Vec::new()));

        start.says(1, "[           resources] Found 391 resources.").says(
            90,
            "[           resources] Found 1 new resources, and refreshed 0/391 resources.",
        );

        assert_eq!(
            start.progress.marks().refreshed,
            Some(Refreshed {
                at_ms: 90_000,
                added: 1,
                changed: 0,
                read: 391
            })
        );
        assert_eq!(start.progress.snapshot().found, Some(391));
        assert_eq!(
            start.progress.moved(),
            Some(start.spawned + Duration::from_secs(1)),
            "a refresh is no part of the start"
        );
    }

    #[test]
    fn the_resource_whose_scripts_load_is_named_until_it_is_up() {
        let start = Start::new(plan(vec![step(3, "[core]", &["framework", "hud"])]));

        start.says(
            1,
            "[    c-scripting-core] Creating script environments for framework",
        );
        assert_eq!(
            start.progress.snapshot().at.unwrap().resource.as_deref(),
            Some("framework")
        );
        assert_eq!(start.progress.moved(), None, "nothing is up yet");

        start.starts(3, "framework");
        assert_eq!(start.progress.snapshot().at.unwrap().resource, None);
        assert_eq!(
            start.progress.moved(),
            Some(start.spawned + Duration::from_secs(3))
        );
    }
}
