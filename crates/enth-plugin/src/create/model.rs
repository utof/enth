//! **Owner: lane `PUIb`** (R3b: the proposal model in the plugin; feature `create`).
//!
//! Which model file "Create from sound" draws starts from, and its loading (docs/r3.md, "The model file"; docs/plugin.md,
//! "The proposal model"). The model's hypotheses join each job as starts (`enth_solver::create::Model::propose`, lane
//! MS); this file only finds, chooses and parses the file, and says what it did.
//!
//! - **Where:** beside the plugin's binary (the CLAP file's folder, or the folder of the binary inside a VST3 bundle),
//!   then `$ENTH_MODELS_DIR` or `<Documents>/enth/Models/` ([`model_dirs`]): the k-NN bank's two folders.
//! - **What:** every `enth-model-*.bin` there ([`is_model_file`], the case of the name ignored), each read through
//!   `ModelPack::parse`. A refused file (any `PackError`: not a model, made for a newer enth, damaged, over a budget,
//!   made for another catalogue) or one that cannot be read is named in the job's notes with its code and hint, and
//!   passed over.
//! - **Which, deterministically:** the first folder holding a usable file wins (beside the plugin before Documents, as
//!   for the bank); in it, the file with the newest header (its layout, feature set, op set, generator, template,
//!   `ds1` version and format, compared in that order), then the greatest file name. Its net is built with
//!   `Model::parse`; a net that refuses (`model_malformed`, `model_too_large`) passes the choice to the next file. The
//!   line the Create view shows names the model and its file; the notes say where it was found and which files were
//!   passed over, and why.
//! - **When:** on the `enth-create` thread at a job's start, never on the audio or the GUI thread, and **parsed once**:
//!   [`Models`] keeps the model between jobs until the model files in those folders change (one added, removed,
//!   replaced or touched: their paths, sizes and modification times), so a file dropped into the folder is used from
//!   the next job without reloading the plugin, and an unchanged folder costs two directory listings per job.
//! - **No model** (none found, or every one refused) means the job runs R3a, search alone, and the line and the notes
//!   say so and why.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use enth_solver::create::Model;
use enth_solver::propose::pack::{Budget, MAX_FILE_BYTES, ModelHeader, ModelPack, PackError};

use crate::presets::{Environment, Os};

/// A model file's name starts with this ...
pub const MODEL_PREFIX: &str = "enth-model-";
/// ... and ends with this (`enth-model-<id>.bin`, docs/r3.md).
pub const MODEL_SUFFIX: &str = ".bin";
/// The pattern, as the notes and the docs write it.
pub const MODEL_PATTERN: &str = "enth-model-*.bin";

/// Whether a file name is a model file's: `enth-model-<something>.bin`, in any case.
pub fn is_model_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.len() > MODEL_PREFIX.len() + MODEL_SUFFIX.len()
        && lower.starts_with(MODEL_PREFIX)
        && lower.ends_with(MODEL_SUFFIX)
}

/// Where model files are looked for, in order: beside the plugin's binary, then [`super::models_dir`]
/// (`$ENTH_MODELS_DIR` or `<Documents>/enth/Models/`).
pub fn model_dirs(env: &dyn Environment, os: Os) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = super::plugin_binary().as_deref().and_then(Path::parent) {
        dirs.push(dir.to_path_buf());
    }
    if let Some(dir) = super::models_dir(env, os)
        && !dirs.contains(&dir)
    {
        dirs.push(dir);
    }
    dirs
}

/// The model files in `dir` (regular files, or links to them), by file name; none when it cannot be listed.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_str().is_some_and(is_model_file))
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

/// A model loaded for create jobs: its net, built once, and the file it came from.
#[derive(Debug)]
pub struct Loaded {
    /// The model (`Model::propose` per job).
    pub model: Model,
    /// Its file.
    pub path: PathBuf,
}

/// What a lookup found: the model, if a file was usable, and what to tell the user.
#[derive(Debug)]
pub struct Lookup {
    /// The model jobs propose starts with, or `None`: search alone (R3a).
    pub model: Option<Loaded>,
    /// One line for the Create view: which model, or why none.
    pub line: String,
    /// The details for the job's notes: where the model was found or where it was looked for, and each file refused
    /// or passed over, with why.
    pub notes: Vec<String>,
}

/// Why a file was not used.
#[derive(Debug)]
enum Refusal {
    /// It could not be read.
    Io(std::io::Error),
    /// The reader or the net refused it.
    Pack(PackError),
}

impl Refusal {
    fn code(&self) -> &'static str {
        match self {
            Refusal::Io(_) => "model_unreadable",
            Refusal::Pack(e) => e.code(),
        }
    }

    fn hint(&self) -> String {
        match self {
            Refusal::Io(_) => {
                "check the file can be read (its permissions, or a copy still in progress); create runs without it meanwhile"
                    .to_owned()
            }
            Refusal::Pack(e) => e.hint(),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Io(e) => write!(f, "it cannot be read: {e}"),
            Refusal::Pack(e) => write!(f, "{e}"),
        }
    }
}

/// A file's bytes, refusing one over the format's size budget before reading it.
fn read(path: &Path) -> Result<Vec<u8>, Refusal> {
    let len = std::fs::metadata(path).map_err(Refusal::Io)?.len();
    if len > MAX_FILE_BYTES as u64 {
        return Err(Refusal::Pack(PackError::TooLarge {
            what: Budget::File,
            found: len,
            limit: MAX_FILE_BYTES as u64,
        }));
    }
    std::fs::read(path).map_err(Refusal::Io)
}

/// How new a header is: the newer of two usable files has the greater key (then the greater file name wins).
fn newness(h: &ModelHeader) -> [u16; 7] {
    [
        h.layout,
        h.feature,
        h.op_set,
        h.generator,
        u16::from(h.template),
        h.ds1,
        h.format,
    ]
}

fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The note for a file that was not used.
fn refused_note(path: &Path, why: &Refusal) -> String {
    format!(
        "the model file {} was refused ({}: {why}): {}",
        path.display(),
        why.code(),
        why.hint()
    )
}

/// The model `dirs` hold, chosen by the rule in the [module docs](self), and what to say about it. Reads every model
/// file in the folders up to the one that holds a usable model (on the create thread).
pub fn lookup(dirs: &[PathBuf]) -> Lookup {
    let mut notes = Vec::new();
    let mut refused: Vec<(String, &'static str)> = Vec::new();
    for dir in dirs {
        // Every usable file of this folder, newest first.
        let mut usable: Vec<([u16; 7], PathBuf)> = Vec::new();
        for path in list(dir) {
            match read(&path).and_then(|b| ModelPack::parse(&b).map_err(Refusal::Pack)) {
                Ok(pack) => usable.push((newness(pack.header()), path)),
                Err(why) => {
                    notes.push(refused_note(&path, &why));
                    refused.push((name(&path), why.code()));
                }
            }
        }
        usable.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        for (i, (_, path)) in usable.iter().enumerate() {
            match read(path).and_then(|b| Model::parse(&b).map_err(Refusal::Pack)) {
                Ok(model) => {
                    let net = model.net();
                    let found = format!(
                        "model: {} from {} ({} hypotheses, {:.2} GFLOP per inference)",
                        model.id(),
                        path.display(),
                        model.hypotheses(),
                        net.flops() as f64 / 1e9
                    );
                    notes.insert(0, found);
                    for (_, other) in &usable[i + 1..] {
                        notes.push(format!(
                            "the model file {} was not used: {} is newer by its header or comes later by name",
                            other.display(),
                            name(path)
                        ));
                    }
                    let line = format!("Model: {} ({})", model.id(), name(path));
                    return Lookup {
                        model: Some(Loaded {
                            model,
                            path: path.clone(),
                        }),
                        line,
                        notes,
                    };
                }
                Err(why) => {
                    notes.push(refused_note(path, &why));
                    refused.push((name(path), why.code()));
                }
            }
        }
    }
    let looked = if dirs.is_empty() {
        "no folder".to_owned()
    } else {
        dirs.iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let line = match refused.as_slice() {
        [] => {
            notes.push(format!(
                "no model file ({MODEL_PATTERN}) found (looked in {looked}): search alone, without the model's proposals"
            ));
            format!("No model: search alone (no {MODEL_PATTERN} found)")
        }
        [(file, code)] => {
            notes.push(
                "no usable model file: search alone, without the model's proposals".to_owned(),
            );
            format!("No model: {file} was refused ({code}); search alone")
        }
        many => {
            notes.push(
                "no usable model file: search alone, without the model's proposals".to_owned(),
            );
            format!(
                "No model: {} model files were refused; search alone",
                many.len()
            )
        }
    };
    Lookup {
        model: None,
        line,
        notes,
    }
}

/// One model file as the folder showed it: what tells a changed file from the one loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

fn scan(dirs: &[PathBuf]) -> Vec<Seen> {
    dirs.iter()
        .flat_map(|d| list(d))
        .map(|path| {
            let meta = std::fs::metadata(&path).ok();
            Seen {
                len: meta.as_ref().map_or(0, std::fs::Metadata::len),
                modified: meta.and_then(|m| m.modified().ok()),
                path,
            }
        })
        .collect()
}

/// The create thread's model, parsed once and kept between jobs (see the [module docs](self)).
#[derive(Debug, Default)]
pub struct Models {
    key: Option<(Vec<PathBuf>, Vec<Seen>)>,
    lookup: Option<Lookup>,
}

impl Models {
    /// The lookup for `dirs`: the kept one while the folders' model files are unchanged, else a new one (the old
    /// model is dropped first, so two are never held at once). The bool says whether it looked again.
    pub fn get(&mut self, dirs: &[PathBuf]) -> (&Lookup, bool) {
        let key = (dirs.to_vec(), scan(dirs));
        let again = self.lookup.is_none() || self.key.as_ref() != Some(&key);
        if again {
            self.lookup = None;
            self.key = Some(key);
        }
        (self.lookup.get_or_insert_with(|| lookup(dirs)), again)
    }
}
