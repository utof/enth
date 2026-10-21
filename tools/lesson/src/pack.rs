//! **Owner: lane LCK.** Pre-declared by lane S0l (R2).
//!
//! `pack [--out DIR] [--check] [PACK...]` (`cargo xtask pack`): builds `content/lessons/<pack>/` into a reproducible
//! `.enthpack` (docs/learn.md, "Packs"). Authors write TOML; this writes canonical JSON (`Lesson::to_json`, the sealed
//! `Manifest::to_json`) into the canonical archive (`enth_learn::reader::write_archive`: `pack.json` first, then the
//! entries in path order, deflate at the fixed level 9 (`miniz_oxide`, pinned with `=`) or stored when that does not
//! shrink, DOS-zero timestamps (the zip epoch, 1980), no permissions or comments), so a pack builds to the same bytes
//! on every platform. The archive is read back through the real reader before it is written.
//!
//! **Pins.** Each built pack is compared with the pins that exist for it, and a mismatch fails:
//!
//! | Pin | Where | Rule |
//! | --- | --- | --- |
//! | A bundled archive | `crates/enth-plugin/packs/<id>.enthpack` (the plugin's `include_bytes!`, lane SKL) | the same bytes |
//! | The catalogue | `content/packs/catalogue.sha256`, `sha256sum` lines `<sha256>  <id>-<version>.enthpack` (report 02 §8's catalogue, in a form `sha256sum -c` reads; absent until a release pins one) | the same SHA-256 for the pack's id and version |
//!
//! Without `--out` the archives go to `$CARGO_TARGET_DIR/packs/` (else `target/packs/`). `--out
//! crates/enth-plugin/packs` rebuilds the bundled archives: that is how a bundled pin is updated on purpose, and the
//! commit says why. `--check` builds and compares without writing.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use enth_learn::Manifest;
use enth_learn::pack::{MANIFEST_PATH, sha256};
use enth_learn::reader::{extract, read_archive, write_archive};

use crate::source::{self, Source};

/// Where the plugin keeps the archives it bundles (lane SKL), under the repository root.
pub(crate) const BUNDLED_DIR: &str = "crates/enth-plugin/packs";
/// The release catalogue of pack hashes (report 02 §8), under the repository root.
pub(crate) const CATALOGUE: &str = "content/packs/catalogue.sha256";

/// A built pack: its id, version, archive bytes and their SHA-256.
#[derive(Debug)]
pub(crate) struct Built {
    /// The pack's id.
    pub id: String,
    /// Its version.
    pub version: String,
    /// The `.enthpack`.
    pub archive: Vec<u8>,
    /// Its SHA-256, hex.
    pub sha256: String,
}

/// Build `source`'s files into the canonical archive and read it back through the real reader (every entry rule,
/// every size and SHA-256, the lessons decoded), checking the reader gives back exactly the files.
pub(crate) fn build(source: &Source) -> Result<Built, String> {
    let manifest_text = source
        .files
        .get(MANIFEST_PATH)
        .and_then(|b| std::str::from_utf8(b).ok())
        .ok_or_else(|| format!("{MANIFEST_PATH}: missing or not UTF-8"))?;
    let manifest = Manifest::from_json(manifest_text).map_err(|e| source::learn_error(&e))?;
    let archive = write_archive(&source.files);
    read_archive(&archive).map_err(|e| {
        format!(
            "the built archive does not read back: {}",
            source::learn_error(&e)
        )
    })?;
    let back = extract(&archive).map_err(|e| source::learn_error(&e))?;
    if back != source.files {
        return Err("the built archive reads back other files than were packed".to_owned());
    }
    Ok(Built {
        id: manifest.id.to_string(),
        version: manifest.version.to_string(),
        sha256: hex(&sha256(&archive)),
        archive,
    })
}

/// What a pin said about a built pack.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PinCheck {
    /// Which pin (its path).
    pub pin: String,
    /// `None` when it matches; otherwise why not.
    pub mismatch: Option<String>,
}

/// Compare `built` with every pin that exists for it under `root` (see the [module docs](self)); `skip` is a
/// bundled directory about to be rewritten (no comparison there).
pub(crate) fn pins(root: &Path, built: &Built, skip: Option<&Path>) -> Vec<PinCheck> {
    let mut out = Vec::new();
    let bundled_dir = root.join(BUNDLED_DIR);
    let bundled = bundled_dir.join(format!("{}.enthpack", built.id));
    let skipped = skip.is_some_and(|s| same_dir(s, &bundled_dir));
    if bundled.is_file() && !skipped {
        let mismatch = match fs::read(&bundled) {
            Ok(bytes) if bytes == built.archive => None,
            Ok(bytes) => Some(format!(
                "the bundled archive (SHA-256 {}) is not what the content builds ({}): rebuild it with `cargo xtask \
                 pack --out {BUNDLED_DIR} {}` and say why in the commit",
                hex(&sha256(&bytes)),
                built.sha256,
                built.id
            )),
            Err(e) => Some(format!("cannot read it: {e}")),
        };
        out.push(PinCheck {
            pin: format!("{BUNDLED_DIR}/{}.enthpack", built.id),
            mismatch,
        });
    }
    let catalogue = root.join(CATALOGUE);
    if catalogue.is_file() {
        let mismatch = match fs::read_to_string(&catalogue) {
            Ok(text) => catalogue_mismatch(&text, built),
            Err(e) => Some(format!("cannot read it: {e}")),
        };
        out.push(PinCheck {
            pin: CATALOGUE.to_owned(),
            mismatch,
        });
    }
    out
}

/// Why the catalogue `text` does not pin `built`. Its lines are `<sha256>  <id>-<version>.enthpack` (`sha256sum`'s
/// form; `#` starts a comment). A pack it does not list at this version is no mismatch; a second line for the same
/// file is.
fn catalogue_mismatch(text: &str, built: &Built) -> Option<String> {
    let file = format!("{}-{}.enthpack", built.id, built.version);
    let mut found: Vec<&str> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((hash, name)) = line.split_once(char::is_whitespace) else {
            return Some(format!("line {}: not `<sha256>  <file>`", n + 1));
        };
        let name = name.trim_start().trim_start_matches('*');
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(format!("line {}: {hash:?} is not a SHA-256", n + 1));
        }
        if name == file {
            found.push(hash);
        }
    }
    match found.as_slice() {
        [] => None,
        [hash] if hash.eq_ignore_ascii_case(&built.sha256) => None,
        [hash] => Some(format!(
            "it pins {file} at SHA-256 {hash}, the content builds {}",
            built.sha256
        )),
        _ => Some(format!("it lists {file} {} times", found.len())),
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// SHA-256 bytes as lower-case hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The default output directory: `$CARGO_TARGET_DIR/packs`, else `<root>/target/packs`.
fn default_out(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), PathBuf::from)
        .join("packs")
}

/// `pack`.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut out: Option<PathBuf> = None;
    let mut check_only = false;
    let mut names = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => {
                out = Some(PathBuf::from(it.next().ok_or("--out needs a directory")?));
            }
            "--check" => check_only = true,
            flag if flag.starts_with("--") => {
                return Err(format!(
                    "pack: unknown option {flag}\nusage: pack [--out DIR] [--check] [<pack>...]"
                ));
            }
            name => names.push(name.to_owned()),
        }
    }
    let root = source::repo_root();
    let out = out.unwrap_or_else(|| default_out(&root));
    let mut failed = 0_usize;
    for path in source::resolve(&root, &names)? {
        if !path.is_dir() {
            println!(
                "FAIL {}: `pack` builds a pack directory, not an archive",
                path.display()
            );
            failed += 1;
            continue;
        }
        let built = match source::read(&path).and_then(|s| build(&s).map_err(|e| vec![e])) {
            Ok(b) => b,
            Err(problems) => {
                println!("FAIL {}:", path.display());
                for p in problems {
                    println!("  {p}");
                }
                failed += 1;
                continue;
            }
        };
        let skip = (!check_only).then_some(out.as_path());
        let checks = pins(&root, &built, skip);
        let bad: Vec<&PinCheck> = checks.iter().filter(|c| c.mismatch.is_some()).collect();
        if check_only {
            println!(
                "{} {} {}: {} bytes, SHA-256 {} (not written)",
                if bad.is_empty() { "ok" } else { "FAIL" },
                built.id,
                built.version,
                built.archive.len(),
                built.sha256
            );
        } else {
            fs::create_dir_all(&out)
                .map_err(|e| format!("cannot create {}: {e}", out.display()))?;
            let file = out.join(format!("{}.enthpack", built.id));
            fs::write(&file, &built.archive)
                .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
            println!(
                "{} {} {} -> {}: {} bytes, SHA-256 {}",
                if bad.is_empty() { "ok" } else { "FAIL" },
                built.id,
                built.version,
                file.display(),
                built.archive.len(),
                built.sha256
            );
        }
        for c in &checks {
            match &c.mismatch {
                None => println!("  pin {}: matches", c.pin),
                Some(why) => println!("  pin {}: MISMATCH: {why}", c.pin),
            }
        }
        if !bad.is_empty() {
            failed += 1;
        }
    }
    if failed == 0 {
        Ok(())
    } else {
        Err(format!("pack: {failed} pack(s) failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> Built {
        Built {
            id: "example".to_owned(),
            version: "0.1.0".to_owned(),
            archive: b"zip".to_vec(),
            sha256: "ab".repeat(32),
        }
    }

    #[test]
    fn the_catalogue_pins_a_hash_per_pack_version() {
        let b = built();
        let ok = format!("# release 0.4.0\n{}  example-0.1.0.enthpack\n", b.sha256);
        assert_eq!(catalogue_mismatch(&ok, &b), None);
        let other = format!(
            "{}  basics-1.0.0.enthpack\n{}  example-0.0.9.enthpack\n",
            "00".repeat(32),
            "11".repeat(32)
        );
        assert_eq!(
            catalogue_mismatch(&other, &b),
            None,
            "other packs and versions"
        );
        let hash = ok.replace(&b.sha256, &"cd".repeat(32));
        assert!(catalogue_mismatch(&hash, &b).unwrap().contains("SHA-256"));
        let twice = format!("{ok}{ok}");
        assert!(catalogue_mismatch(&twice, &b).unwrap().contains("2 times"));
        assert!(
            catalogue_mismatch("nothex  example-0.1.0.enthpack", &b)
                .unwrap()
                .contains("not a SHA-256")
        );
        assert!(catalogue_mismatch("lonely", &b).unwrap().contains("line 1"));
    }
}
