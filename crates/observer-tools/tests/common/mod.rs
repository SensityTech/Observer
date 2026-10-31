//! Synthetic servers in temporary folders: manifests, a cfg, `.ytyp` files built byte by byte by
//! `observer_gamefiles::testing`, model-info files written as text. No game file is copied.

// Each test file uses some of these helpers; all are test code, though not in `#[test]` functions.
#![allow(dead_code, clippy::unwrap_used)]

pub mod fixes;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use observer_domain::tools::{ArchetypeGroup, GroupAction, LeaveReason, ToolSettings};
use observer_gamefiles::hash::name_hash;
use observer_gamefiles::testing::{ArchetypeSpec, YtypBuilder};
use observer_tools::archetypes::{self, Analysis};
use observer_tools::resources::{ServerIndex, Starts};
use tempfile::TempDir;

/// The manifest of a resource FXServer starts on a GTA V server.
pub const MANIFEST: &str = "fx_version 'cerulean'\ngame 'gta5'\n";

pub struct Server {
    dir: TempDir,
}

impl Server {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("resources")).unwrap();
        Self { dir }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Writes `bytes` at `path`, relative to the server folder, with its folders.
    pub fn write(&self, path: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let full = self.root().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, bytes).unwrap();
        full
    }

    /// A resource at `resources/<path>` with `fxmanifest.lua` holding the usual header then `extra`.
    pub fn resource(&self, path: &str, extra: &str) -> PathBuf {
        let folder = self.root().join("resources").join(path);
        self.write(
            &format!("resources/{path}/fxmanifest.lua"),
            format!("{MANIFEST}{extra}"),
        );
        folder
    }

    /// A `.ytyp` at `resources/<path>` defining a prop for each name.
    pub fn ytyp(&self, path: &str, names: &[&str]) -> PathBuf {
        let stem = Path::new(path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        self.write(
            &format!("resources/{path}"),
            observer_gamefiles::testing::ytyp(&stem, names),
        )
    }

    /// A `.ytyp` at `resources/<path>` with these archetypes.
    pub fn ytyp_with(&self, path: &str, specs: Vec<ArchetypeSpec>) -> PathBuf {
        let stem = Path::new(path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let builder = specs
            .into_iter()
            .fold(YtypBuilder::new(&stem), YtypBuilder::archetype);
        self.write(&format!("resources/{path}"), builder.build())
    }

    pub fn cfg(&self, text: &str) {
        self.write("server.cfg", text);
    }

    pub fn starts(&self) -> Starts {
        let scan = observer_tools::resources::scan_cfg(self.root(), "server.cfg", None).unwrap();
        Starts::from_scan(&scan, None)
    }

    pub fn index(&self) -> ServerIndex {
        ServerIndex::build(self.root(), &self.starts(), &AtomicBool::new(false)).unwrap()
    }

    pub fn analyse(&self, settings: &ToolSettings) -> Analysis {
        archetypes::analyse(&self.index(), settings, &AtomicBool::new(false)).unwrap()
    }

    /// The names of the resources started, in order.
    pub fn started(&self) -> Vec<String> {
        let index = self.index();
        index
            .started()
            .iter()
            .map(|&at| index.resources()[at].name.clone())
            .collect()
    }
}

/// A weapon model-info file defining an item per model name, each with `extra` inside.
pub fn weapons(items: &[(&str, &str)]) -> String {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CWeaponModelInfo__InitDataList>\n  <InitDatas>\n",
    );
    for (name, extra) in items {
        write!(
            text,
            "    <Item>\n      <modelName>{name}</modelName>\n      <txdName>{name}</txdName>\n{extra}    </Item>\n"
        )
        .unwrap();
    }
    text.push_str("  </InitDatas>\n</CWeaponModelInfo__InitDataList>\n");
    text
}

/// A `.ymf` as the client reads one, big-endian PSO, naming the stems' hashes.
pub fn ymf(stems: &[&str]) -> Vec<u8> {
    let mut bytes = b"PSIN\0\0\0\x40".to_vec();
    bytes.extend_from_slice(&[0x70; 8]);
    for stem in stems {
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&name_hash(stem).to_be_bytes());
    }
    bytes
}

/// The group of the archetype `name`.
pub fn group<'a>(analysis: &'a Analysis, name: &str) -> &'a ArchetypeGroup {
    let hash = name_hash(name);
    analysis
        .report
        .groups
        .iter()
        .find(|group| group.hash == hash)
        .unwrap_or_else(|| panic!("no group for {name}: {:#?}", analysis.report.groups))
}

/// The reasons a group is left, none when it is fixed.
pub fn reasons(group: &ArchetypeGroup) -> Vec<LeaveReason> {
    match &group.action {
        GroupAction::Fix { .. } => Vec::new(),
        GroupAction::Leave { reasons } => reasons.clone(),
    }
}

pub fn fixed(group: &ArchetypeGroup) -> bool {
    matches!(group.action, GroupAction::Fix { .. })
}
