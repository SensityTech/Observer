//! Every `.meta` file under a real folder, read as a model-info file. Set `OBSERVER_GAMEFILES_CORPUS`
//! to a folder, a server's `resources` for instance, and run with `--nocapture` to see the summary;
//! without it the test reads nothing. The folder is only read: removals are checked in memory.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use observer_gamefiles::model_info::{self, Kind, ModelInfoFile, ModelItem};
use observer_gamefiles::xml::Document;

/// How many files each list of the summary names.
const SHOWN: usize = 40;

/// The `.meta` files under `root`, whatever the case of their extension, in path order. Links to
/// folders are not followed.
fn metas(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                folders.push(path);
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("meta"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[derive(Debug, Default)]
struct Tally {
    files: usize,
    items: usize,
    unnamed: usize,
    /// Items whose hash an earlier item of the same file already has.
    repeated: usize,
    /// Of those, the ones whose canonical text matches the earlier copy.
    repeated_identical: usize,
    /// `<modelName>` (or `<Name>` for peds) as a plain search finds it, commented copies included.
    written_names: usize,
    /// Children of `InitDatas` that are not `Item`.
    other_children: usize,
    without_list: usize,
}

/// The variant of an error, to group them by.
fn reason(error: &model_info::Error) -> String {
    let variant = |debug: String| debug.split(['(', ' ', '{']).next().unwrap_or_default().to_owned();
    match error {
        model_info::Error::Xml(error) => format!("Xml {}", variant(format!("{:?}", error.problem))),
        error => variant(format!("{error:?}")),
    }
}

/// Occurrences of `<needle>` in `text`, ASCII case ignored.
fn occurrences(text: &[u8], needle: &str) -> usize {
    let needle = format!("<{needle}>").into_bytes();
    text.windows(needle.len())
        .filter(|window| window.eq_ignore_ascii_case(&needle))
        .count()
}

/// Checks that removing the last item of `file` cuts one run of bytes holding it and white space
/// alone, and that the rest reads back as the same items.
fn check_last_removal(bytes: &[u8], file: &ModelInfoFile) -> Result<(), String> {
    let Some(last) = file.items.last() else {
        return Ok(());
    };
    let output = model_info::remove_items(bytes, &[last.ordinal]).map_err(|error| error.to_string())?;
    let removed = bytes.len() - output.len();
    let prefix = bytes
        .iter()
        .zip(&output)
        .take_while(|(before, after)| before == after)
        .count();
    let suffix = bytes
        .iter()
        .rev()
        .zip(output.iter().rev())
        .take_while(|(before, after)| before == after)
        .count();
    // Removing `removed` bytes from any start between these two leaves the same output: a cut that
    // begins as the text after it does can be seen a few bytes later.
    let earliest = output.len().saturating_sub(suffix);
    if earliest > prefix {
        return Err("more than one run of bytes changed".to_owned());
    }
    let blank = |part: &[u8]| {
        part.iter()
            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
    };
    let item = last.span.clone();
    let clean = (earliest..=prefix).any(|start| {
        let end = start + removed;
        start <= item.start
            && item.end <= end
            && blank(&bytes[start..item.start])
            && blank(&bytes[item.end..end])
    });
    if !clean {
        return Err("the bytes cut are not the item and white space around it".to_owned());
    }
    let after = model_info::read(&output)
        .map_err(|error| error.to_string())?
        .ok_or("no longer a model-info file")?;
    let left = &file.items[..file.items.len() - 1];
    if after.items != left || after.unnamed != file.unnamed {
        return Err("the items left differ".to_owned());
    }
    Ok(())
}

/// What the corpus holds, file by file.
#[derive(Debug, Default)]
struct Survey {
    other_files: usize,
    /// Files that could not be read, by reason.
    errors: BTreeMap<String, Vec<String>>,
    tallies: BTreeMap<Kind, Tally>,
    repeats: Vec<String>,
    unnamed: Vec<String>,
    failures: Vec<String>,
}

impl Survey {
    fn add(&mut self, shown: &str, bytes: &[u8]) {
        let file = match model_info::read(bytes) {
            Ok(Some(file)) => file,
            Ok(None) => {
                self.other_files += 1;
                return;
            }
            Err(error) => {
                self.errors
                    .entry(reason(&error))
                    .or_default()
                    .push(format!("{shown}: {error}"));
                return;
            }
        };
        let tally = self.tallies.entry(file.kind).or_default();
        tally.files += 1;
        tally.items += file.items.len();
        tally.unnamed += file.unnamed;
        tally.written_names += occurrences(bytes, file.kind.name_element());
        self.list(shown, bytes, &file);
        self.repeats(shown, bytes, &file);
        if let Err(failure) = check_last_removal(bytes, &file) {
            self.failures.push(format!("{shown}: {failure}"));
        }
    }

    /// Looks at `InitDatas` through the XML reader: what is not an `Item`, and the items without a name.
    fn list(&mut self, shown: &str, bytes: &[u8], file: &ModelInfoFile) {
        let tally = self.tallies.entry(file.kind).or_default();
        let document = Document::parse(std::str::from_utf8(bytes).unwrap()).unwrap();
        let Some(list) = document.root().child("InitDatas") else {
            tally.without_list += 1;
            return;
        };
        tally.other_children += list.children().filter(|child| child.name() != "Item").count();
        for item in list.children().filter(|child| child.name() == "Item") {
            let named = item
                .child(file.kind.name_element())
                .is_some_and(|name| !name.text().trim().is_empty());
            if !named {
                let first = item.children().next().map_or("nothing", |child| child.name());
                self.unnamed
                    .push(format!("{shown}:{} (first child {first})", item.line()));
            }
        }
    }

    /// Items whose hash an earlier item of the same file has, then the file without those later
    /// copies, which must read back with one item per hash.
    fn repeats(&mut self, shown: &str, bytes: &[u8], file: &ModelInfoFile) {
        let tally = self.tallies.entry(file.kind).or_default();
        let mut first_by_hash: HashMap<u32, &ModelItem> = HashMap::new();
        let mut later = Vec::new();
        for item in &file.items {
            let Some(first) = first_by_hash.get(&item.hash) else {
                first_by_hash.insert(item.hash, item);
                continue;
            };
            later.push(item.ordinal);
            tally.repeated += 1;
            let difference = if first.canonical == item.canonical {
                tally.repeated_identical += 1;
                "identical".to_owned()
            } else {
                format!("differing in {:?}", model_info::differing_fields(first, item))
            };
            self.repeats.push(format!(
                "{shown}: {} at lines {} and {}, {difference}",
                item.name, first.line, item.line
            ));
        }
        if later.is_empty() {
            return;
        }
        let left = model_info::remove_items(bytes, &later)
            .map_err(|error| error.to_string())
            .and_then(|output| model_info::read(&output).map_err(|error| error.to_string()));
        match left {
            Ok(Some(left)) if left.items.len() == first_by_hash.len() => {}
            other => self
                .failures
                .push(format!("{shown}: removing the later copies gave {other:?}")),
        }
    }

    fn print(&self) {
        println!("other kinds of meta: {}", self.other_files);
        for (kind, tally) in &self.tallies {
            println!(
                "{kind:?}: {} files, {} named items, {} unnamed, {} repeated in their file ({} identical), \
                 {} `<{}>` written (commented ones included), {} other children of InitDatas, {} without \
                 InitDatas",
                tally.files,
                tally.items,
                tally.unnamed,
                tally.repeated,
                tally.repeated_identical,
                tally.written_names,
                kind.name_element(),
                tally.other_children,
                tally.without_list,
            );
        }
        let error_count: usize = self.errors.values().map(Vec::len).sum();
        println!("errors: {error_count}");
        for (reason, files) in &self.errors {
            println!("  {reason}: {}", files.len());
            for file in files.iter().take(SHOWN) {
                println!("    {file}");
            }
        }
        for (title, lines) in [
            ("repeated items", &self.repeats),
            ("unnamed items", &self.unnamed),
            ("removal failures", &self.failures),
        ] {
            println!("{title}: {}", lines.len());
            for line in lines.iter().take(SHOWN) {
                println!("  {line}");
            }
        }
    }
}

#[test]
fn every_meta_file_of_the_corpus_reads_and_loses_its_last_item_cleanly() {
    let Some(root) = std::env::var_os("OBSERVER_GAMEFILES_CORPUS") else {
        return;
    };
    let root = PathBuf::from(root);
    let paths = metas(&root);
    let mut survey = Survey::default();
    for path in &paths {
        let shown = path.strip_prefix(&root).unwrap_or(path).display().to_string();
        match fs::read(path) {
            Ok(bytes) => survey.add(&shown, &bytes),
            Err(error) => survey
                .errors
                .entry("Io".to_owned())
                .or_default()
                .push(format!("{shown}: {error}")),
        }
    }

    println!("corpus: {} ({} .meta files)", root.display(), paths.len());
    survey.print();
    assert_eq!(survey.failures, Vec::<String>::new());
}
