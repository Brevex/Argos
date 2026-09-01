//! No write path to a source medium exists, checked rather than reviewed.
//!
//! [`BlockSource`](argos_core::ports::BlockSource) has no write, discard or
//! passthrough method, so a *caller* cannot modify a medium and the compiler is
//! what says so. That leaves the one thing the port's shape cannot reach: this
//! crate's own syscall layer, which is where a handle is actually opened. The
//! three opens there are read-only — `linux.rs` asks for `O_DIRECT` over
//! `.read(true)`, `macos.rs` for `O_RDONLY`, `windows.rs` for `GENERIC_READ` —
//! and nothing fails if a fourth is added that is not.
//!
//! So this reads the crate's own sources and refuses the spellings that grant
//! write access. It is a floor rather than a proof: a raw `libc::open` handed a
//! flag integer computed at run time would pass it. What it covers is every way
//! the three current opens are written, which is how a fourth would be written.
//!
//! The acquisition *destination* is written outside this crate, by the binary,
//! which is why the crate can be held to zero rather than to a list of
//! exceptions.

use std::fs;
use std::path::{Path, PathBuf};

/// Spellings that grant write access, and what each would do to a medium.
///
/// Comment lines are exempt: a doc comment that says a handle is never opened
/// `GENERIC_WRITE` is the guarantee being described, not a breach of it.
const FORBIDDEN: &[(&str, &str)] = &[
    (".write(true)", "opens a file for writing"),
    (".append(true)", "opens a file for appending"),
    (".create(true)", "creates a file, and grants write to do it"),
    (".create_new(", "creates a file, and grants write to do it"),
    (".truncate(true)", "empties a file"),
    ("File::create", "creates or truncates a file"),
    ("O_RDWR", "opens a descriptor for reading and writing"),
    ("O_WRONLY", "opens a descriptor for writing"),
    ("O_TRUNC", "empties a file on open"),
    ("GENERIC_WRITE", "opens a Windows handle for writing"),
    ("FILE_WRITE_DATA", "grants write access to a Windows handle"),
    ("fs::write", "replaces a file's contents"),
    ("fs::remove", "deletes a file or directory"),
    ("fs::create_dir", "creates a directory"),
];

/// Every `.rs` file under `dir`, recursively.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {} failed: {e}", dir.display()))
        .map(|e| e.expect("reading a directory entry").path());
    for path in entries {
        if path.is_dir() {
            found.extend(sources(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
    found
}

#[test]
#[cfg_attr(miri, ignore = "reads the crate's own source files")]
fn no_open_in_this_crate_asks_for_write_access() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = sources(&src);
    assert!(
        !files.is_empty(),
        "found no sources under {} — this check would pass vacuously",
        src.display()
    );

    let mut breaches = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("reading {} failed: {e}", file.display()));
        for (number, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for (token, effect) in FORBIDDEN {
                if line.contains(token) {
                    breaches.push(format!(
                        "{}:{}: `{token}` {effect}\n    {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        breaches.is_empty(),
        "{} line(s) in {} source files of this crate ask an operating system \
         for write access. No write path to a source medium may exist: devices are \
         opened read-only at the lowest layer, and this crate is that layer. \
         If a write is genuinely needed here, it belongs to the binary — the \
         acquisition destination is written there for this reason.\n\n{}",
        breaches.len(),
        files.len(),
        breaches.join("\n")
    );
}
