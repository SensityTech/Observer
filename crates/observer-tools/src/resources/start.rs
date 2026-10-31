//! Which resources FXServer starts and in which order, replayed from the commands of the cfg the
//! way `ServerResources.cpp` and `ResourceDependencyLoader.cpp` run them. The order matters to the
//! clients: they load the data files of the resources in the order the server started them, and a
//! resource never started sends nothing.
//!
//! What the replay follows: a resource starts its dependencies first, `dependency` entries then
//! `dependencies {…}` ones, stopping at the first that cannot start; a name another resource
//! `provide`s resolves to it when no resource of that name runs; a dependency that fails to start
//! keeps its dependants waiting until it starts; `stop` stops the resource's dependants first, by
//! name; `restart` stops then starts, which puts the resource last; `start` and `ensure` of a
//! `[category]` take its resources in byte order of their names, and `ensure` never restarts a
//! resource while the cfg runs. Constraints on the server (`/assetpacks`, `/gameBuild:…`) are
//! decided by FXServer alone: they are taken as met and noted.

use std::collections::{BTreeSet, HashMap, HashSet};

use observer_domain::tools::ToolNoteKind;
use observer_fxserver::cfg::{ResourceCommand, ResourceVerb};

use super::Notes;
use crate::manifest::Entry;

/// Past this many nested starts the replay gives up on the chain: a real server's dependency chains
/// are a few resources long, and the stack stays small whatever the folder holds.
const MAX_DEPTH: usize = 256;

/// What the replay needs of a resource FXServer loaded.
#[derive(Debug, Clone)]
pub(super) struct Candidate<'a> {
    pub name: &'a str,
    /// `dependency` entries, then `dependencie` ones, as FiveM reads them.
    pub dependencies: &'a [Entry],
    pub provides: &'a [Entry],
    pub categories: &'a [String],
    /// Why FXServer refuses to start it once its dependencies run: another game, no `fx_version`.
    pub refusal: Option<String>,
    /// Its manifest, relative to the server folder, for the notes.
    pub manifest: String,
}

/// What the commands start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Stopped,
    Starting,
    Started,
    Stopping,
}

/// What a name resolves to, the way `ResourceManager::GetResource` resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    Resource(usize),
    /// One of the build's system resources: it runs, and holds nothing the tools read.
    System,
    Missing,
}

pub(super) struct Replay<'a, 'n> {
    /// Indexed like the folder's resources: `None` for those FXServer does not load.
    candidates: &'a [Option<Candidate<'a>>],
    by_name: HashMap<&'a str, usize>,
    providers: HashMap<&'a str, Vec<usize>>,
    categories: HashMap<&'a str, BTreeSet<&'a str>>,
    system: &'a HashSet<String>,
    state: Vec<State>,
    /// For each resource, the names of the resources that named it as a dependency.
    dependants: HashMap<usize, BTreeSet<&'a str>>,
    /// For each resource that failed to start, the dependants waiting for it.
    waiting: HashMap<usize, BTreeSet<&'a str>>,
    order: Vec<usize>,
    notes: &'n mut Notes,
}

impl<'a, 'n> Replay<'a, 'n> {
    pub fn new(
        candidates: &'a [Option<Candidate<'a>>],
        system: &'a HashSet<String>,
        notes: &'n mut Notes,
    ) -> Self {
        let mut by_name = HashMap::new();
        let mut providers: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut categories: HashMap<&str, BTreeSet<&str>> = HashMap::new();
        for (index, candidate) in candidates.iter().enumerate() {
            let Some(candidate) = candidate else { continue };
            by_name.insert(candidate.name, index);
            for provide in candidate.provides {
                providers.entry(provide.value.as_str()).or_default().push(index);
            }
            for category in candidate.categories {
                categories
                    .entry(category.as_str())
                    .or_default()
                    .insert(candidate.name);
            }
        }
        Self {
            candidates,
            by_name,
            providers,
            categories,
            system,
            state: vec![State::Stopped; candidates.len()],
            dependants: HashMap::new(),
            waiting: HashMap::new(),
            order: Vec::new(),
            notes,
        }
    }

    /// Runs the cfg's commands, in order.
    pub fn run(&mut self, commands: &[ResourceCommand]) {
        for command in commands {
            let category = is_category(&command.name);
            match command.verb {
                ResourceVerb::Start | ResourceVerb::Ensure if category => {
                    for name in self.category(command) {
                        self.start_named(name, command);
                    }
                }
                ResourceVerb::Start | ResourceVerb::Ensure => self.start_named(&command.name, command),
                ResourceVerb::Stop if category => {
                    for name in self.category(command) {
                        if let Lookup::Resource(index) = self.lookup(name) {
                            self.stop(index);
                        }
                    }
                }
                ResourceVerb::Stop => match self.lookup(&command.name) {
                    Lookup::Resource(index) => self.stop(index),
                    Lookup::System => {}
                    Lookup::Missing => self.missing(command),
                },
                ResourceVerb::Restart => match self.lookup(&command.name) {
                    Lookup::Resource(index) if self.state[index] == State::Started => {
                        self.stop(index);
                        self.start_named(&command.name, command);
                    }
                    Lookup::Resource(_) | Lookup::System => {}
                    Lookup::Missing => self.missing(command),
                },
            }
        }
    }

    /// Starts every resource FXServer loads, in byte order of their names: what a server that
    /// starts everything does, when no cfg is given.
    pub fn start_all(&mut self) {
        let mut names: Vec<(&str, usize)> =
            self.by_name.iter().map(|(&name, &index)| (name, index)).collect();
        names.sort_unstable();
        for (_, index) in names {
            self.start(index, 0);
        }
    }

    /// The resources started, in the order they started: server-only ones included.
    pub fn finish(self) -> Vec<usize> {
        self.order
    }

    /// The resources of a `[category]`, in byte order of their names.
    fn category(&mut self, command: &ResourceCommand) -> Vec<&'a str> {
        let found: Vec<&'a str> = self
            .categories
            .get(command.name.as_str())
            .map(|names| names.iter().copied().collect())
            .unwrap_or_default();
        if found.is_empty() {
            self.missing(command);
        }
        found
    }

    fn missing(&mut self, command: &ResourceCommand) {
        self.notes.add(
            ToolNoteKind::MissingResource,
            None,
            Some(command.file.clone()),
            Some(command.line),
            Some(command.name.clone()),
        );
    }

    fn start_named(&mut self, name: &str, command: &ResourceCommand) {
        match self.lookup(name) {
            Lookup::Resource(index) => {
                self.start(index, 0);
            }
            Lookup::System => {}
            Lookup::Missing => self.missing(command),
        }
    }

    fn lookup(&self, name: &str) -> Lookup {
        let found = self.by_name.get(name).copied();
        if found.is_none_or(|index| self.state[index] == State::Stopped)
            && let Some(providers) = self.providers.get(name)
            && let Some(&provider) = providers
                .iter()
                .find(|&&provider| self.state[provider] == State::Started || found.is_none())
        {
            return Lookup::Resource(provider);
        }
        match found {
            Some(index) => Lookup::Resource(index),
            None if self.system.contains(name) => Lookup::System,
            None => Lookup::Missing,
        }
    }

    /// Starts the resource after its dependencies; whether it runs now.
    fn start(&mut self, index: usize, depth: usize) -> bool {
        match self.state[index] {
            State::Started | State::Starting => return true,
            State::Stopping => return false,
            State::Stopped => {}
        }
        let Some(candidate) = self.candidates.get(index).and_then(Option::as_ref) else {
            return false;
        };
        if depth > MAX_DEPTH {
            self.note(candidate, ToolNoteKind::DependencyNotStarted, None, None);
            return false;
        }
        self.state[index] = State::Starting;
        for dependency in candidate.dependencies {
            let name = dependency.value.as_str();
            if name.starts_with('/') {
                self.note(
                    candidate,
                    ToolNoteKind::ConstraintNotChecked,
                    Some(dependency.line),
                    Some(name),
                );
                continue;
            }
            let other = match self.lookup(name) {
                Lookup::Resource(other) => other,
                Lookup::System => continue,
                Lookup::Missing => {
                    self.note(
                        candidate,
                        ToolNoteKind::MissingDependency,
                        Some(dependency.line),
                        Some(name),
                    );
                    self.state[index] = State::Stopped;
                    return false;
                }
            };
            self.dependants.entry(other).or_default().insert(candidate.name);
            if self.state[other] != State::Stopped {
                continue;
            }
            if !self.start(other, depth + 1) {
                self.note(
                    candidate,
                    ToolNoteKind::DependencyNotStarted,
                    Some(dependency.line),
                    Some(name),
                );
                self.waiting.entry(other).or_default().insert(candidate.name);
                self.state[index] = State::Stopped;
                return false;
            }
        }
        if let Some(refusal) = &candidate.refusal {
            self.note(candidate, ToolNoteKind::ManifestRefused, None, Some(refusal));
            self.state[index] = State::Stopped;
            return false;
        }
        // FXServer starts the dependants that waited for this resource before it lists it.
        for name in self.waiting.remove(&index).unwrap_or_default() {
            if let Lookup::Resource(dependant) = self.lookup(name) {
                self.start(dependant, depth + 1);
            }
        }
        self.order.push(index);
        self.state[index] = State::Started;
        true
    }

    /// Stops the resource, its dependants first, by name.
    fn stop(&mut self, index: usize) {
        if self.state[index] != State::Started {
            return;
        }
        self.state[index] = State::Stopping;
        for name in self.dependants.remove(&index).unwrap_or_default() {
            if let Lookup::Resource(dependant) = self.lookup(name)
                && dependant != index
            {
                self.stop(dependant);
            }
        }
        self.order.retain(|&started| started != index);
        self.state[index] = State::Stopped;
    }

    fn note(
        &mut self,
        candidate: &Candidate<'_>,
        kind: ToolNoteKind,
        line: Option<u32>,
        detail: Option<&str>,
    ) {
        self.notes.add(
            kind,
            Some(candidate.name.to_owned()),
            Some(candidate.manifest.clone()),
            line,
            detail.map(str::to_owned),
        );
    }
}

/// A name in brackets, which FXServer takes as a category.
pub(super) fn is_category(name: &str) -> bool {
    name.len() >= 2 && name.starts_with('[') && name.ends_with(']')
}
