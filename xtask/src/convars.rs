//! `cargo xtask convars`: the catalog of FXServer's convars, read from the sources of citizenfx/fivem
//! at one commit and written to `crates/observer-domain/convars.json`. The sources are fetched with
//! git, a shallow and sparse checkout of the folders FXServer is built from; `--source` reads a
//! checkout already there instead.

mod cpp;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use observer_domain::convars::{
    Convar, ConvarCatalog, ConvarFlag, ConvarKind, ConvarPlatform, ConvarRange, ConvarSource, ConvarValue,
};

use self::cpp::{ConsoleType, Declaration, RateLimiter, Read};

const REPOSITORY: &str = "https://github.com/citizenfx/fivem";
/// FXServer 38435, the build the changelogs API recommended when the catalog was pinned.
const COMMIT: &str = "5c3065c261fb3ba82032fee6803f247198fec221";
const BUILD: u32 = 38435;
const OUTPUT: &str = "crates/observer-domain/convars.json";
/// `CoreRT`, which every component shares: the console and its own convars.
const CORE: &str = "code/client/citicore";
/// The components FXServer loads, as the `components.json` of its Windows build 38220 lists them,
/// but for `svadhesive`, whose sources are not public.
const COMPONENTS: &[&str] = &[
    "net-base",
    "net-packet",
    "net-tcp-server",
    "net-http-server",
    "citizen-server-net",
    "citizen-scripting-core",
    "citizen-scripting-lua",
    "conhost-server",
    "citizen-server-main",
    "citizen-server-instance",
    "citizen-server-impl",
    "citizen-server-state-fivesv",
    "citizen-server-state-rdr3sv",
    "citizen-devtools",
    "citizen-resources-core",
    "citizen-resources-metadata-lua",
    "vfs-core",
    "vfs-impl-server",
    "scripting-server",
    "citizen-scripting-mono",
    "citizen-scripting-mono-v2",
    "citizen-scripting-node",
    "voip-server-mumble",
    "http-client",
    "pool-sizes-state",
    "citizen-server-gui",
];
/// Components FXServer has, left out because Observer never runs them.
const LEFT_OUT: &[(&str, &str)] = &[
    (
        "citizen-server-monitor",
        "FXServer starts it in place of citizen-server-main only when its command line has no `+exec` \
         (server/launcher/src/Server.cpp); Observer always passes one",
    ),
    (
        "citizen-server-fxdk",
        "runs only with `-fxdk`, which Observer refuses",
    ),
];
/// What `KeyedRateLimiter.h` must still say for each `GetRateLimiter("key", …)` to declare
/// `rateLimiter_<key>_rate` and `rateLimiter_<key>_burst` as floats.
const RATE_LIMITER: (&str, [&str; 4]) = (
    "code/components/citizen-server-instance/include/KeyedRateLimiter.h",
    [
        "fmt::sprintf(\"rateLimiter_%s_rate\", key), ConVar_None, defaults.rate)",
        "fmt::sprintf(\"rateLimiter_%s_burst\", key), ConVar_None, defaults.burst)",
        "ConVar<float> rateVar;",
        "ConVar<float> burstVar;",
    ],
);
const EXTENSIONS: [&str; 4] = ["cpp", "h", "hpp", "cc"];

/// `cargo xtask convars [--commit <sha> --build <n>] [--source <checkout>]`.
pub fn run(root: &Path, args: &[String]) -> anyhow::Result<()> {
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let (commit, build) = match (value("--commit"), value("--build")) {
        (None, None) => (COMMIT.to_owned(), BUILD),
        (Some(commit), Some(build)) => (commit, build.parse().context("--build takes a build number")?),
        _ => bail!("--commit and --build go together: the build FXServer made from that commit"),
    };
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("--commit takes a full commit hash");
    }
    let checkout = match value("--source") {
        Some(source) => PathBuf::from(source),
        None => fetch(root, &commit)?,
    };
    let head = git(&checkout, &["rev-parse", "HEAD"])?;
    if head != commit {
        bail!("{} is at {head}, not {commit}", checkout.display());
    }
    let report = catalog(&checkout, &commit, build)?;
    for skipped in &report.skipped {
        eprintln!("left out, its name is not a literal: {skipped}");
    }
    for unknown in &report.unknown_conditions {
        eprintln!("kept, under an #if the generator cannot evaluate: {unknown}");
    }
    let mut json = serde_json::to_string_pretty(&report.catalog)?;
    json.push('\n');
    let output = root.join(OUTPUT);
    fs::write(&output, json).with_context(|| format!("cannot write {}", output.display()))?;
    let declared = report
        .catalog
        .convars
        .iter()
        .filter(|convar| convar.declared)
        .count();
    println!(
        "{} convars from {REPOSITORY} at {commit} (build {build}): {declared} declared, {} only read; wrote {OUTPUT}",
        report.catalog.convars.len(),
        report.catalog.convars.len() - declared,
    );
    Ok(())
}

/// A shallow, sparse checkout of `commit` under `target/fivem/`, fetched once.
fn fetch(root: &Path, commit: &str) -> anyhow::Result<PathBuf> {
    let checkout = root.join("target").join("fivem").join(commit);
    if checkout.join(".git").exists()
        && git(&checkout, &["rev-parse", "HEAD"]).is_ok_and(|head| head == commit)
    {
        return Ok(checkout);
    }
    fs::create_dir_all(&checkout)?;
    git(&checkout, &["init", "--quiet"])?;
    if git(&checkout, &["remote", "get-url", "origin"]).is_err() {
        git(&checkout, &["remote", "add", "origin", REPOSITORY])?;
    }
    git(&checkout, &["sparse-checkout", "init", "--cone"])?;
    let mut folders = vec!["sparse-checkout".to_owned(), "set".to_owned(), CORE.to_owned()];
    folders.extend(
        COMPONENTS
            .iter()
            .map(|component| format!("code/components/{component}")),
    );
    git(&checkout, &folders.iter().map(String::as_str).collect::<Vec<_>>())?;
    println!("fetching {REPOSITORY} at {commit}");
    git(
        &checkout,
        &[
            "fetch",
            "--quiet",
            "--depth",
            "1",
            "--filter=blob:none",
            "origin",
            commit,
        ],
    )?;
    git(
        &checkout,
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "FETCH_HEAD",
        ],
    )?;
    Ok(checkout)
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .context("cannot run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[derive(Debug)]
struct Report {
    catalog: ConvarCatalog,
    skipped: Vec<String>,
    unknown_conditions: Vec<String>,
}

/// The catalog of the checkout at `checkout`, which must be at `commit`.
fn catalog(checkout: &Path, commit: &str, build: u32) -> anyhow::Result<Report> {
    let config = fs::read_to_string(checkout.join("code/components/config.lua"))
        .context("cannot read code/components/config.lua")?;
    let blocks = server_components(&config);
    for component in &blocks.server {
        let known = COMPONENTS.contains(&component.as_str())
            || LEFT_OUT.iter().any(|(left_out, _)| left_out == component);
        if !known {
            eprintln!("config.lua builds {component} for the server, which the generator does not read");
        }
    }
    let rate_limiter = fs::read_to_string(checkout.join(RATE_LIMITER.0))
        .with_context(|| format!("cannot read {}", RATE_LIMITER.0))?;
    for needed in RATE_LIMITER.1 {
        if !rate_limiter.contains(needed) {
            bail!(
                "{} no longer says `{needed}`: the rate limiters' convars changed",
                RATE_LIMITER.0
            );
        }
    }

    let mut files = Vec::new();
    files.extend(source_files(checkout, CORE, None)?);
    for component in COMPONENTS {
        let folder = format!("code/components/{component}");
        if !checkout.join(&folder).is_dir() {
            bail!("{folder} is not in the checkout");
        }
        let platform = blocks
            .windows_only
            .contains(*component)
            .then_some(ConvarPlatform::Windows);
        files.extend(source_files(checkout, &folder, platform)?);
    }

    let mut collected = Collected::default();
    for (path, platform) in files {
        let text = String::from_utf8_lossy(&fs::read(checkout.join(&path))?).into_owned();
        let (findings, limiters) = cpp::read(&path, &text);
        let at = |line| ConvarSource {
            file: path.clone(),
            line,
        };
        collected.types.extend(findings.types);
        for declaration in findings.declarations {
            let platform = declaration.platform.or(platform);
            collected
                .declarations
                .push((at(declaration.line), platform, declaration));
        }
        for read in findings.reads {
            let platform = read.platform.or(platform);
            collected.reads.push((at(read.line), platform, read));
        }
        for limiter in limiters {
            let platform = limiter.platform.or(platform);
            collected.limiters.push((at(limiter.line), platform, limiter));
        }
        collected.skipped.extend(
            findings
                .skipped
                .into_iter()
                .map(|skipped| format!("{path}:{skipped}")),
        );
        collected.unknown.extend(
            findings
                .unknown_conditions
                .into_iter()
                .map(|line| format!("{path}:{line}")),
        );
    }
    Ok(Report {
        catalog: ConvarCatalog {
            repository: REPOSITORY.to_owned(),
            commit: commit.to_owned(),
            build,
            convars: collected.convars(),
        },
        skipped: collected.skipped,
        unknown_conditions: collected.unknown,
    })
}

/// The C++ files of `folder`, as paths from the checkout's root with `/`, sorted.
fn source_files(
    checkout: &Path,
    folder: &str,
    platform: Option<ConvarPlatform>,
) -> anyhow::Result<Vec<(String, Option<ConvarPlatform>)>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(checkout.join(folder)).sort_by_file_name() {
        let entry = entry?;
        let path = entry.path();
        let wanted = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| EXTENSIONS.contains(&extension));
        if !entry.file_type().is_file() || !wanted {
            continue;
        }
        let relative = path.strip_prefix(checkout)?;
        let name = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        files.push((name, platform));
    }
    Ok(files)
}

/// What `config.lua` builds for the server, and which of those only on Windows.
#[derive(Debug, Default, PartialEq, Eq)]
struct ServerBlocks {
    server: Vec<String>,
    windows_only: HashSet<String>,
}

/// Reads the `if _OPTIONS['game'] == 'server' then` block of `config.lua`, and the
/// `if os.istarget('windows') then` blocks inside it.
fn server_components(config: &str) -> ServerBlocks {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Block {
        Server,
        Windows,
        Other,
    }
    let mut blocks = ServerBlocks::default();
    let mut stack: Vec<Block> = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with("if ") && line.ends_with("then") {
            let block = if line.contains("_OPTIONS['game'] == 'server'") {
                Block::Server
            } else if line.contains("os.istarget('windows')") {
                Block::Windows
            } else {
                Block::Other
            };
            stack.push(block);
        } else if line == "else" || line.starts_with("elseif ") {
            if let Some(top) = stack.last_mut() {
                *top = Block::Other;
            }
        } else if line == "end" {
            stack.pop();
        } else if let Some(name) = line
            .strip_prefix("component '")
            .and_then(|rest| rest.strip_suffix('\''))
            && stack.contains(&Block::Server)
        {
            blocks.server.push(name.to_owned());
            let server = stack
                .iter()
                .position(|block| *block == Block::Server)
                .unwrap_or(0);
            if stack[server..].contains(&Block::Windows) {
                blocks.windows_only.insert(name.to_owned());
            }
        }
    }
    blocks
}

/// Where the sources say something, and the only platform they say it on.
type Site = (ConvarSource, Option<ConvarPlatform>);

#[derive(Default)]
struct Collected {
    types: HashMap<String, ConsoleType>,
    declarations: Vec<(ConvarSource, Option<ConvarPlatform>, Declaration)>,
    reads: Vec<(ConvarSource, Option<ConvarPlatform>, Read)>,
    limiters: Vec<(ConvarSource, Option<ConvarPlatform>, RateLimiter)>,
    skipped: Vec<String>,
    unknown: Vec<String>,
}

/// One declaration, its type known.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Resolved {
    name: String,
    kind: Option<ConvarKind>,
    cpp_type: Option<String>,
    default: Option<String>,
    default_expression: Option<String>,
    values: Vec<ConvarValue>,
    open: bool,
    range: Option<ConvarRange>,
    flags: Option<Vec<ConvarFlag>>,
    platform: Option<ConvarPlatform>,
    description: Option<String>,
    source: ConvarSource,
}

impl Collected {
    fn convars(&self) -> Vec<Convar> {
        let mut by_name: Vec<(String, Vec<Resolved>)> = Vec::new();
        let mut add = |resolved: Resolved| {
            let key = resolved.name.to_ascii_lowercase();
            match by_name.iter_mut().find(|(known, _)| *known == key) {
                Some((_, list)) => list.push(resolved),
                None => by_name.push((key, vec![resolved])),
            }
        };
        for (source, platform, declaration) in &self.declarations {
            add(self.resolve(source, *platform, declaration));
        }
        for (source, platform, limiter) in &self.limiters {
            for (suffix, default) in [("rate", &limiter.rate), ("burst", &limiter.burst)] {
                add(Resolved {
                    name: format!("rateLimiter_{}_{suffix}", limiter.key),
                    kind: Some(ConvarKind::Float),
                    cpp_type: Some("float".to_owned()),
                    default: Some(default.clone()),
                    default_expression: None,
                    values: Vec::new(),
                    open: false,
                    range: None,
                    flags: Some(Vec::new()),
                    platform: *platform,
                    description: None,
                    source: source.clone(),
                });
            }
        }
        let mut convars: Vec<Convar> = by_name.into_iter().map(|(_, list)| merge(&list)).collect();

        let declared: HashSet<String> = convars
            .iter()
            .map(|convar| convar.name.to_ascii_lowercase())
            .collect();
        let mut read: Vec<(String, Vec<Site>)> = Vec::new();
        for (source, platform, entry) in &self.reads {
            let key = entry.name.to_ascii_lowercase();
            if declared.contains(&key) {
                continue;
            }
            match read.iter_mut().find(|(name, _)| name.eq_ignore_ascii_case(&key)) {
                Some((_, sites)) => sites.push((source.clone(), *platform)),
                None => read.push((entry.name.clone(), vec![(source.clone(), *platform)])),
            }
        }
        for (name, sites) in read {
            let platforms: Vec<Option<ConvarPlatform>> =
                sites.iter().map(|(_, platform)| *platform).collect();
            convars.push(Convar {
                name,
                kind: None,
                cpp_type: None,
                default: None,
                default_expression: None,
                values: Vec::new(),
                open: false,
                range: None,
                flags: None,
                platform: same(&platforms).flatten(),
                declared: false,
                description: None,
                sources: dedup(sites.into_iter().map(|(source, _)| source)),
            });
        }
        convars.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        convars
    }

    /// Gives a declaration its kind, its printed default and its values, from its C++ type and
    /// the console types the sources specialize.
    fn resolve(
        &self,
        source: &ConvarSource,
        platform: Option<ConvarPlatform>,
        declaration: &Declaration,
    ) -> Resolved {
        let cpp_type = declaration.cpp_type.as_str();
        let console = self.types.get(cpp::last_segment(cpp_type));
        let kind = match cpp_type {
            "bool" => Some(ConvarKind::Bool),
            "int" | "int32_t" | "uint32_t" | "int64_t" | "uint64_t" | "uint16_t" | "int16_t" | "uint8_t"
            | "size_t" => Some(ConvarKind::Int),
            "float" | "double" => Some(ConvarKind::Float),
            "std::string" | "const char*" | "std::string_view" => Some(ConvarKind::String),
            _ => console
                .filter(|console| !console.parsed.is_empty())
                .map(|_| ConvarKind::Enum),
        };
        let written = declaration.default.as_deref();
        let printed = written.and_then(|written| match kind {
            Some(ConvarKind::Bool) => matches!(written, "true" | "false").then(|| written.to_owned()),
            Some(ConvarKind::Int) => cpp::integer_literal(written).map(|value| value.to_string()),
            Some(ConvarKind::Float) => cpp::float_literal(written).map(cpp::print_float),
            Some(ConvarKind::String) => cpp::string_literal(written),
            Some(ConvarKind::Enum) => console.and_then(|console| enum_default(console, written)),
            None => None,
        });
        let values = match (kind, console) {
            (Some(ConvarKind::Enum), Some(console)) => console
                .parsed
                .iter()
                .map(|(value, read)| ConvarValue {
                    value: value.clone(),
                    aliases: read
                        .iter()
                        .filter(|alias| !alias.eq_ignore_ascii_case(value))
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                })
                .collect(),
            _ => Vec::new(),
        };
        Resolved {
            name: declaration.name.clone(),
            kind,
            cpp_type: Some(cpp_type.to_owned()),
            default_expression: if printed.is_none() {
                written.map(str::to_owned)
            } else {
                None
            },
            default: printed,
            open: kind == Some(ConvarKind::Enum) && console.is_some_and(|console| console.numbers),
            values,
            range: declaration.range,
            flags: declaration.flags.clone(),
            platform,
            description: declaration.description.clone(),
            source: source.clone(),
        }
    }
}

/// `fx::OneSyncState::On` as `Unparse` prints it.
fn enum_default(console: &ConsoleType, written: &str) -> Option<String> {
    if !written.contains("::") {
        return None;
    }
    console.printed.get(cpp::last_segment(written)).cloned()
}

/// One convar from its declarations: what they all say, and nothing where they disagree.
fn merge(list: &[Resolved]) -> Convar {
    let first = &list[0];
    let agreed = |field: fn(&Resolved) -> String| list.iter().all(|other| field(other) == field(first));
    let defaults_agree = agreed(|r| format!("{:?}{:?}", r.default, r.default_expression));
    let kind_agrees = agreed(|r| format!("{:?}{:?}", r.kind, r.cpp_type));
    let values_agree = agreed(|r| format!("{:?}{}", r.values, r.open));
    let platforms: Vec<Option<ConvarPlatform>> = list.iter().map(|r| r.platform).collect();
    Convar {
        name: first.name.clone(),
        kind: if kind_agrees { first.kind } else { None },
        cpp_type: if kind_agrees { first.cpp_type.clone() } else { None },
        default: if defaults_agree {
            first.default.clone()
        } else {
            None
        },
        default_expression: if defaults_agree {
            first.default_expression.clone()
        } else {
            None
        },
        values: if kind_agrees && values_agree {
            first.values.clone()
        } else {
            Vec::new()
        },
        open: kind_agrees && values_agree && first.open,
        range: if agreed(|r| format!("{:?}", r.range)) {
            first.range
        } else {
            None
        },
        flags: if agreed(|r| format!("{:?}", r.flags)) {
            first.flags.clone()
        } else {
            None
        },
        platform: same(&platforms).flatten(),
        declared: true,
        description: list.iter().find_map(|r| r.description.clone()),
        sources: dedup(list.iter().map(|r| r.source.clone())),
    }
}

fn same<T: PartialEq + Copy>(list: &[T]) -> Option<T> {
    let first = *list.first()?;
    list.iter().all(|other| *other == first).then_some(first)
}

fn dedup(sources: impl Iterator<Item = ConvarSource>) -> Vec<ConvarSource> {
    let mut seen = Vec::new();
    for source in sources {
        if !seen.contains(&source) {
            seen.push(source);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\
component 'http-client'
component 'pool-sizes-state'

if _OPTIONS['game'] == 'server' then
	component 'citizen-server-main'

	if os.istarget('windows') then
		component 'citizen-server-gui'
		component 'citizen-server-fxdk'
	end
	component 'citizen-server-impl'
else
	component 'citizen-devtools'
	component 'nui-core'
end

component 'net-base'
";

    #[test]
    fn config_lua_names_the_server_components_and_the_windows_ones() {
        let blocks = server_components(CONFIG);

        assert_eq!(
            blocks.server,
            [
                "citizen-server-main",
                "citizen-server-gui",
                "citizen-server-fxdk",
                "citizen-server-impl"
            ]
        );
        assert_eq!(
            blocks.windows_only,
            HashSet::from(["citizen-server-gui".to_owned(), "citizen-server-fxdk".to_owned()])
        );
    }

    /// A checkout made of the files the generator reads, without git.
    fn checkout(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut all: Vec<(String, String)> = vec![
            ("code/components/config.lua".to_owned(), CONFIG.to_owned()),
            (RATE_LIMITER.0.to_owned(), RATE_LIMITER.1.join("\n")),
        ];
        all.extend(
            files
                .iter()
                .map(|(path, text)| ((*path).to_owned(), (*text).to_owned())),
        );
        for component in COMPONENTS {
            fs::create_dir_all(dir.path().join("code/components").join(component)).unwrap();
        }
        fs::create_dir_all(dir.path().join(CORE)).unwrap();
        for (path, text) in all {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    fn convar<'a>(catalog: &'a ConvarCatalog, name: &str) -> &'a Convar {
        catalog.find(name).unwrap_or_else(|| panic!("{name} is missing"))
    }

    const ONESYNC_TYPE: &str = r#"
struct ConsoleArgumentType<fx::OneSyncState>
{
	static std::string Unparse(const fx::OneSyncState& input)
	{
		switch (input)
		{
			case fx::OneSyncState::Off: return "off";
			case fx::OneSyncState::On: return "on";
		}
	}
	static bool Parse(const std::string& input, fx::OneSyncState* out)
	{
		if (_stricmp(input.c_str(), "on") == 0 || _stricmp(input.c_str(), "true") == 0)
		{
			*out = fx::OneSyncState::On;
			return true;
		}
		else if (_stricmp(input.c_str(), "off") == 0)
		{
			*out = fx::OneSyncState::Off;
			return true;
		}
		return false;
	}
};
"#;

    const STATE: &str = r#"
		g_oneSyncVar = instance->AddVariable<fx::OneSyncState>("onesync", ConVar_Internal, fx::OneSyncState::On);
		auto pureVar = instance->AddVariable<int>("sv_pureLevel", ConVar_ServerInfo, 0);
		auto hostname = instance->AddVariable<std::string>("sv_hostname", ConVar_ServerInfo, "default FXServer");
		auto master = instance->AddVariable<std::string>("sv_master1", ConVar_None, kDefaultServerList);
		auto limiter = store->GetRateLimiter("netEvent", fx::RateLimiterDefaults{ 50.f, 200.f });
		auto token = ctx->FindEntryRaw("sv_licenseKeyToken");
		auto again = ctx->FindEntryRaw("sv_hostname");
"#;

    #[test]
    fn a_checkout_gives_a_sorted_catalog_with_kinds_defaults_and_values() {
        let dir = checkout(&[
            (
                "code/components/citizen-server-impl/include/OneSyncVars.h",
                ONESYNC_TYPE,
            ),
            ("code/components/citizen-server-impl/src/State.cpp", STATE),
            (
                "code/components/citizen-server-gui/src/ServerGui.cpp",
                r#"m_disableVar = m_instance->AddVariable<bool>("svgui_disable", ConVar_None, true);"#,
            ),
            (
                "code/components/citizen-server-monitor/src/MonitorInstance.cpp",
                r#"auto monitorVar = AddVariable<bool>("monitorMode", ConVar_None, true);"#,
            ),
        ]);

        let report = catalog(dir.path(), COMMIT, BUILD).unwrap();
        let catalog = &report.catalog;

        let names: Vec<&str> = catalog.convars.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "onesync",
                "rateLimiter_netEvent_burst",
                "rateLimiter_netEvent_rate",
                "sv_hostname",
                "sv_licenseKeyToken",
                "sv_master1",
                "sv_pureLevel",
                "svgui_disable"
            ]
        );
        let onesync = convar(catalog, "onesync");
        assert_eq!(onesync.kind, Some(ConvarKind::Enum));
        assert_eq!(onesync.default.as_deref(), Some("on"));
        assert_eq!(onesync.flags, Some(vec![ConvarFlag::Internal]));
        assert_eq!(
            onesync.values,
            [
                ConvarValue {
                    value: "on".to_owned(),
                    aliases: vec!["true".to_owned()]
                },
                ConvarValue {
                    value: "off".to_owned(),
                    aliases: vec![]
                },
            ]
        );
        assert_eq!(
            onesync.sources,
            [ConvarSource {
                file: "code/components/citizen-server-impl/src/State.cpp".to_owned(),
                line: 2
            }]
        );
        let pure = convar(catalog, "sv_pureLevel");
        assert_eq!(
            (pure.kind, pure.default.as_deref()),
            (Some(ConvarKind::Int), Some("0"))
        );
        assert_eq!(pure.values, Vec::<ConvarValue>::new());
        assert_eq!(
            convar(catalog, "sv_hostname").default.as_deref(),
            Some("default FXServer")
        );
        let master = convar(catalog, "sv_master1");
        assert_eq!(master.default, None);
        assert_eq!(master.default_expression.as_deref(), Some("kDefaultServerList"));
        assert_eq!(
            convar(catalog, "rateLimiter_netEvent_rate").default.as_deref(),
            Some("50.000000")
        );
        let token = convar(catalog, "sv_licenseKeyToken");
        assert!(!token.declared);
        assert_eq!(token.kind, None);
        assert_eq!(
            convar(catalog, "svgui_disable").platform,
            Some(ConvarPlatform::Windows)
        );
        assert_eq!(convar(catalog, "onesync").platform, None);
    }

    #[test]
    fn declarations_that_disagree_leave_the_field_unknown_and_keep_every_source() {
        let dir = checkout(&[
            (
                "code/components/citizen-scripting-core/src/A.cpp",
                r#"static ConVar<bool> strict("sv_stateBagStrictMode", ConVar_Replicated, false);"#,
            ),
            (
                "code/components/citizen-server-impl/src/B.cpp",
                r#"m_var = instance->AddVariable<bool>("sv_stateBagStrictMode", ConVar_None, m_stateBagStrictMode);"#,
            ),
        ]);

        let report = catalog(dir.path(), COMMIT, BUILD).unwrap();

        let strict = convar(&report.catalog, "sv_stateBagStrictMode");
        assert_eq!(strict.kind, Some(ConvarKind::Bool));
        assert_eq!(strict.flags, None);
        assert_eq!(strict.default, None);
        assert_eq!(strict.default_expression, None);
        assert_eq!(strict.sources.len(), 2);
    }

    #[test]
    fn a_rate_limiter_header_that_changed_stops_the_generator() {
        let dir = checkout(&[]);
        fs::write(dir.path().join(RATE_LIMITER.0), "ConVar<double> rateVar;").unwrap();

        let error = catalog(dir.path(), COMMIT, BUILD).unwrap_err();

        assert!(
            error.to_string().contains("the rate limiters' convars changed"),
            "{error}"
        );
    }
}
