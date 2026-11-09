//! The convars FXServer declares, as `cargo xtask convars` reads them in the sources of
//! citizenfx/fivem at one commit, and those the server's cfg sets. What the sources do not say
//! stays unknown: a field is null rather than guessed. An owner may still set a convar the catalog
//! does not know.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::server_data::CfgFileProblem;

/// The catalog, as `cargo xtask convars` writes it; `GET /api/convars` serves it as it is.
pub const CATALOG_JSON: &str = include_str!("../convars.json");

static CATALOG: LazyLock<ConvarCatalog> = LazyLock::new(|| {
    serde_json::from_str(CATALOG_JSON).expect("the generator writes the catalog from these very types")
});

/// What `GET /api/convars` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ConvarCatalog {
    /// The repository read: `https://github.com/citizenfx/fivem`.
    pub repository: String,
    pub commit: String,
    /// The FXServer build made from that commit.
    pub build: u32,
    /// Sorted by name, without regard to case.
    pub convars: Vec<Convar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Convar {
    /// As the first declaration spells it. FXServer reads convar names without regard to case.
    pub name: String,
    /// Null for a convar the sources only read, or declare with different types.
    pub kind: Option<ConvarKind>,
    /// The C++ type of its declaration: `uint16_t`, `fx::OneSyncState`.
    pub cpp_type: Option<String>,
    /// The default as FXServer prints it, when the sources write it as a literal.
    pub default: Option<String>,
    /// The default as the sources write it when it is not a literal:
    /// `xbr::GetMandatedDefaultGTA5BuildString()`.
    pub default_expression: Option<String>,
    /// The values its type reads, each with the other spellings it takes for it.
    pub values: Vec<ConvarValue>,
    /// The type also reads values outside `values`: any build number, for a game build.
    pub open: bool,
    /// The bounds the sources set on it.
    pub range: Option<ConvarRange>,
    /// Null when unknown: its declarations disagree, or it has none.
    pub flags: Option<Vec<ConvarFlag>>,
    /// The only platform whose FXServer declares it.
    pub platform: Option<ConvarPlatform>,
    /// False for a convar the sources read but never declare: it exists once something sets it.
    pub declared: bool,
    /// The comment written above or beside its declaration, as the sources have it.
    pub description: Option<String>,
    /// Its declarations, or the places that read it when it has none.
    pub sources: Vec<ConvarSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConvarKind {
    Bool,
    Int,
    Float,
    String,
    /// One of `values`, or, when `open`, another value its type reads.
    Enum,
}

/// The flags of a declaration, from `ConsoleVariableFlags` in `Console.Variables.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConvarFlag {
    Archive,
    /// Public: listed in the server's information.
    ServerInfo,
    /// Sent to the clients, which may not change it.
    Replicated,
    /// Taken only from the command line or the cfg FXServer starts with.
    ReadOnly,
    UserPref,
    /// No `set` command changes it once declared.
    Internal,
    /// Hidden from scripts.
    ScriptRestricted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConvarPlatform {
    Windows,
    Linux,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ConvarValue {
    /// As FXServer prints it.
    pub value: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ConvarRange {
    pub min: i32,
    pub max: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ConvarSource {
    /// From the repository's root: `code/components/citizen-server-impl/src/GameServer.cpp`.
    pub file: String,
    pub line: u32,
}

/// The catalog this build of Observer carries.
pub fn catalog() -> &'static ConvarCatalog {
    &CATALOG
}

impl ConvarCatalog {
    pub fn find(&self, name: &str) -> Option<&Convar> {
        self.convars
            .iter()
            .find(|convar| convar.name.eq_ignore_ascii_case(name))
    }
}

/// `GET /api/servers/default/launch/cfg-convars`: what the launch's cfg and the files it runs
/// set, in the order FXServer runs them. FXServer applies the command line's `+set` first and the
/// cfg after, so a convar set here keeps the cfg's last value whatever the launch settings say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgConvars {
    pub cfg_file: String,
    /// Why the cfg could not be read; the list is then empty.
    pub problem: Option<CfgFileProblem>,
    pub convars: Vec<CfgConvar>,
    /// False when a file the chain runs was left unread, or the list reached its limit: a convar
    /// may be set where Observer did not look.
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgConvar {
    /// As the cfg spells it.
    pub name: String,
    /// Null when it holds a secret, which never leaves Observer. Secrets inside other values are
    /// masked.
    pub value: Option<String>,
    pub command: CfgSetCommand,
    /// The file of the chain, named as the setup's report names it.
    pub file: String,
    pub line: u32,
}

/// How the cfg sets a convar: `set` and its kin, or its name alone followed by the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CfgSetCommand {
    Set,
    Sets,
    Setr,
    Seta,
    Bare,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_catalog_reads_and_is_sorted_by_name() {
        let catalog = catalog();

        assert_ne!(catalog.convars.len(), 0);
        assert_eq!(catalog.commit.len(), 40);
        let names: Vec<String> = catalog
            .convars
            .iter()
            .map(|convar| convar.name.to_ascii_lowercase())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            names, sorted,
            "sorted by name, each name once without regard to case"
        );
    }

    #[test]
    fn every_convar_names_where_the_sources_have_it() {
        for convar in &catalog().convars {
            assert!(!convar.sources.is_empty(), "{}", convar.name);
            assert!(
                convar
                    .sources
                    .iter()
                    .all(|source| source.file.starts_with("code/") && source.line > 0),
                "{}",
                convar.name
            );
            assert!(convar.declared || convar.kind.is_none(), "{}", convar.name);
            assert!(
                convar.values.is_empty() || convar.kind == Some(ConvarKind::Enum),
                "{}",
                convar.name
            );
        }
    }

    #[test]
    fn a_name_is_found_without_regard_to_case() {
        let catalog = catalog();

        let onesync = catalog.find("OneSync").unwrap();

        assert_eq!(onesync.name, "onesync");
        assert_eq!(onesync.kind, Some(ConvarKind::Enum));
        assert!(catalog.find("observer_not_a_convar").is_none());
    }
}
