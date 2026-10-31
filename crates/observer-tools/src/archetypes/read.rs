//! Reading the files the clients load, spread over the machine's cores: a large server loads
//! hundreds of `.ytyp` and model-info files, each inflated and digested, and the start of the
//! server may wait for the scan. Results come back in the order asked for, whatever thread read
//! them.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::thread;

use observer_domain::tools::{FileFormat, FileProblem};
use observer_gamefiles::model_info::{self, ModelInfoFile};
use observer_gamefiles::rsc7::{self, FileKind};
use observer_gamefiles::ytyp::{self, MapTypes};
use sha2::{Digest, Sha256};

use crate::resources::Error;

/// The largest file read: a streamed `.ytyp` holds kilobytes, a model-info file at most a few
/// hundred.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// The most threads one scan takes.
const MAX_THREADS: usize = 8;

/// What a file holds, as the client reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Want {
    Ytyp,
    ModelInfo(model_info::Kind),
}

/// A file read.
#[derive(Debug)]
pub(super) enum Loaded {
    Ytyp {
        sha256: [u8; 32],
        map: MapTypes,
    },
    ModelInfo {
        sha256: [u8; 32],
        file: ModelInfoFile,
    },
    /// A model-info file of another kind than its data file says, or none of the three.
    Mismatch(Option<model_info::Kind>),
    Unreadable(FileProblem),
}

/// Reads every file of `jobs`, under `folder`, in order.
pub(super) fn read_all(
    folder: &Path,
    jobs: &[(PathBuf, Want)],
    cancel: &AtomicBool,
) -> Result<Vec<Loaded>, Error> {
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    let threads = thread::available_parallelism()
        .map_or(1, usize::from)
        .clamp(1, MAX_THREADS)
        .min(jobs.len());
    let chunk = jobs.len().div_ceil(threads);
    thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|(path, want)| {
                            crate::resources::check(cancel)?;
                            Ok(read_one(&folder.join(path), *want))
                        })
                        .collect::<Result<Vec<_>, Error>>()
                })
            })
            .collect();
        let mut all = Vec::with_capacity(jobs.len());
        for handle in handles {
            match handle.join() {
                Ok(part) => all.extend(part?),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        Ok(all)
    })
}

/// Reads one file, its problem said when it cannot be.
pub(super) fn read_one(path: &Path, want: Want) -> Loaded {
    let bytes = match read_bytes(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Loaded::Unreadable(FileProblem::Io {
                error: error.to_string(),
            });
        }
    };
    parse(&bytes, want)
}

/// Parses the bytes of a file.
pub(super) fn parse(bytes: &[u8], want: Want) -> Loaded {
    let sha256: [u8; 32] = Sha256::digest(bytes).into();
    match want {
        Want::Ytyp => match rsc7::sniff(bytes) {
            FileKind::Rsc7 => match ytyp::read(bytes) {
                Ok(map) => Loaded::Ytyp { sha256, map },
                Err(error) => Loaded::Unreadable(FileProblem::Invalid {
                    error: error.to_string(),
                }),
            },
            other => Loaded::Unreadable(FileProblem::Format {
                found: file_format(other),
            }),
        },
        Want::ModelInfo(kind) => match model_info::read(bytes) {
            Ok(Some(file)) if file.kind == kind => Loaded::ModelInfo { sha256, file },
            Ok(Some(file)) => Loaded::Mismatch(Some(file.kind)),
            Ok(None) => Loaded::Mismatch(None),
            Err(error) => Loaded::Unreadable(FileProblem::Invalid {
                error: error.to_string(),
            }),
        },
    }
}

/// The file's bytes, refusing one past [`MAX_BYTES`].
pub(super) fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let length = fs::metadata(path)?.len();
    if length > MAX_BYTES {
        return Err(io::Error::other(format!("larger than {} MiB", MAX_BYTES >> 20)));
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_BYTES).read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn file_format(kind: FileKind) -> FileFormat {
    match kind {
        FileKind::Rsc7 => FileFormat::Rsc7,
        FileKind::Escrowed => FileFormat::Escrowed,
        FileKind::Pso => FileFormat::Pso,
        FileKind::Rbf => FileFormat::Rbf,
        FileKind::Xml => FileFormat::Xml,
        FileKind::Empty => FileFormat::Empty,
        FileKind::Unknown => FileFormat::Unknown,
    }
}
