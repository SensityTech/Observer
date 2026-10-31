//! Reads every `.ytyp` under the folder `OBSERVER_GAMEFILES_CORPUS` names, a server's `resources`
//! for example, and prints what it found. Real game files are paid assets and never enter the
//! repository: this test only reads them where they are, and does nothing without the variable.
//!
//! Every RSC7 file must read, every archetype must digest, and every file with archetypes must
//! survive a patch that removes nothing and one that removes its last archetype.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use observer_gamefiles::rsc7::{FileKind, sniff};
use observer_gamefiles::ytyp::{self, ArchetypeKind};

const VARIABLE: &str = "OBSERVER_GAMEFILES_CORPUS";
/// Larger files are counted and left unread.
const MAX_FILE: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct Summary {
    files: usize,
    not_resources: BTreeMap<String, usize>,
    read_failures: BTreeMap<String, Vec<PathBuf>>,
    archetypes: BTreeMap<ArchetypeKind, usize>,
    skipped_entries: usize,
    multi_block_files: usize,
    digest_failures: Vec<(PathBuf, usize)>,
    patch_refusals: BTreeMap<String, usize>,
    patch_failures: Vec<(PathBuf, String)>,
    patched_files: usize,
    /// Every copy of each archetype name, over the whole corpus.
    copies: BTreeMap<u32, Vec<ytyp::Archetype>>,
}

#[test]
fn every_ytyp_of_the_corpus_reads_digests_and_survives_a_patch() {
    let Some(root) = std::env::var_os(VARIABLE) else {
        eprintln!("{VARIABLE} is not set: no corpus to read");
        return;
    };
    let mut paths = Vec::new();
    walk(Path::new(&root), &mut paths);
    paths.sort();

    let mut summary = Summary::default();
    for path in &paths {
        check(path, &mut summary);
    }
    print(&summary);

    assert!(summary.files > 0, "no .ytyp under {}", Path::new(&root).display());
    assert!(summary.read_failures.is_empty(), "RSC7 files failed to read");
    assert!(summary.digest_failures.is_empty(), "archetypes failed to digest");
    assert!(summary.patch_failures.is_empty(), "files failed to patch");
}

/// Every file named `*.ytyp` in any case under `folder`, links left alone.
fn walk(folder: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, paths);
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ytyp"))
        {
            paths.push(path);
        }
    }
}

fn check(path: &Path, summary: &mut Summary) {
    summary.files += 1;
    let too_large = fs::metadata(path).map_or(true, |metadata| metadata.len() > MAX_FILE);
    let bytes = match fs::read(path) {
        Ok(bytes) if !too_large => bytes,
        Ok(_) => return count(&mut summary.not_resources, "larger than 64 MiB"),
        Err(error) => return count(&mut summary.not_resources, &format!("unreadable: {error}")),
    };
    let kind = sniff(&bytes);
    if kind != FileKind::Rsc7 {
        return count(&mut summary.not_resources, kind.describe());
    }
    let map_types = match ytyp::read(&bytes) {
        Ok(map_types) => map_types,
        Err(error) => {
            summary
                .read_failures
                .entry(error.to_string())
                .or_default()
                .push(path.to_owned());
            return;
        }
    };
    for archetype in &map_types.archetypes {
        *summary.archetypes.entry(archetype.kind).or_default() += 1;
        if archetype.digest.is_none() || archetype.major_digest.is_none() {
            summary.digest_failures.push((path.to_owned(), archetype.index));
        }
        summary
            .copies
            .entry(archetype.name)
            .or_default()
            .push(archetype.clone());
    }
    summary.skipped_entries += map_types.skipped.len();
    if blocks_of_one_kind(&bytes) {
        summary.multi_block_files += 1;
    }
    let Some(last) = map_types.archetypes.last() else {
        return;
    };
    if let Some(reason) = map_types.patch_refusal {
        return count(&mut summary.patch_refusals, reason);
    }
    match patch_round_trips(&bytes, &map_types, last.index) {
        Ok(()) => summary.patched_files += 1,
        Err(problem) => summary.patch_failures.push((path.to_owned(), problem)),
    }
}

/// Whether the file spreads one archetype kind over several blocks.
fn blocks_of_one_kind(bytes: &[u8]) -> bool {
    let Ok(resource) = observer_gamefiles::rsc7::Resource::read(bytes) else {
        return false;
    };
    let Ok(meta) = observer_gamefiles::meta::Meta::parse(resource.system()) else {
        return false;
    };
    ArchetypeKind::ALL.into_iter().any(|kind| {
        meta.blocks()
            .iter()
            .filter(|block| block.structure == kind.structure())
            .count()
            > 1
    })
}

fn patch_round_trips(bytes: &[u8], before: &ytyp::MapTypes, last: usize) -> Result<(), String> {
    let same = ytyp::remove_archetypes(bytes, &[]).map_err(|error| format!("removing nothing: {error}"))?;
    let reread = ytyp::read(&same).map_err(|error| format!("reading the unchanged copy: {error}"))?;
    if &reread != before {
        return Err("the unchanged copy reads differently".to_owned());
    }
    let shorter =
        ytyp::remove_archetypes(bytes, &[last]).map_err(|error| format!("removing the last: {error}"))?;
    let after = ytyp::read(&shorter).map_err(|error| format!("reading the patched copy: {error}"))?;
    let mut expected = before.clone();
    expected.archetypes.retain(|archetype| archetype.index != last);
    // Entries of other types after the last archetype move up by one.
    for skipped in &mut expected.skipped {
        skipped.index -= usize::from(skipped.index > last);
    }
    if after != expected {
        return Err("the patched copy does not hold the archetypes kept".to_owned());
    }
    Ok(())
}

fn count(counts: &mut BTreeMap<String, usize>, reason: &str) {
    *counts.entry(reason.to_owned()).or_default() += 1;
}

fn print(summary: &Summary) {
    let rsc7 = summary.files - summary.not_resources.values().sum::<usize>();
    let total: usize = summary.archetypes.values().sum();
    println!("files: {} ({rsc7} RSC7)", summary.files);
    for (reason, files) in &summary.not_resources {
        println!("  not read, {reason}: {files}");
    }
    println!(
        "read failures among RSC7 files: {}",
        summary.read_failures.values().map(Vec::len).sum::<usize>()
    );
    for (reason, files) in &summary.read_failures {
        println!("  {reason}: {}", files.len());
        for file in files.iter().take(5) {
            println!("    {}", file.display());
        }
    }
    println!("archetypes: {total}");
    for (kind, archetypes) in &summary.archetypes {
        println!("  {}: {archetypes}", kind.name());
    }
    println!("entries of other types skipped: {}", summary.skipped_entries);
    println!(
        "files with one kind over several blocks: {}",
        summary.multi_block_files
    );
    println!("digest failures: {}", summary.digest_failures.len());
    for (file, index) in summary.digest_failures.iter().take(10) {
        println!("  {} entry {index}", file.display());
    }
    let groups: Vec<&Vec<ytyp::Archetype>> = summary
        .copies
        .values()
        .filter(|copies| copies.len() > 1)
        .collect();
    let (mut identical, mut minor, mut fields) = (0, 0, BTreeMap::<&str, usize>::new());
    for copies in &groups {
        let first = &copies[0];
        if copies.iter().all(|copy| copy.digest == first.digest) {
            identical += 1;
        } else if copies.iter().all(|copy| copy.major_digest == first.major_digest) {
            minor += 1;
        } else {
            for copy in &copies[1..] {
                for field in ytyp::differences(first, copy) {
                    *fields.entry(field).or_default() += 1;
                }
            }
        }
    }
    println!("names defined more than once in the corpus: {}", groups.len());
    println!("  identical: {identical}");
    println!("  differing in minor fields only: {minor}");
    println!("  differing otherwise: {}", groups.len() - identical - minor);
    for (field, copies) in &fields {
        println!("    copies differing in {field}: {copies}");
    }
    println!(
        "files patched (nothing, then the last archetype): {}",
        summary.patched_files
    );
    for (reason, files) in &summary.patch_refusals {
        println!("  refused, {reason}: {files}");
    }
    println!("patch failures: {}", summary.patch_failures.len());
    for (file, problem) in summary.patch_failures.iter().take(10) {
        println!("  {}: {problem}", file.display());
    }
}
