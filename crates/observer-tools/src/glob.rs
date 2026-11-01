//! FiveM's globs, the way `ResourceMetaDataComponent.cpp` resolves the paths a manifest names:
//! FXServer against the resource's folder for `file`, `client_script`, `shared_script` and `map`,
//! the client against the files shipped in the resource's packfile for `data_file`. Which data
//! files a client loads, and in which order, follows from these matches, so they are reproduced
//! with FiveM's quirks rather than with the rules of any other glob:
//!
//! - the pattern is put under the resource's folder and normalised by FiveM's `path_normalize`:
//!   `\` becomes `/`, `./` and repeated separators go, `..` climbs, and each run of characters
//!   between blanks is normalised on its own, so a blank after a `/` drops that `/`;
//! - the client looks the folders written before the first `*` up the way the packfile looks a path
//!   up, the exact name first, then the first name equal to it in ASCII case; FXServer opens them
//!   through its system, which ignores case on Windows and keeps it on Linux ([`Folders::SERVER`]);
//!   every name compared against a pattern goes through a regular expression, byte for byte and
//!   case included, in which `*` takes any bytes and `?` one, both within one name, and `{n}`,
//!   `{n,}` or `{n,m}` repeat the byte before them, since FiveM leaves braces unescaped (any other
//!   brace makes its regex throw, and matches nothing here);
//! - a `**` that starts a name (`a/**`, `**.ytyp`, `a/**/b`) also tries the rest of the pattern in
//!   every folder below, at every depth; elsewhere (`a**`) it is two `*`;
//! - a `*` in a folder name loses the byte before the next `/`: FiveM lists those folders with that
//!   byte cut off, so `a/*/b` matches nothing and `a/x*y/b` looks in the folders `x*` matches;
//! - the matches come sorted in byte order, duplicates dropped, as FiveM gathers them in a set.

use std::collections::{BTreeMap, HashSet};

/// The resource's folder in the patterns resolved here, where FiveM writes the resource's path.
const ROOT: &[u8] = b"R";
const ROOT_SLASH: &[u8] = b"R/";
/// `path_normalize` writes into a 16 KiB buffer: FiveM overflows it with a longer path, which
/// matches nothing here.
const MAX_PATTERN_BYTES: usize = 0x3FFF;
/// No file system names a file or folder with more bytes; a longer name matches no pattern.
const MAX_NAME_BYTES: usize = 1024;
/// The work one expansion may do, in bytes of patterns queued and names compared: a `**` over the
/// largest resource takes a few megabytes, and this bounds what a hostile pattern costs.
const MAX_WORK: usize = 16 * 1024 * 1024;

/// How the folders a pattern names are found among the files: those written before its first `*`,
/// and those written after a wildcard, below the folders it matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folders {
    /// The exact name first, then the first one equal to it in ASCII case: the packfile's lookup,
    /// which the client's `data_file` entries go through, and near enough NTFS's.
    IgnoringCase,
    /// The exact name only: a folder FXServer opens with `opendir` on Linux, whose file systems
    /// keep case. `files { 'extra/a.meta' }` ships nothing there from a folder named `Extra`.
    Exact,
}

impl Folders {
    /// How FXServer finds the folders of its `file`, `client_script`, `shared_script` and `map`
    /// patterns on this system. A case-insensitive mount on Linux is taken as keeping case: it can
    /// only make a file look unshipped, so a copy is missed, never a copy kept that is not loaded.
    pub const SERVER: Self = if cfg!(windows) {
        Self::IgnoringCase
    } else {
        Self::Exact
    };
}

/// Whether `pattern` holds one of FiveM's wildcards, `*` or `?`.
pub fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

/// The path FiveM resolves `path` to inside the resource: `stream\a.ytyp` and `./stream//a.ytyp`
/// are both `stream/a.ytyp`. `None` when it leaves the resource's folder or names the folder itself.
pub fn normalize(path: &str) -> Option<String> {
    let rooted = rooted(path.as_bytes())?;
    Some(String::from_utf8_lossy(&rooted[ROOT.len() + 1..]).into_owned())
}

/// Whether `pattern` matches the file `path`, relative to the resource's folder with `/` between
/// names, as FiveM matches it.
pub fn matches(pattern: &str, path: &str) -> bool {
    !expand(pattern, std::slice::from_ref(&path.to_owned())).is_empty()
}

/// The entries of `files` that `pattern` matches, in the order FiveM gives them: byte order of the
/// path as FiveM writes it, which keeps the pattern's spelling of the folders it names before any
/// wildcard (looked up ignoring case, as the packfile does). `files` are the files the resource
/// holds or ships, relative to its folder with `/` between names; an entry with a `.` or `..` name
/// is never matched.
///
/// A pattern starting with `@` names another resource's file: FiveM returns it as it is, and so
/// does this. An empty pattern matches nothing.
pub fn expand(pattern: &str, files: &[String]) -> Vec<String> {
    expand_with(pattern, files, Folders::IgnoringCase)
}

/// The entries of `files` that `pattern` matches as [`expand`] gives them, the folders it names
/// found as `folders` says: [`Folders::SERVER`] for the patterns FXServer resolves against the
/// resource's folder.
pub fn expand_with(pattern: &str, files: &[String], folders: Folders) -> Vec<String> {
    if pattern.is_empty() {
        return Vec::new();
    }
    if pattern.starts_with('@') {
        return vec![pattern.to_owned()];
    }
    let tree = Tree::new(files, folders);
    let mut full = ROOT.to_vec();
    full.push(b'/');
    full.extend_from_slice(pattern.as_bytes());
    walk(full, &tree)
        .into_values()
        .filter_map(|index| files.get(index).cloned())
        .collect()
}

/// The paths the client hands the game for a `data_file` entry (`ResourcesTest.cpp`): the matches
/// among the files the resource ships, or, when there are none and the path has no `*`, the path as
/// written. The game then opens that path (see [`lookup`]), or, for a `DLC_ITYP_REQUEST`, takes
/// only its file name.
pub fn data_file_paths(pattern: &str, shipped: &[String]) -> Vec<String> {
    let matched = expand(pattern, shipped);
    if matched.is_empty() && !pattern.contains('*') {
        return vec![pattern.to_owned()];
    }
    matched
}

/// The shipped file the client opens for `path`, the way the resource's packfile finds a path
/// (`VFSRagePackfile.cpp`): name by name, the exact name first, then the first one equal to it in
/// ASCII case. Only `/` separates names; empty names are skipped.
pub fn lookup<'a>(path: &str, shipped: &'a [String]) -> Option<&'a str> {
    let tree = Tree::new(shipped, Folders::IgnoringCase);
    let mut node = 0;
    for name in path
        .as_bytes()
        .split(|&byte| byte == b'/')
        .filter(|name| !name.is_empty())
    {
        let current = tree.nodes.get(node)?;
        if !current.is_folder() {
            break;
        }
        node = tree.child(node, name)?;
    }
    let found = tree.nodes.get(node)?;
    if found.is_folder() {
        return None;
    }
    found
        .file
        .and_then(|index| shipped.get(index))
        .map(String::as_str)
}

/// The resource's files as folders and files, each folder's names in byte order like the packfile's.
struct Tree<'a> {
    nodes: Vec<Node<'a>>,
    folders: Folders,
}

#[derive(Default)]
struct Node<'a> {
    children: BTreeMap<&'a [u8], usize>,
    /// The entry of the list this node is, for a file. A name that is both a file and a folder,
    /// which no file system holds, is taken as the folder.
    file: Option<usize>,
}

impl Node<'_> {
    fn is_folder(&self) -> bool {
        !self.children.is_empty()
    }

    fn is_file(&self) -> bool {
        self.file.is_some() && self.children.is_empty()
    }
}

impl<'a> Tree<'a> {
    fn new(files: &'a [String], folders: Folders) -> Self {
        let mut tree = Tree {
            nodes: vec![Node::default()],
            folders,
        };
        'files: for (index, file) in files.iter().enumerate() {
            let names: Vec<&[u8]> = file
                .as_bytes()
                .split(|&byte| byte == b'/')
                .filter(|name| !name.is_empty())
                .collect();
            if names.is_empty() {
                continue;
            }
            for name in &names {
                if *name == b"." || *name == b".." {
                    continue 'files;
                }
            }
            let mut node = 0;
            for name in names {
                let next = tree.nodes.len();
                let child = *tree.nodes[node].children.entry(name).or_insert(next);
                if child == next {
                    tree.nodes.push(Node::default());
                }
                node = child;
            }
            tree.nodes[node].file.get_or_insert(index);
        }
        tree
    }

    /// The child of `node` named `name`: exactly, or else ignoring ASCII case unless the tree finds
    /// its folders exactly.
    fn child(&self, node: usize, name: &[u8]) -> Option<usize> {
        let children = &self.nodes.get(node)?.children;
        children.get(name).copied().or_else(|| {
            if self.folders == Folders::Exact {
                return None;
            }
            children
                .iter()
                .find(|(child, _)| child.eq_ignore_ascii_case(name))
                .map(|(_, &index)| index)
        })
    }

    /// The folder FiveM lists for `folder`, a path under [`ROOT`] ending with `/`.
    fn folder(&self, folder: &[u8]) -> Option<usize> {
        let mut names = folder.split(|&byte| byte == b'/').filter(|name| !name.is_empty());
        if names.next() != Some(ROOT) {
            return None;
        }
        let mut node = 0;
        for name in names {
            node = self.child(node, name)?;
            if !self.nodes.get(node)?.is_folder() {
                return None;
            }
        }
        Some(node)
    }
}

/// FiveM's `MatchFiles`, run over a work list instead of recursion: each pattern is normalised,
/// then either lists a folder against its last name or, for a `**` starting a name, tries the
/// rest with one `*` less, with the `**` gone, and in every folder below. A pattern already tried
/// adds nothing, its matches going to the same set.
fn walk(pattern: Vec<u8>, tree: &Tree) -> BTreeMap<Vec<u8>, usize> {
    let mut found = BTreeMap::new();
    let mut work = Work {
        patterns: Vec::new(),
        spent: 0,
    };
    work.push(pattern);
    let mut tried = HashSet::new();
    while !work.exhausted()
        && let Some(raw) = work.patterns.pop()
    {
        let Some(pattern) = rooted_full(&raw) else {
            continue;
        };
        if !tried.insert(pattern.clone()) {
            continue;
        }
        let star = pattern.iter().position(|&byte| byte == b'*');
        let before = directory(star.map_or(&pattern[..], |star| &pattern[..star]));
        let slash = star.and_then(|star| {
            pattern[star..]
                .iter()
                .position(|&byte| byte == b'/')
                .map(|offset| star + offset)
        });
        let after = slash.map_or(&[][..], |slash| &pattern[slash + 1..]);
        let recurse = star
            .filter(|&star| pattern.get(star + 1) == Some(&b'*') && (star == 0 || pattern[star - 1] == b'/'));

        if let Some(star) = recurse {
            // One `*` less is what FiveM calls correct, the `**` gone its legacy behaviour.
            for cut in [1, 2] {
                let mut submask = pattern[..star].to_vec();
                submask.extend_from_slice(pattern.get(star + cut..).unwrap_or_default());
                work.push(submask);
            }
            for (name, node) in listing(tree, &pattern[..=star], &mut work) {
                if tree.nodes[node].is_folder() {
                    work.push([before, b"/", name, b"/", &pattern[star..]].concat());
                }
            }
        } else {
            // FiveM cuts the folder pattern one byte short of the `/` that ends it.
            let find = match slash {
                Some(slash) => &pattern[..slash - 1],
                None => &pattern[..],
            };
            for (name, node) in listing(tree, find, &mut work) {
                let is_file = tree.nodes[node].is_file();
                if slash.is_some() && is_file {
                    continue;
                }
                let path = [before, b"/", name].concat();
                if !after.is_empty() {
                    work.push([&path[..], b"/", after].concat());
                } else if is_file
                    && let (Some(index), Some(relative)) =
                        (tree.nodes[node].file, path.strip_prefix(ROOT_SLASH))
                {
                    found.entry(relative.to_vec()).or_insert(index);
                }
            }
        }
    }
    found
}

/// The patterns left to try, and the work spent: the bytes of every pattern queued and every name
/// compared. Past [`MAX_WORK`] the expansion stops with what it matched so far.
struct Work {
    patterns: Vec<Vec<u8>>,
    spent: usize,
}

impl Work {
    fn push(&mut self, pattern: Vec<u8>) {
        self.spend(pattern.len() + 1);
        if !self.exhausted() {
            self.patterns.push(pattern);
        }
    }

    fn spend(&mut self, amount: usize) {
        self.spent = self.spent.saturating_add(amount);
    }

    fn exhausted(&self) -> bool {
        self.spent > MAX_WORK
    }
}

/// The names of the folder `find` names up to its last `/` that match its last name, with their
/// nodes; nothing when the folder does not exist or the name's expression is one FiveM's regex
/// refuses.
fn listing<'t>(tree: &'t Tree, find: &[u8], work: &mut Work) -> Vec<(&'t [u8], usize)> {
    work.spend(find.len());
    let Some(slash) = find.iter().rposition(|&byte| byte == b'/') else {
        return Vec::new();
    };
    let Some(atoms) = compile(&find[slash + 1..]) else {
        return Vec::new();
    };
    let Some(folder) = tree.folder(&find[..=slash]) else {
        return Vec::new();
    };
    let children = &tree.nodes[folder].children;
    work.spend(children.len());
    children
        .iter()
        .filter(|(name, _)| matches_name(&atoms, name))
        .map(|(name, &node)| (*name, node))
        .collect()
}

/// `getDirectory`: up to the last `/`, or `.` without one.
fn directory(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|&byte| byte == b'/')
        .map_or(b".", |slash| &path[..slash])
}

/// `pattern`, already under [`ROOT`], normalised; `None` when it leaves the resource's folder, is
/// the folder itself or is longer than FiveM's buffer.
fn rooted_full(pattern: &[u8]) -> Option<Vec<u8>> {
    let (normalized, escaped) = path_normalize(pattern);
    let under_root =
        normalized.len() > ROOT.len() + 1 && normalized.starts_with(ROOT) && normalized[ROOT.len()] == b'/';
    (!escaped && under_root && normalized.len() <= MAX_PATTERN_BYTES).then_some(normalized)
}

/// `path`, relative to the resource's folder, normalised under [`ROOT`].
fn rooted(path: &[u8]) -> Option<Vec<u8>> {
    let mut full = ROOT.to_vec();
    full.push(b'/');
    full.extend_from_slice(path);
    rooted_full(&full)
}

fn is_separator(byte: u8) -> bool {
    byte == b'/' || byte == b'\\'
}

fn is_blank(byte: u8) -> bool {
    (b'\t'..=b'\r').contains(&byte) || byte == b' '
}

/// Whether a `path_normalize` token starts here: `%`, `$(` or `${`.
fn token_start(rest: &[u8]) -> bool {
    rest.first() == Some(&b'%') || rest.starts_with(b"$(") || rest.starts_with(b"${")
}

/// How many bytes of build-system tokens start `rest`, which `path_normalize` copies untouched:
/// `%{…}`, a lone `%`, `$(…)` and `${…}`. FiveM reads past the end of a token left open; here it
/// runs to the end.
fn token_length(rest: &[u8]) -> usize {
    let through = |at: usize, end: u8| {
        rest[at..]
            .iter()
            .position(|&byte| byte == end)
            .map_or(rest.len() - at, |offset| offset + 1)
    };
    let mut at = 0;
    loop {
        if rest[at..].starts_with(b"%{") {
            at += through(at, b'}');
        }
        // The `%` that starts a `%…%` token also ends it.
        if rest.get(at) == Some(&b'%') {
            at += 1;
        }
        if rest[at..].starts_with(b"$(") {
            at += through(at, b')');
        }
        if rest[at..].starts_with(b"${") {
            at += through(at, b'}');
        }
        if !token_start(&rest[at..]) {
            return at;
        }
    }
}

/// FiveM's `path_normalize`: the path up to its first NUL, leading and trailing blanks dropped,
/// each run between blanks and tokens normalised on its own. Also says whether the first run
/// climbed above its first folder, which for a pattern under [`ROOT`] leaves the resource.
fn path_normalize(path: &[u8]) -> (Vec<u8>, bool) {
    let path = &path[..path.iter().position(|&byte| byte == 0).unwrap_or(path.len())];
    let mut read = path
        .iter()
        .position(|&byte| !is_blank(byte))
        .unwrap_or(path.len());
    let mut out = Vec::with_capacity(path.len());
    let mut end = read;
    let mut first = true;
    let mut escaped = false;
    while end < path.len() {
        let skipped = token_length(&path[read..]);
        if skipped > 0 {
            if read > 0 && is_separator(path[read - 1]) && out.last().is_some_and(|&last| !is_separator(last))
            {
                out.push(path[read - 1]);
            }
            out.extend_from_slice(&path[read..read + skipped]);
            read += skipped;
            end = read;
        }
        while end < path.len() && !is_blank(path[end]) && !token_start(&path[end..]) {
            end += 1;
        }
        if read < end && matches!(path[read], b'"' | b'\'') && path[read] == path[end - 1] {
            out.push(path[read]);
            read += 1;
        }
        let climbed = normalize_run(&path[read..end], &mut out);
        if first && read < end {
            escaped = climbed;
            first = false;
        }
        while end < path.len() && is_blank(path[end]) {
            out.push(path[end]);
            end += 1;
        }
        read = end;
    }
    while out.last().is_some_and(|&last| is_blank(last)) {
        out.pop();
    }
    (out, escaped)
}

/// One run of `path_normalize`, appended to `out`; says whether a `..` climbed above the run's
/// first folder.
fn normalize_run(run: &[u8], out: &mut Vec<u8>) -> bool {
    let mut at = 0;
    if run.len() >= 2 && run[0].is_ascii_alphabetic() && run[1] == b':' {
        out.extend_from_slice(&run[..2]);
        at = 2;
    }
    if run.get(at).is_some_and(|&byte| is_separator(byte)) {
        at += 1;
        out.push(b'/');
        if run.get(at).is_some_and(|&byte| is_separator(byte)) {
            at += 1;
            out.push(b'/');
        }
    }
    let root = out.len();
    let mut depth = 0usize;
    let mut climbed = false;
    let separator_or_end = |at: usize| run.get(at).is_none_or(|&byte| is_separator(byte));
    while at < run.len() {
        while at < run.len() && (is_separator(run[at]) || (run[at] == b'.' && separator_or_end(at + 1))) {
            at += 1;
        }
        if at >= run.len() {
            break;
        }
        if run[at] == b'.' && run.get(at + 1) == Some(&b'.') && separator_or_end(at + 2) {
            if depth > 0 {
                let mut cut = out.len();
                loop {
                    cut -= 1;
                    if cut == root || is_separator(out[cut - 1]) {
                        break;
                    }
                }
                out.truncate(cut);
                depth -= 1;
                climbed |= depth == 0;
            } else {
                out.extend_from_slice(b"../");
                climbed = true;
            }
            at += 3;
        } else {
            while at < run.len() && !is_separator(run[at]) {
                out.push(run[at]);
                at += 1;
            }
            if at < run.len() {
                out.push(b'/');
                at += 1;
                depth += 1;
            }
        }
    }
    while out.len() > root && out.last().is_some_and(|&last| is_separator(last)) {
        out.pop();
    }
    climbed
}

/// One element of the regular expression FiveM builds for a name: a byte or any byte, repeated
/// between `min` and `max` times.
#[derive(Debug, Clone, Copy)]
struct Atom {
    any: bool,
    byte: u8,
    min: usize,
    max: usize,
}

impl Atom {
    fn accepts(self, byte: u8) -> bool {
        // `.` in an ECMAScript expression stops at line breaks.
        if self.any {
            byte != b'\n' && byte != b'\r'
        } else {
            byte == self.byte
        }
    }
}

/// The expression FiveM builds for one name of a pattern: every byte literal but `*` (`.*`), `?`
/// (`.`) and the braces, which FiveM leaves unescaped and the regex reads as repetitions. `None`
/// for braces the regex refuses, which throw in FiveM.
fn compile(name: &[u8]) -> Option<Vec<Atom>> {
    let mut atoms: Vec<Atom> = Vec::with_capacity(name.len());
    let mut repeatable = false;
    let mut at = 0;
    while at < name.len() {
        match name[at] {
            b'*' => {
                atoms.push(Atom {
                    any: true,
                    byte: 0,
                    min: 0,
                    max: usize::MAX,
                });
                repeatable = false;
            }
            b'?' => {
                atoms.push(Atom {
                    any: true,
                    byte: 0,
                    min: 1,
                    max: 1,
                });
                repeatable = true;
            }
            b'{' => {
                let last = atoms.last_mut().filter(|_| repeatable)?;
                let (min, max, length) = repetition(&name[at..])?;
                last.min = min;
                last.max = max;
                repeatable = false;
                at += length;
                continue;
            }
            b'}' => return None,
            byte => {
                atoms.push(Atom {
                    any: false,
                    byte,
                    min: 1,
                    max: 1,
                });
                repeatable = true;
            }
        }
        at += 1;
    }
    Some(atoms)
}

/// `{n}`, `{n,}` or `{n,m}` at the start of `text`: the bounds and the length written.
fn repetition(text: &[u8]) -> Option<(usize, usize, usize)> {
    let number = |from: usize| {
        let digits = text[from..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let value = std::str::from_utf8(&text[from..from + digits])
            .ok()?
            .parse::<usize>()
            .ok()?;
        Some((value, from + digits))
    };
    let (min, mut at) = number(1)?;
    let max = if text.get(at) == Some(&b',') {
        at += 1;
        if text.get(at).is_some_and(u8::is_ascii_digit) {
            let (max, next) = number(at)?;
            at = next;
            max
        } else {
            usize::MAX
        }
    } else {
        min
    };
    (text.get(at) == Some(&b'}') && min <= max).then_some((min, max, at + 1))
}

/// Whether the whole `name` matches `atoms`. Each atom moves the set of positions reached: from a
/// position, between `min` and `max` bytes it accepts in a row.
fn matches_name(atoms: &[Atom], name: &[u8]) -> bool {
    if name == b"." || name == b".." || name.len() > MAX_NAME_BYTES {
        return false;
    }
    let length = name.len();
    let mut reached = vec![false; length + 1];
    reached[0] = true;
    let mut counts = vec![0usize; length + 2];
    for atom in atoms {
        // counts[i] = positions reached before i.
        for at in 0..=length {
            counts[at + 1] = counts[at] + usize::from(reached[at]);
        }
        let mut next = vec![false; length + 1];
        // The first position from which every byte up to the current one is accepted.
        let mut run_start = 0;
        for end in 0..=length {
            if end > 0 && !atom.accepts(name[end - 1]) {
                run_start = end;
            }
            if end < atom.min {
                continue;
            }
            let latest = end - atom.min;
            let earliest = run_start.max(end.saturating_sub(atom.max));
            next[end] = earliest <= latest && counts[latest + 1] > counts[earliest];
        }
        reached = next;
        if !reached.contains(&true) {
            return false;
        }
    }
    reached[length]
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: [&str; 0] = [];

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn a_star_matches_within_one_folder_only() {
        let files = files(&["data/a.meta", "data/b.meta", "data/sub/c.meta", "data/a.txt"]);
        assert_eq!(expand("data/*.meta", &files), ["data/a.meta", "data/b.meta"]);
    }

    #[test]
    fn a_double_star_at_the_start_of_a_name_reaches_every_depth_including_the_first() {
        let files = files(&[
            "meta/weaponarchetypes.meta",
            "meta/pack_02/weaponarchetypes.meta",
            "meta/a/b/weaponarchetypes.meta",
            "meta/a/b/other.meta",
        ]);
        assert_eq!(
            expand("meta/**/weaponarchetypes.meta", &files),
            [
                "meta/a/b/weaponarchetypes.meta",
                "meta/pack_02/weaponarchetypes.meta",
                "meta/weaponarchetypes.meta"
            ]
        );
        assert_eq!(
            expand("**/weaponarchetypes.meta", &files),
            [
                "meta/a/b/weaponarchetypes.meta",
                "meta/pack_02/weaponarchetypes.meta",
                "meta/weaponarchetypes.meta"
            ]
        );
    }

    #[test]
    fn a_double_star_glued_to_a_name_ending_matches_files_at_every_depth() {
        let files = files(&[
            "stream/a.ytyp",
            "stream/x/b.ytyp",
            "stream/x/y/c.ytyp",
            "stream/x/d.ydr",
            "e.ytyp",
        ]);
        assert_eq!(
            expand("stream/**.ytyp", &files),
            ["stream/a.ytyp", "stream/x/b.ytyp", "stream/x/y/c.ytyp"]
        );
        assert_eq!(expand("**/**.ytyp", &files).len(), 4);
    }

    #[test]
    fn a_trailing_double_star_takes_every_file_below_the_folder() {
        let files = files(&["stream/a.ytd", "stream/x/b.ydr", "other/c.ytd"]);
        assert_eq!(expand("stream/**", &files), ["stream/a.ytd", "stream/x/b.ydr"]);
    }

    #[test]
    fn a_double_star_inside_a_name_is_two_stars() {
        let files = files(&["data/vehicles.meta", "data/sub/vehicles.meta"]);
        assert_eq!(expand("data/v**s.meta", &files), ["data/vehicles.meta"]);
    }

    #[test]
    fn a_star_ending_a_folder_name_matches_nothing_as_in_fivem() {
        let files = files(&["meta/pack/weapons.meta", "meta/weapons.meta"]);
        assert_eq!(expand("meta/*/weapons.meta", &files), NONE);
        assert_eq!(expand("*/weapons.meta", &files), NONE);
    }

    #[test]
    fn a_folder_pattern_loses_its_last_byte_before_the_slash() {
        let files = files(&[
            "meta/xmas_pack_02/w.meta",
            "meta/xmas_pack_0/w.meta",
            "meta/xmasX/w.meta",
        ]);
        // `xmas_*_02` is listed as `xmas_*_0`.
        assert_eq!(
            expand("meta/xmas_*_02/w.meta", &files),
            ["meta/xmas_pack_0/w.meta"]
        );
        // `x*y` is listed as `x*`.
        assert_eq!(expand("meta/xmas*Q/w.meta", &files).len(), 3);
    }

    #[test]
    fn a_question_mark_takes_exactly_one_byte() {
        let files = files(&["a1.meta", "a12.meta", "a\u{e9}.meta"]);
        assert_eq!(expand("a?.meta", &files), ["a1.meta"]);
        // `é` is two bytes in UTF-8, as FiveM's regex sees it.
        assert_eq!(expand("a??.meta", &files), ["a12.meta", "a\u{e9}.meta"]);
    }

    #[test]
    fn names_compare_case_included_but_folders_before_a_wildcard_ignore_case() {
        let files = files(&["Data/Vehicles.meta", "data2/x.meta"]);
        assert_eq!(expand("Data/vehicles.meta", &files), NONE);
        assert_eq!(expand("data/Vehicles.meta", &files), ["Data/Vehicles.meta"]);
        assert_eq!(expand("DATA/*.meta", &files), ["Data/Vehicles.meta"]);
        assert_eq!(expand("data/*.META", &files), NONE);
    }

    #[test]
    fn folders_found_exactly_keep_their_case_before_and_after_a_wildcard() {
        let files = files(&["Extra/a.meta", "meta/Pack/b.meta"]);

        assert_eq!(expand_with("extra/a.meta", &files, Folders::Exact), NONE);
        assert_eq!(expand_with("extra/*.meta", &files, Folders::Exact), NONE);
        assert_eq!(expand_with("meta/**/pack/b.meta", &files, Folders::Exact), NONE);
        assert_eq!(
            expand_with("Extra/a.meta", &files, Folders::Exact),
            ["Extra/a.meta"]
        );
        assert_eq!(
            expand_with("meta/**/Pack/b.meta", &files, Folders::Exact),
            ["meta/Pack/b.meta"]
        );
        assert_eq!(
            expand_with("extra/a.meta", &files, Folders::IgnoringCase),
            ["Extra/a.meta"]
        );
        assert_eq!(expand("meta/**/pack/b.meta", &files), ["meta/Pack/b.meta"]);
        // The client's lookup in the packfile ignores case whatever the system.
        assert_eq!(lookup("EXTRA/a.meta", &files), Some("Extra/a.meta"));
    }

    #[test]
    fn matches_come_in_byte_order_of_the_path_as_fivem_spells_it() {
        let files = files(&["a/sub/x.meta", "B/SUB/x.meta", "Sub/x.meta"]);
        // FiveM spells the folder after `**` as written: `Sub/x.meta`, `B/Sub/x.meta`, `a/Sub/x.meta`.
        assert_eq!(
            expand("**/Sub/x.meta", &files),
            ["B/SUB/x.meta", "Sub/x.meta", "a/sub/x.meta"]
        );
    }

    #[test]
    fn a_plain_path_is_normalised_before_it_is_looked_up() {
        let files = files(&["stream/a.ytyp"]);
        for spelling in [
            "./stream/a.ytyp",
            "stream\\a.ytyp",
            "stream//a.ytyp",
            "stream/./a.ytyp",
            "x/../stream/a.ytyp",
            " stream/a.ytyp",
        ] {
            let expected: &[&str] = if spelling.starts_with(' ') {
                &[]
            } else {
                &["stream/a.ytyp"]
            };
            assert_eq!(expand(spelling, &files), expected, "{spelling:?}");
        }
        assert_eq!(normalize("./stream\\a.ytyp").as_deref(), Some("stream/a.ytyp"));
        assert_eq!(normalize("stream/a.ytyp   ").as_deref(), Some("stream/a.ytyp"));
        assert_eq!(normalize("a/ b.ytyp").as_deref(), Some("a b.ytyp"));
        assert_eq!(normalize("../other/a.ytyp"), None);
        assert_eq!(normalize("a/../.."), None);
        assert_eq!(normalize("./"), None);
    }

    #[test]
    fn a_path_climbing_out_of_the_resource_matches_nothing() {
        let files = files(&["a.meta"]);
        assert_eq!(expand("../R/a.meta", &files), NONE);
        assert_eq!(expand("x/../../R/a.meta", &files), NONE);
    }

    #[test]
    fn braces_repeat_the_byte_before_them_like_fivems_regex() {
        let files = files(&["aa.meta", "a{2}.meta", "aaa.meta"]);
        assert_eq!(expand("a{2}.meta", &files), ["aa.meta"]);
        assert_eq!(expand("a{2,}.meta", &files), ["aa.meta", "aaa.meta"]);
        assert_eq!(expand("?{1,2}.meta", &files), ["aa.meta"]);
        assert_eq!(expand("*{2}.meta", &files), NONE);
        assert_eq!(expand("a}.meta", &files), NONE);
        assert_eq!(expand("a{,2}.meta", &files), NONE);
    }

    #[test]
    fn an_empty_pattern_matches_nothing_and_an_at_sign_names_another_resource() {
        let files = files(&["a.meta"]);
        assert_eq!(expand("", &files), NONE);
        assert_eq!(expand("@other/a.meta", &files), ["@other/a.meta"]);
    }

    #[test]
    fn a_folder_is_never_a_match() {
        let files = files(&["stream/x/a.ytd"]);
        assert_eq!(expand("stream/*", &files), NONE);
        assert_eq!(expand("stream/x", &files), NONE);
        assert_eq!(expand("stream/x/", &files), NONE);
    }

    #[test]
    fn matches_agrees_with_expand_for_one_path() {
        assert!(matches("data/**/vehicles.meta", "data/a/vehicles.meta"));
        assert!(matches("*.meta", "a.meta"));
        assert!(!matches("*.meta", "x/a.meta"));
        assert!(!matches("data/*/vehicles.meta", "data/a/vehicles.meta"));
    }

    #[test]
    fn a_data_file_path_without_a_star_is_kept_as_written_when_nothing_matches() {
        let shipped = files(&["data/vehicles.meta"]);
        assert_eq!(data_file_paths("stream/x.ytyp", &shipped), ["stream/x.ytyp"]);
        assert_eq!(
            data_file_paths("Data/Vehicles.meta", &shipped),
            ["Data/Vehicles.meta"]
        );
        assert_eq!(data_file_paths("stream/*.ytyp", &shipped), NONE);
        assert_eq!(data_file_paths("data/*.meta", &shipped), ["data/vehicles.meta"]);
        assert_eq!(lookup("Data/Vehicles.meta", &shipped), Some("data/vehicles.meta"));
        assert_eq!(
            lookup("data//vehicles.meta", &shipped),
            Some("data/vehicles.meta")
        );
        assert_eq!(lookup("data", &shipped), None);
        assert_eq!(lookup("data/missing.meta", &shipped), None);
    }

    #[test]
    fn is_glob_looks_for_fivems_wildcards() {
        assert!(is_glob("stream/**.ytyp"));
        assert!(is_glob("a?.meta"));
        assert!(!is_glob("stream/a.ytyp"));
    }

    #[test]
    fn hostile_patterns_and_paths_end_without_a_panic() {
        let deep = vec!["a"; 1000].join("/") + "/x.meta";
        let files = vec![
            deep.clone(),
            "\u{e9}/\u{e9}*/b".to_owned(),
            "./x".to_owned(),
            String::new(),
        ];
        assert_eq!(expand("**/x.meta", &files), [deep.as_str()]);
        // Forty `**` over a thousand folders would take gigabytes: the work runs out first.
        let _ = expand(&"**/".repeat(40), &files);
        assert_eq!(expand("a*\u{e9}/b", &files), NONE);
        assert_eq!(expand("%{x/%/$(y/${z", &files), NONE);
        assert_eq!(expand("\u{e9}*\u{e9}/b", &files), NONE);
        assert_eq!(expand(&"*".repeat(MAX_PATTERN_BYTES + 10), &files), NONE);
        assert_eq!(expand("a\0b", &files), NONE);
        assert!(!matches("{", "{"));
        assert_eq!(expand("{99999999999999999999999}", &files), NONE);
    }

    #[test]
    fn tokens_and_quotes_pass_through_normalisation() {
        assert_eq!(normalize("100%/a.ytd").as_deref(), Some("100%/a.ytd"));
        assert_eq!(normalize("$(x)/./a.ytd").as_deref(), Some("$(x)/a.ytd"));
        assert_eq!(normalize("'a b'").as_deref(), Some("'a b'"));
    }
}
