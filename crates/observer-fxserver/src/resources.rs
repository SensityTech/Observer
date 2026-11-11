//! The resources FXServer knows while it runs, as its bridge reports them: every one with its
//! folder, its version and what its manifest depends on once the cfg has run and after each
//! `refresh`, then those started whenever one starts or stops. How FXServer names, finds and stops
//! resources is read from its sources: `ServerResourceList.cpp`, `ResourceManager.cpp` and
//! `ResourceDependencyLoader.cpp`.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

/// Resources kept of one list. A large server holds a few hundred.
pub const MAX_RESOURCES: usize = 5000;
const MAX_NAME: usize = 128;
const MAX_VERSION: usize = 64;
/// Dependencies or provided names kept of one manifest.
const MAX_LINKS: usize = 256;
const MAX_CATEGORIES: usize = 16;
/// The folder of a build that holds FXServer's own resources, and Observer's bridge.
const SYSTEM_RESOURCES: &str = "system_resources";

/// One resource of the bridge's list, as FXServer gives it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Reported {
    #[serde(default)]
    pub name: String,
    /// What `GetResourceState` said.
    #[serde(default)]
    pub state: String,
    /// What `GetResourcePath` said: a folder of the host.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// The manifest's dependencies, constraints such as `/onesync` among them.
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Started,
    Starting,
    Stopped,
    Stopping,
    /// Anything else FXServer may say: `uninitialized`, `missing`, `unknown`.
    Unknown,
}

impl State {
    fn read(text: &str) -> Self {
        match text {
            "started" => Self::Started,
            "starting" => Self::Starting,
            "stopped" => Self::Stopped,
            "stopping" => Self::Stopping,
            _ => Self::Unknown,
        }
    }
}

/// Which of FXServer's two roots a resource lies under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The server folder's `resources`.
    Server,
    /// The build's `system_resources`.
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnKind {
    Started,
    Stopped,
    Restarted,
}

/// What last happened to a resource while the server ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub at_ms: i64,
    pub kind: TurnKind,
    /// The resource it stopped with, being one that depends on it.
    pub with: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// As FXServer names it.
    pub name: String,
    pub state: State,
    pub origin: Origin,
    /// The category folders it lies under, outermost first, as `[core]/[ox]`: empty at the root.
    pub folder: String,
    pub version: Option<String>,
    /// The resources its manifest depends on, by the names it gives.
    pub needs: Vec<String>,
    /// The names it answers to besides its own.
    pub provides: Vec<String>,
    pub turn: Option<Turn>,
}

/// The resources a list brought that the list before it did not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub at_ms: i64,
    pub names: Vec<String>,
}

/// A list being received, page by page.
#[derive(Debug, Clone)]
struct Incoming {
    list: u64,
    next_page: u32,
    resources: Vec<Reported>,
    truncated: bool,
}

/// What one run of FXServer knows of its resources.
#[derive(Debug, Clone, Default)]
pub struct Book {
    known: Vec<Known>,
    listed: bool,
    truncated: bool,
    added: Option<Added>,
    incoming: Option<Incoming>,
    /// The resources a report said were started while a list was being received: the list's
    /// first pages were read before it.
    held: Option<Vec<String>>,
}

impl Book {
    /// Whether FXServer has listed its resources in this run.
    pub fn listed(&self) -> bool {
        self.listed
    }

    /// In name order, without regard to case.
    pub fn resources(&self) -> &[Known] {
        &self.known
    }

    /// The list held more resources than are kept, or names too long to keep.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// What the latest list added to the one before: none for the first list of a run.
    pub fn added(&self) -> Option<&Added> {
        self.added.as_ref()
    }

    pub fn find(&self, name: &str) -> Option<&Known> {
        self.known.iter().find(|known| known.name == name)
    }

    /// Takes one page of the list numbered `number`, and says whether it completed the list. Pages
    /// come in order from 1; a page out of turn, or of another list than the one being received,
    /// is dropped. `bridge`, Observer's own resource in this run, is left out.
    pub fn page(
        &mut self,
        number: u64,
        page: u32,
        last: bool,
        resources: Vec<Reported>,
        bridge: &str,
        at_ms: i64,
    ) -> bool {
        if page == 1 {
            self.incoming = Some(Incoming {
                list: number,
                next_page: 1,
                resources: Vec::new(),
                truncated: false,
            });
        }
        let Some(incoming) = self
            .incoming
            .as_mut()
            .filter(|incoming| incoming.list == number && incoming.next_page == page)
        else {
            return false;
        };
        incoming.next_page += 1;
        let room = MAX_RESOURCES.saturating_sub(incoming.resources.len());
        incoming.truncated |= resources.len() > room;
        incoming.resources.extend(resources.into_iter().take(room));
        if !last {
            return false;
        }
        let Some(incoming) = self.incoming.take() else {
            return false;
        };
        self.replace(incoming, bridge, at_ms);
        if let Some(held) = self.held.take() {
            self.started(&held, &[], at_ms);
        }
        true
    }

    /// The list just received takes the place of the one before, whose turns the resources still
    /// there keep.
    fn replace(&mut self, incoming: Incoming, bridge: &str, at_ms: i64) {
        let before = std::mem::take(&mut self.known);
        let names_before: Option<HashSet<String>> = self
            .listed
            .then(|| before.iter().map(|known| known.name.clone()).collect());
        let mut turns: HashMap<String, Turn> = before
            .into_iter()
            .filter_map(|known| Some((known.name, known.turn?)))
            .collect();
        let mut truncated = incoming.truncated;
        let mut seen = HashSet::new();
        let mut known: Vec<Known> = incoming
            .resources
            .into_iter()
            .filter_map(|reported| {
                let name = reported.name;
                if name.is_empty() || name.eq_ignore_ascii_case(bridge) {
                    return None;
                }
                if name.len() > MAX_NAME {
                    truncated = true;
                    return None;
                }
                if !seen.insert(name.clone()) {
                    return None;
                }
                let (origin, folder) = place(reported.path.as_deref().unwrap_or_default());
                Some(Known {
                    state: State::read(&reported.state),
                    origin,
                    folder,
                    version: reported
                        .version
                        .map(|version| version.trim().chars().take(MAX_VERSION).collect::<String>())
                        .filter(|version| !version.is_empty()),
                    needs: links(reported.needs, true),
                    provides: links(reported.provides, false),
                    turn: turns.remove(&name),
                    name,
                })
            })
            .collect();
        known.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        self.added = names_before.map(|before| Added {
            at_ms,
            names: known
                .iter()
                .filter(|known| !before.contains(&known.name))
                .map(|known| known.name.clone())
                .collect(),
        });
        self.known = known;
        self.truncated = truncated;
        self.listed = true;
    }

    /// Takes the resources started now and the starts and stops since the report before, as
    /// `(name, started)` in the order they happened, and says whether anything changed. Nothing is
    /// known of a run that has not listed its resources yet.
    pub fn started(&mut self, started: &[String], events: &[(String, bool)], at_ms: i64) -> bool {
        if self.incoming.is_some() {
            self.held = Some(started.to_vec());
        }
        if !self.listed {
            return false;
        }
        let mut turns = turns(events);
        let stopped: Vec<String> = turns
            .iter()
            .filter(|(_, kind)| *kind != TurnKind::Started)
            .map(|(name, _)| name.clone())
            .collect();
        let with: HashMap<String, String> = stopped
            .iter()
            .filter_map(|name| Some((name.clone(), self.stopped_with(name, &stopped)?)))
            .collect();
        let up: HashSet<&str> = started.iter().map(String::as_str).collect();
        let mut changed = false;
        for known in &mut self.known {
            let state = match (up.contains(known.name.as_str()), known.state) {
                (true, _) => State::Started,
                (false, State::Started) => State::Stopped,
                (false, state) => state,
            };
            let told = turns
                .iter()
                .position(|(name, _)| *name == known.name)
                .map(|index| turns.swap_remove(index).1);
            // A report tells 64 starts and stops at most: past them, the states alone say.
            let kind = told.or(match (known.state, state) {
                (before, State::Started) if before != State::Started => Some(TurnKind::Started),
                (State::Started, State::Stopped) => Some(TurnKind::Stopped),
                _ => None,
            });
            if let Some(kind) = kind {
                known.turn = Some(Turn {
                    at_ms,
                    kind,
                    with: with.get(&known.name).cloned(),
                });
                changed = true;
            }
            changed |= known.state != state;
            known.state = state;
        }
        changed
    }

    /// The resource among `stopped` that `name` stopped with: one it depends on, directly or not,
    /// that depends on none of the others. FXServer stops what depends on a resource before the
    /// resource itself, so the report tells both.
    fn stopped_with(&self, name: &str, stopped: &[String]) -> Option<String> {
        let needed = self.needed_by(name);
        stopped
            .iter()
            .filter(|other| other.as_str() != name && needed.contains(other.as_str()))
            .find(|root| {
                let above = self.needed_by(root);
                !stopped.iter().any(|other| above.contains(other.as_str()))
            })
            .cloned()
    }

    /// Every resource `name` depends on, directly or not, whatever is started.
    fn needed_by(&self, name: &str) -> HashSet<&str> {
        let mut needed: HashSet<&str> = HashSet::new();
        let mut queue = vec![name];
        while let Some(current) = queue.pop() {
            let Some(known) = self.find(current) else {
                continue;
            };
            for need in &known.needs {
                let found = self.find(need).or_else(|| {
                    self.known
                        .iter()
                        .find(|other| other.provides.iter().any(|provided| provided == need))
                });
                if let Some(found) = found
                    && found.name != name
                    && needed.insert(found.name.as_str())
                {
                    queue.push(found.name.as_str());
                }
            }
        }
        needed
    }

    /// What depends on what among the started resources, as they stand: worked out once, for
    /// every resource asked of it after.
    pub fn dependants(&self) -> Dependants<'_> {
        let known = &self.known;
        let started = |at: usize| known[at].state == State::Started;
        let named: HashMap<&str, usize> = known
            .iter()
            .enumerate()
            .map(|(at, resource)| (resource.name.as_str(), at))
            .collect();
        // `ResourceManager::GetResource` finds a dependency by the resource of that name when it
        // is started, or else by a started one that provides the name.
        let mut provided: HashMap<&str, usize> = HashMap::new();
        for (at, resource) in known.iter().enumerate().filter(|(at, _)| started(*at)) {
            for name in &resource.provides {
                provided.entry(name.as_str()).or_insert(at);
            }
        }
        let mut direct: Vec<Vec<usize>> = vec![Vec::new(); known.len()];
        for (at, resource) in known.iter().enumerate().filter(|(at, _)| started(*at)) {
            for need in &resource.needs {
                let on = named
                    .get(need.as_str())
                    .copied()
                    .filter(|on| started(*on))
                    .or_else(|| provided.get(need.as_str()).copied());
                if let Some(on) = on
                    && on != at
                    && !direct[on].contains(&at)
                {
                    direct[on].push(at);
                }
            }
        }
        Dependants { known, named, direct }
    }
}

/// The started resources of a run by what depends on what.
pub struct Dependants<'a> {
    known: &'a [Known],
    named: HashMap<&'a str, usize>,
    /// For each resource, the started ones whose manifest names it.
    direct: Vec<Vec<usize>>,
}

impl Dependants<'_> {
    /// The started resources that depend on `name`, directly or not, the nearest first: FXServer
    /// stops them with it, and starts none of them again.
    pub fn of(&self, name: &str) -> Vec<String> {
        let Some(&root) = self.named.get(name) else {
            return Vec::new();
        };
        let mut seen = vec![false; self.known.len()];
        seen[root] = true;
        let mut found: Vec<usize> = Vec::new();
        let mut frontier = vec![root];
        while !frontier.is_empty() {
            let mut next: Vec<usize> = frontier
                .iter()
                .flat_map(|at| &self.direct[*at])
                .copied()
                .filter(|at| !std::mem::replace(&mut seen[*at], true))
                .collect();
            next.sort_unstable();
            found.extend(&next);
            frontier = next;
        }
        found.into_iter().map(|at| self.known[at].name.clone()).collect()
    }
}

/// What became of each resource over one report's starts and stops, in the order each first
/// showed: a stop then a start is a restart.
fn turns(events: &[(String, bool)]) -> Vec<(String, TurnKind)> {
    let mut turns: Vec<(String, TurnKind)> = Vec::new();
    for (name, started) in events {
        let turn = turns.iter_mut().find(|(known, _)| known == name);
        match (turn, started) {
            (Some((_, kind)), true) => {
                *kind = match kind {
                    TurnKind::Stopped | TurnKind::Restarted => TurnKind::Restarted,
                    TurnKind::Started => TurnKind::Started,
                };
            }
            (Some((_, kind)), false) => *kind = TurnKind::Stopped,
            (None, true) => turns.push((name.clone(), TurnKind::Started)),
            (None, false) => turns.push((name.clone(), TurnKind::Stopped)),
        }
    }
    turns
}

/// Where a resource lies, from its path as FXServer gives it. `ServerResourceList.cpp` takes a
/// folder named `[x]` for a category, read in turn, and any other for a resource: the categories
/// are the folders that end the path, and what holds them is the root.
fn place(path: &str) -> (Origin, String) {
    let mut folders: Vec<&str> = path.split(['/', '\\']).filter(|part| !part.is_empty()).collect();
    folders.pop();
    let categories = folders
        .iter()
        .rev()
        .take_while(|folder| folder.starts_with('[') && folder.ends_with(']'))
        .count();
    let root = folders.len() - categories;
    let origin = if root > 0 && folders[root - 1] == SYSTEM_RESOURCES {
        Origin::Build
    } else {
        Origin::Server
    };
    let kept = &folders[root..];
    let kept = &kept[kept.len().saturating_sub(MAX_CATEGORIES)..];
    let folder = kept
        .iter()
        .filter(|category| category.len() <= MAX_NAME)
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    (origin, folder)
}

/// The names a manifest gives, each once. A dependency that starts with `/` is a constraint on the
/// server (`ResourceManagerConstraintsComponent.cpp`), not a resource.
fn links(names: Vec<String>, constraints: bool) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for name in names {
        let fits = !name.is_empty() && name.len() <= MAX_NAME && !(constraints && name.starts_with('/'));
        if fits && kept.len() < MAX_LINKS && !kept.contains(&name) {
            kept.push(name);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRIDGE: &str = "sys_k3jx7qmw2pza";
    const ROOT: &str = "C:/srv/server-data/resources/";

    fn reported(name: &str, state: &str, folder: &str, needs: &[&str]) -> Reported {
        Reported {
            name: name.to_owned(),
            state: state.to_owned(),
            path: Some(format!("{ROOT}/{folder}{name}")),
            version: Some("1.0.0".to_owned()),
            needs: needs.iter().map(|&need| need.to_owned()).collect(),
            provides: Vec::new(),
        }
    }

    /// A framework, two resources that depend on it, one that depends on one of those, and one
    /// that depends on nothing.
    fn listed() -> Book {
        let mut book = Book::default();
        let resources = vec![
            reported("ox_lib", "started", "[core]/", &[]),
            reported(
                "ox_inventory",
                "started",
                "[core]/[ox]/",
                &["/onesync", "ox_lib", "oxmysql"],
            ),
            reported("ox_target", "started", "[core]/[ox]/", &["ox_lib"]),
            reported("shops", "started", "[jobs]/", &["ox_inventory"]),
            reported("oxmysql", "started", "[core]/", &[]),
            reported("taxi", "stopped", "[jobs]/", &["ox_lib"]),
            reported(BRIDGE, "started", "", &[]),
        ];
        assert!(book.page(1, 1, true, resources, BRIDGE, 1000));
        book
    }

    fn names(book: &Book) -> Vec<&str> {
        book.resources().iter().map(|known| known.name.as_str()).collect()
    }

    fn events(events: &[(&str, bool)]) -> Vec<(String, bool)> {
        events
            .iter()
            .map(|&(name, started)| (name.to_owned(), started))
            .collect()
    }

    fn started(names: &[&str]) -> Vec<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    #[test]
    fn a_list_is_kept_in_name_order_without_the_bridge_or_the_constraints() {
        let book = listed();

        assert!(book.listed());
        assert_eq!(
            names(&book),
            ["ox_inventory", "ox_lib", "ox_target", "oxmysql", "shops", "taxi"]
        );
        let inventory = book.find("ox_inventory").unwrap();
        assert_eq!(inventory.needs, ["ox_lib", "oxmysql"]);
        assert_eq!(
            (inventory.state, inventory.origin, inventory.folder.as_str()),
            (State::Started, Origin::Server, "[core]/[ox]")
        );
        assert_eq!(inventory.version.as_deref(), Some("1.0.0"));
        assert_eq!(book.find("taxi").unwrap().state, State::Stopped);
        assert_eq!(book.added(), None, "the first list of a run adds to nothing");
    }

    #[test]
    fn a_resource_lies_under_the_categories_that_end_its_path() {
        for (path, origin, folder) in [
            ("C:/srv/data/resources//chat", Origin::Server, ""),
            (
                "C:/srv/data/resources//[core]/[ox]/ox_lib",
                Origin::Server,
                "[core]/[ox]",
            ),
            (
                "C:\\srv\\data\\resources\\[maps]\\docks",
                Origin::Server,
                "[maps]",
            ),
            ("/home/fx/[servers]/main/resources/[a]/b", Origin::Server, "[a]"),
            ("C:/fx/38435/citizen/system_resources//monitor", Origin::Build, ""),
            ("/opt/cfx/citizen/system_resources/[x]/y", Origin::Build, "[x]"),
            ("relative", Origin::Server, ""),
            ("", Origin::Server, ""),
        ] {
            assert_eq!(place(path), (origin, folder.to_owned()), "{path}");
        }
    }

    #[test]
    fn pages_out_of_turn_or_of_another_list_are_dropped() {
        let mut book = Book::default();
        let page = |name: &str| vec![reported(name, "started", "", &[])];

        assert!(!book.page(1, 2, true, page("late"), BRIDGE, 0), "no first page");
        assert!(!book.page(1, 1, false, page("a"), BRIDGE, 0));
        assert!(
            !book.page(2, 1, false, page("x"), BRIDGE, 0),
            "a newer list begins"
        );
        assert!(
            !book.page(1, 2, true, page("b"), BRIDGE, 0),
            "the older one's next page"
        );
        assert!(!book.listed());
        assert!(!book.page(2, 3, true, page("z"), BRIDGE, 0), "page 2 never came");
        assert!(book.page(2, 2, true, page("y"), BRIDGE, 0));

        assert_eq!(names(&book), ["x", "y"]);
    }

    #[test]
    fn a_report_that_comes_between_two_pages_of_a_list_still_counts() {
        let mut book = Book::default();
        let page = |name: &str| vec![reported(name, "started", "", &[])];

        book.page(1, 1, false, page("a"), BRIDGE, 0);
        book.started(&started(&["b"]), &events(&[("a", false)]), 10);
        book.page(1, 2, true, page("b"), BRIDGE, 20);

        let state = |name: &str| book.find(name).unwrap().state;
        assert_eq!((state("a"), state("b")), (State::Stopped, State::Started));
    }

    #[test]
    fn a_later_list_says_what_it_added_and_keeps_what_happened_to_the_others() {
        let mut book = listed();
        book.started(
            &started(&["ox_inventory", "ox_lib", "ox_target", "oxmysql", "shops"]),
            &events(&[("oxmysql", false), ("oxmysql", true)]),
            2000,
        );

        let again = vec![
            reported("ox_lib", "started", "[core]/", &[]),
            reported("oxmysql", "started", "[core]/", &[]),
            reported("bank_heist", "stopped", "[jobs]/", &[]),
        ];
        assert!(book.page(2, 1, true, again, BRIDGE, 3000));

        assert_eq!(
            book.added(),
            Some(&Added {
                at_ms: 3000,
                names: vec!["bank_heist".to_owned()]
            })
        );
        assert_eq!(names(&book), ["bank_heist", "ox_lib", "oxmysql"]);
        assert_eq!(
            book.find("oxmysql").unwrap().turn,
            Some(Turn {
                at_ms: 2000,
                kind: TurnKind::Restarted,
                with: None
            })
        );
    }

    #[test]
    fn the_started_resources_that_depend_on_one_are_found_through_the_others() {
        let mut book = listed();

        let dependants = book.dependants();
        assert_eq!(dependants.of("ox_lib"), ["ox_inventory", "ox_target", "shops"]);
        assert_eq!(dependants.of("ox_inventory"), ["shops"]);
        assert_eq!(dependants.of("oxmysql"), ["ox_inventory", "shops"]);
        assert_eq!(dependants.of("shops"), Vec::<String>::new());
        assert_eq!(dependants.of("unknown"), Vec::<String>::new());

        book.started(&started(&["ox_lib", "ox_target", "oxmysql"]), &[], 2000);
        assert_eq!(
            book.dependants().of("ox_lib"),
            ["ox_target"],
            "a stopped resource has nothing more to lose"
        );
    }

    #[test]
    fn a_dependency_is_found_by_a_name_another_resource_provides() {
        let mut book = Book::default();
        let mut provider = reported("oxmysql", "started", "", &[]);
        provider.provides = vec!["mysql-async".to_owned(), "ghmattimysql".to_owned()];
        let mut old = reported("mysql-async", "stopped", "", &[]);
        old.version = Some("  ".to_owned());
        let resources = vec![
            provider,
            old,
            reported("garage", "started", "", &["mysql-async"]),
            reported("bank", "started", "", &["ghmattimysql"]),
        ];
        book.page(1, 1, true, resources, BRIDGE, 0);

        assert_eq!(book.dependants().of("oxmysql"), ["bank", "garage"]);
        assert_eq!(book.dependants().of("mysql-async"), Vec::<String>::new());
        assert_eq!(book.find("mysql-async").unwrap().version, None);
    }

    #[test]
    fn a_report_of_started_resources_sets_every_state_and_what_happened() {
        let mut book = listed();

        let changed = book.started(
            &started(&[BRIDGE, "ox_lib", "oxmysql", "taxi"]),
            &events(&[
                ("shops", false),
                ("ox_inventory", false),
                ("ox_target", false),
                ("ox_lib", false),
                ("ox_lib", true),
                ("taxi", true),
            ]),
            5000,
        );

        assert!(changed);
        let turn = |name: &str| {
            let known = book.find(name).unwrap();
            let turn = known.turn.clone().unwrap();
            assert_eq!(turn.at_ms, 5000, "{name}");
            (known.state, turn.kind, turn.with)
        };
        let with_lib = Some("ox_lib".to_owned());
        assert_eq!(turn("ox_lib"), (State::Started, TurnKind::Restarted, None));
        assert_eq!(
            turn("ox_inventory"),
            (State::Stopped, TurnKind::Stopped, with_lib.clone())
        );
        assert_eq!(
            turn("ox_target"),
            (State::Stopped, TurnKind::Stopped, with_lib.clone())
        );
        assert_eq!(
            turn("shops"),
            (State::Stopped, TurnKind::Stopped, with_lib),
            "the root of what it depends on, not the resource in between"
        );
        assert_eq!(turn("taxi"), (State::Started, TurnKind::Started, None));
        assert_eq!(book.find("oxmysql").unwrap().turn, None);
        assert!(!book.started(&started(&[BRIDGE, "ox_lib", "oxmysql", "taxi"]), &[], 6000));
    }

    #[test]
    fn states_that_changed_without_an_event_still_say_what_happened() {
        let mut book = listed();

        book.started(&started(&["ox_lib", "taxi"]), &[], 7000);

        let kind = |name: &str| book.find(name).unwrap().turn.as_ref().map(|turn| turn.kind);
        assert_eq!(kind("oxmysql"), Some(TurnKind::Stopped));
        assert_eq!(kind("taxi"), Some(TurnKind::Started));
        assert_eq!(kind("ox_lib"), None);
    }

    #[test]
    fn a_run_that_has_not_listed_its_resources_takes_no_report() {
        let mut book = Book::default();

        assert!(!book.started(&started(&["chat"]), &events(&[("chat", true)]), 1));
        assert_eq!(book.resources(), []);
    }

    #[test]
    fn a_stop_then_a_start_of_one_resource_is_a_restart_wherever_they_stand() {
        let folded = turns(&events(&[
            ("a", false),
            ("b", false),
            ("a", true),
            ("c", true),
            ("c", false),
            ("b", true),
            ("b", false),
        ]));

        assert_eq!(
            folded,
            [
                ("a".to_owned(), TurnKind::Restarted),
                ("b".to_owned(), TurnKind::Stopped),
                ("c".to_owned(), TurnKind::Stopped)
            ]
        );
    }

    #[test]
    fn a_list_keeps_what_fits_and_says_when_it_left_some_out() {
        let mut book = Book::default();
        let mut long = reported(&"n".repeat(MAX_NAME + 1), "started", "", &[]);
        long.needs = vec!["x".to_owned()];
        let mut twice = reported("twice", "started", "", &["a", "a", "", "/server:4500"]);
        twice.version = Some("v".repeat(200));
        let resources = vec![long, twice.clone(), twice, reported("", "started", "", &[])];

        book.page(1, 1, true, resources, BRIDGE, 0);

        assert_eq!(names(&book), ["twice"]);
        assert!(book.truncated());
        let twice = book.find("twice").unwrap();
        assert_eq!(twice.needs, ["a"]);
        assert_eq!(twice.version.as_ref().map(String::len), Some(MAX_VERSION));
    }
}
