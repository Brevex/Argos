//! Copying selected artifacts out of a session directory.
//!
//! An export is the step where recovered evidence leaves the tool, so it
//! verifies rather than trusts: every artifact is re-hashed while it is copied
//! and compared against the digest the scan recorded. A file whose bytes no
//! longer reproduce that digest is **not** exported. It is named in the result
//! instead, because "this artifact changed on disk since the scan" is
//! information an examiner needs, and quietly copying it anyway would put an
//! unattributable file in the destination (A-PROVENANCE).

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;
use argos_report::{ArtifactRecord, Manifest};
use sha2::{Digest, Sha256};

/// Bytes copied per streaming step while hashing.
const COPY_CHUNK_BYTES: usize = 64 * 1024;

/// Shortest artifact-hash prefix a selection may use.
///
/// Eight hex digits is 32 bits: enough that a collision inside one session's
/// artifacts is not something a user will meet by accident, and short enough
/// to type. An ambiguous prefix is refused rather than resolved (see
/// [`select`]).
const MIN_PREFIX_LEN: usize = 8;

/// What an export produced.
#[derive(Clone, Debug, Default)]
pub struct Exported {
    /// Artifacts copied and verified.
    pub copied: Vec<String>,
    /// Preview files copied alongside them.
    pub previews: u64,
    /// Artifacts whose stored bytes no longer reproduce the manifest's digest,
    /// and were therefore not copied.
    pub tampered: Vec<String>,
    /// Artifacts named in the manifest whose file is missing from the session
    /// directory.
    pub missing: Vec<String>,
    /// Artifacts the scan recognised and was told not to write, by digest.
    /// Their bytes are still on the medium at the extents the manifest
    /// records; nothing here is damage.
    pub omitted: Vec<String>,
}

/// Copies the selected artifacts from session directory `from` into `to`.
///
/// An empty `wanted` exports everything the manifest records.
///
/// # Errors
///
/// Fails when the manifest cannot be read, when a selection matches no record
/// or more than one, when the destination cannot be created, or when a copy
/// fails for a reason other than the artifact itself being absent or altered.
pub fn run(from: &Path, to: &Path, wanted: &[String]) -> anyhow::Result<Exported> {
    let manifest = Manifest::read(from)
        .with_context(|| format!("cannot read the session manifest in {}", from.display()))?;
    let chosen = select(&manifest, wanted)?;

    fs::create_dir_all(to)
        .with_context(|| format!("cannot create destination {}", to.display()))?;

    let mut exported = Exported::default();
    let mut kept: Vec<ArtifactRecord> = Vec::with_capacity(chosen.len());
    for record in chosen {
        // A record with no file is an artifact the scan recognised and was
        // told not to write. It is not missing — it is on the medium, at the
        // extents the record carries — so it is reported as omitted rather
        // than as damage.
        let Some(name) = record.name.clone() else {
            exported.omitted.push(record.sha256.clone());
            continue;
        };
        let source = from.join(&name);
        if !source.is_file() {
            exported.missing.push(name);
            continue;
        }
        let digest = copy_verified(&source, &to.join(&name))
            .with_context(|| format!("cannot export {name}"))?;
        if digest != record.sha256 {
            // The copy is already written; remove it rather than leave a file
            // in the destination that the exported manifest does not describe.
            let _ = fs::remove_file(to.join(&name));
            exported.tampered.push(name);
            continue;
        }

        if let Some(preview) = &record.preview
            && copy_preview(from, to, preview).is_ok()
        {
            exported.previews += 1;
        }
        exported.copied.push(name);
        kept.push(record.clone());
    }

    // The destination gets a manifest of exactly what landed in it — an export
    // that carried the whole session's manifest would describe artifacts that
    // are not there.
    let subset = Manifest {
        tool_version: manifest.tool_version.clone(),
        source: manifest.source.clone(),
        scan_state: manifest.scan_state.clone(),
        rejected_candidates: manifest.rejected_candidates,
        unreadable: manifest.unreadable.clone(),
        triage: manifest.triage.clone(),
        artifacts: kept,
    };
    subset
        .write(to)
        .with_context(|| format!("cannot write the export manifest in {}", to.display()))?;

    Ok(exported)
}

/// The records `wanted` names, in manifest order.
///
/// Each entry is a full artifact hash or an unambiguous prefix of at least
/// [`MIN_PREFIX_LEN`] hex digits. A prefix matching several artifacts is an
/// error naming them: guessing which one was meant would export the wrong
/// evidence.
fn select<'a>(
    manifest: &'a Manifest,
    wanted: &[String],
) -> anyhow::Result<Vec<&'a ArtifactRecord>> {
    if wanted.is_empty() {
        return Ok(manifest.artifacts.iter().collect());
    }

    let mut chosen: Vec<&ArtifactRecord> = Vec::with_capacity(wanted.len());
    for hash in wanted {
        let hash = hash.to_ascii_lowercase();
        anyhow::ensure!(
            hash.len() >= MIN_PREFIX_LEN,
            "{hash} is too short to name an artifact; use at least {MIN_PREFIX_LEN} hex digits \
             of its SHA-256"
        );
        let matched: Vec<&ArtifactRecord> = manifest
            .artifacts
            .iter()
            .filter(|record| record.sha256.starts_with(&hash))
            .collect();
        match matched.as_slice() {
            [record] => chosen.push(record),
            [] => anyhow::bail!("no artifact in this session has a hash starting {hash}"),
            several => anyhow::bail!(
                "{hash} names {} artifacts ({}); use more of the hash",
                several.len(),
                several
                    .iter()
                    .map(|record| record.sha256.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    // Manifest order, and each artifact at most once however it was named.
    let mut ordered: Vec<&ArtifactRecord> = manifest
        .artifacts
        .iter()
        .filter(|record| chosen.iter().any(|picked| picked.sha256 == record.sha256))
        .collect();
    ordered.dedup_by(|left, right| left.sha256 == right.sha256);
    Ok(ordered)
}

/// Copies `from` to `to`, returning the SHA-256 of what was copied.
fn copy_verified(from: &Path, to: &Path) -> anyhow::Result<String> {
    let mut input = File::open(from).with_context(|| format!("cannot read {}", from.display()))?;
    let mut output = File::create(to).with_context(|| format!("cannot write {}", to.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; COPY_CHUNK_BYTES];
    loop {
        let read = input
            .read(&mut buf)
            .with_context(|| format!("cannot read {}", from.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        std::io::Write::write_all(&mut output, &buf[..read])
            .with_context(|| format!("cannot write {}", to.display()))?;
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").unwrap_or_else(|err| {
            unreachable!("writing hex into a String cannot fail: {err}");
        });
    }
    Ok(hex)
}

/// Copies one preview file, keeping its position relative to the session
/// directory.
///
/// Previews are derived, so this is best-effort: a missing or unreadable one
/// costs a thumbnail, and the artifact it belongs to is already exported and
/// verified.
fn copy_preview(from: &Path, to: &Path, relative: &str) -> std::io::Result<()> {
    // A manifest is a file this tool wrote, but it is still a file on disk
    // that something else may have edited. A preview path that climbs out of
    // the session directory would make an export write wherever it pointed.
    let relative = PathBuf::from(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(std::io::Error::other("preview path escapes the session"));
    }
    let target = to.join(&relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from.join(&relative), target).map(|_| ())
}
