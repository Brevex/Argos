//! Recall against corpora whose answers someone else published.
//!
//! Every other measurement in this workspace plants its own images and then
//! finds them, which proves the search handles layouts this project
//! constructed. It cannot show that the layouts are the ones a real allocator
//! produces, and a search tuned against its own fixtures is the failure mode a
//! recovery rate is least able to reveal.
//!
//! A published corpus removes the circularity: the fragmentation was designed
//! by someone with no stake in this tool, and the answers are known
//! independently of what this tool reports.
//!
//! # Supplying a corpus
//!
//! The data is external — too large to commit and not this project's to
//! redistribute — so the harness reads it from `ARGOS_CORPUS_DIR`. Without that
//! variable the suite reports what it would need and passes: an absent corpus
//! is a corpus that was not supplied, never a failing search.
//!
//! For each case the directory holds two files sharing a stem:
//!
//! ```text
//! dfrws-2006.raw      the disk image to scan
//! dfrws-2006.sha256   the answers, in `sha256sum` format
//! ```
//!
//! The answer file is one line per file known to be recoverable from the
//! image, `<64 hex digits><whitespace><name>` — what `sha256sum *` writes over
//! a directory of the extracted originals. The name is used only to report a
//! miss; recall is decided by digest alone, so a recovery counts only when its
//! bytes reproduce the original exactly.
//!
//! Corpora this harness is meant for: the DFRWS 2006 and 2007 carving
//! challenges, and the NIST `CFReDS` file-carving cases FC-01 through FC-05,
//! whose fragmented and non-cluster-aligned scenarios are the ones this
//! project's own fixtures do not pose.
//!
//! # What is asserted
//!
//! Against a supplied corpus the harness measures and prints; it does not fail
//! a run, because no floor has been established for data this suite has never
//! seen, and inventing one would assert a number rather than measure it. What
//! *is* asserted, on every run, is that the harness itself distinguishes a
//! recovery from a miss — see the self-test below. A measuring instrument that
//! is never checked reports whatever it likes.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use argos_core::ports::{Digest, Discard};
use argos_engine::fixture::Collector;
use argos_engine::{Medium, ScanConfig, ScanSession};

/// Environment variable naming the directory of image/answer pairs.
const CORPUS_DIR: &str = "ARGOS_CORPUS_DIR";

/// One file the corpus says is recoverable, and whether it came back.
struct Expected {
    /// Name from the answer file. Reported on a miss so a gap is nameable.
    name: String,
    /// Whether some recovered artifact reproduced this digest.
    found: bool,
}

/// What one case measured.
struct Recall {
    case: String,
    expected: Vec<Expected>,
    /// Artifacts the scan produced, whether or not the answers name them.
    recovered: usize,
}

impl Recall {
    fn hits(&self) -> usize {
        self.expected.iter().filter(|file| file.found).count()
    }

    fn percent(&self) -> usize {
        if self.expected.is_empty() {
            return 0;
        }
        self.hits() * 100 / self.expected.len()
    }
}

/// Parses `sha256sum` output into digest → name.
///
/// A line this cannot read is a malformed answer file, which is a fault in the
/// corpus rather than in the tool, so it is reported by position and skipped
/// rather than silently dropped.
fn answers(text: &str) -> Result<BTreeMap<Digest, String>, String> {
    let mut wanted = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hex, name) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| format!("line {}: no name after the digest", index + 1))?;
        let digest = parse_digest(hex)
            .ok_or_else(|| format!("line {}: {hex:?} is not sixty-four hex digits", index + 1))?;
        wanted.insert(digest, name.trim_start().trim_start_matches('*').to_owned());
    }
    Ok(wanted)
}

/// Reads sixty-four hex digits into a digest, or `None` for anything else.
fn parse_digest(hex: &str) -> Option<Digest> {
    if hex.len() != Digest::LEN * 2 {
        return None;
    }
    let mut bytes = [0_u8; Digest::LEN];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let pair = hex.get(index * 2..index * 2 + 2)?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(Digest::new(bytes))
}

/// Scans `image` and reports which of `wanted` came back byte-identical.
///
/// Recall is decided by the digest the pipeline recorded at recovery time, so
/// a file counts only when the bytes handed to the sink reproduce the original
/// exactly — a partial prefix of the right photograph is not a recovery of it.
fn measure(case: &str, image: &[u8], wanted: &BTreeMap<Digest, String>) -> Recall {
    let workers = 4;
    let config = ScanConfig::builder()
        .workers(NonZeroUsize::new(workers).expect("at least one worker"))
        // The corpora hold thumbnails and sprites among the photographs, and
        // the answers decide what counts; a size floor would decide it first.
        .min_long_side(0)
        // A published corpus is small enough to search exhaustively, and a
        // recall measured under a clock would measure the clock.
        .reassembly_budget(None)
        .build()
        .expect("valid configuration");

    let views: Vec<_> = (0..workers).map(|_| Cursor::new(image.to_vec())).collect();
    let medium = Medium::new(views, image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    ScanSession::new(config)
        .start(medium, &mut sink, &Discard)
        .expect("scan");

    let got: Vec<Digest> = sink
        .artifacts()
        .iter()
        .map(|artifact| artifact.sha256)
        .collect();
    Recall {
        case: case.to_owned(),
        expected: wanted
            .iter()
            .map(|(digest, name)| Expected {
                name: name.clone(),
                found: got.contains(digest),
            })
            .collect(),
        recovered: got.len(),
    }
}

/// The image beside an answer file, whatever raw extension it carries.
fn image_beside(answers: &Path) -> Option<PathBuf> {
    ["raw", "dd", "img", "bin"].into_iter().find_map(|ext| {
        let candidate = answers.with_extension(ext);
        candidate.is_file().then_some(candidate)
    })
}

#[test]
fn recall_against_published_corpora() {
    let Ok(dir) = std::env::var(CORPUS_DIR) else {
        println!(
            "no {CORPUS_DIR} in the environment, so no corpus was measured.\n\
             Supply one as image/answer pairs sharing a stem — `case.raw` beside\n\
             `case.sha256` in `sha256sum` format — and set {CORPUS_DIR} to the\n\
             directory. The DFRWS 2006 and 2007 carving challenges and the NIST\n\
             CFReDS file-carving cases are what this measures."
        );
        return;
    };

    let dir = PathBuf::from(dir);
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{CORPUS_DIR} is {}: {error}", dir.display()));

    let mut measured = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "sha256") {
            continue;
        }
        let case = path.file_stem().map_or_else(
            || "?".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        let Some(image) = image_beside(&path) else {
            println!("{case}: answers with no image beside them, skipped");
            continue;
        };
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let wanted = answers(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let bytes =
            std::fs::read(&image).unwrap_or_else(|error| panic!("{}: {error}", image.display()));
        measured.push(measure(&case, &bytes, &wanted));
    }

    assert!(
        !measured.is_empty(),
        "{CORPUS_DIR} is {} and holds no `.sha256` answer file beside an image",
        dir.display()
    );

    for recall in &measured {
        println!(
            "{:<20} {:>3}%  {} of {} known files, {} artifacts recovered in all",
            recall.case,
            recall.percent(),
            recall.hits(),
            recall.expected.len(),
            recall.recovered
        );
        for missed in recall.expected.iter().filter(|file| !file.found) {
            println!("  missed  {}", missed.name);
        }
    }
}

/// A measuring instrument that is never checked reports whatever it likes.
///
/// Two planted photographs, one of which is removed from the medium before the
/// scan: the harness must find the one that is there and name the one that is
/// not. Both halves matter — an instrument that reports everything found and
/// one that reports nothing found are equally useless, and only checking both
/// directions separates them from a working one.
#[test]
fn the_harness_reports_a_recovery_as_found_and_an_absent_file_as_missed() {
    let present = argos_carve::fixture::photo_jpeg(320, 240, 0xC0F5_0000_0000_0001);
    let absent = argos_carve::fixture::photo_jpeg(288, 216, 0xC0F5_0000_0000_0002);

    let chunk = argos_engine::config::MIN_CHUNK_BYTES;
    let disk = argos_carve::fixture::Disk::filled(chunk * 4)
        .with(chunk, &present)
        .into_bytes();

    let wanted = BTreeMap::from([
        (sha256_of(&present), "present.jpg".to_owned()),
        (sha256_of(&absent), "absent.jpg".to_owned()),
    ]);

    let recall = measure("self-test", &disk, &wanted);

    let found: Vec<&str> = recall
        .expected
        .iter()
        .filter(|file| file.found)
        .map(|file| file.name.as_str())
        .collect();
    assert_eq!(
        found,
        ["present.jpg"],
        "the harness must credit exactly the file the medium holds"
    );
    assert_eq!(
        recall.percent(),
        50,
        "one of two known files is half of them"
    );
}

/// Digest of `bytes`, the same way the pipeline computes an artifact's.
fn sha256_of(bytes: &[u8]) -> Digest {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let mut out = [0_u8; Digest::LEN];
    out.copy_from_slice(&hasher.finalize());
    Digest::new(out)
}

#[test]
fn an_answer_file_is_read_as_sha256sum_writes_it() {
    let text = "\
# a comment, and a blank line follow

0000000000000000000000000000000000000000000000000000000000000001  plain.jpg
0000000000000000000000000000000000000000000000000000000000000002 *binary-mode.jpg
";
    let wanted = answers(text).expect("well-formed answers");
    let names: Vec<&String> = wanted.values().collect();
    assert_eq!(
        names,
        ["plain.jpg", "binary-mode.jpg"],
        "both of sha256sum's modes are read, and the `*` is not part of a name"
    );
    // A malformed answer is a fault in the corpus and is reported rather than
    // dropped: a silently shortened answer list flatters the recall.
    answers("nothexdigits  x.jpg").unwrap_err();
    answers("0000000000000000000000000000000000000000000000000000000000000001").unwrap_err();
}
