//! Copying selected artifacts out of a session directory.
//!
//! An export is the step where recovered evidence leaves the tool, so it
//! verifies rather than trusts: every artifact is re-hashed while it is copied
//! and compared against the digest the scan recorded. A file whose bytes no
//! longer reproduce that digest is **not** exported. It is named in the result
//! instead, because "this artifact changed on disk since the scan" is
//! information an examiner needs, and quietly copying it anyway would put an
//! unattributable file in the destination (A-PROVENANCE).
//!
//! Where each recorded artifact stands is decided here too, in one place,
//! because three readers need the same answer — the report, the export filter
//! and the gallery — and three implementations of an ordering would be three
//! orderings (`A-ONE-IMPLEMENTATION`).
//!
//! A session written before standings existed carries none. It is still
//! ordered, and correctly: a standing is derived from dimensions, camera,
//! capture date and same-size neighbours, and a manifest records all four
//! beside every artifact. So an older session sorts exactly as a new one does,
//! without re-reading the medium and without a migration.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;
use argos_classify::rank::{Evidence, Standing};
use argos_core::artifact::Digest;
use argos_report::{ArtifactRecord, Manifest};
use sha2::{Digest as _, Sha256};

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

/// Which of a session's artifacts to export.
///
/// A scan of a used disk produces hundreds of thousands of artifacts, so
/// naming one by its hash only helps a person who already knows which one they
/// want. These are the questions someone looking for their own photographs
/// actually asks: which camera, what year, how big.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Artifact hashes, or unambiguous prefixes of them. Empty means every
    /// artifact the other criteria admit.
    pub hashes: Vec<String>,
    /// Smallest long side, in pixels, an artifact is exported for.
    pub min_long_side: Option<u32>,
    /// Substring the recorded camera make or model must contain, matched
    /// without regard to case.
    pub camera: Option<String>,
    /// Earliest and latest capture date to export, each compared as the text
    /// EXIF stores — `YYYY:MM:DD HH:MM:SS` sorts chronologically as it is, so
    /// a prefix like `2009` is a whole year and needs no date arithmetic.
    pub taken_from: Option<String>,
    pub taken_until: Option<String>,
    /// Weakest standing an artifact may have and still be exported.
    ///
    /// This is the "give me the photographs" filter: the evidence a picture
    /// carries about itself, in the order the report lists it. A session that
    /// recorded no standing is not waived — the standing is derived from the
    /// record's own fields, so an older session filters as a new one does.
    pub standing: Option<argos_classify::rank::Standing>,
}

impl Filter {
    /// Whether the filter names any criterion at all.
    fn is_empty(&self) -> bool {
        self.hashes.is_empty()
            && self.min_long_side.is_none()
            && self.camera.is_none()
            && self.taken_from.is_none()
            && self.taken_until.is_none()
            && self.standing.is_none()
    }

    /// Whether `record` satisfies every criterion but the hashes.
    ///
    /// An artifact that recorded nothing about its camera or its date is kept
    /// unless a criterion asks about exactly that: absent is not a match, but
    /// it is also not evidence of anything, and silently dropping every
    /// undescribed picture would hide most of a carved disk.
    fn admits(&self, record: &ArtifactRecord) -> bool {
        if let Some(floor) = self.min_long_side {
            let long_side = long_side(record);
            // Nothing measured clears the floor, as it does during a scan: a
            // decoder that gave up is not evidence the bytes are worthless.
            if long_side > 0 && long_side < floor {
                return false;
            }
        }
        if let Some(floor) = self.standing {
            // Derived when the session did not record one, so an older
            // session filters exactly as a new one does.
            if standing_of(record) < floor {
                return false;
            }
        }
        if let Some(camera) = &self.camera {
            let wanted = camera.to_lowercase();
            let named = |value: &Option<String>| {
                value
                    .as_ref()
                    .is_some_and(|value| value.to_lowercase().contains(&wanted))
            };
            if !named(&record.camera_make) && !named(&record.camera_model) {
                return false;
            }
        }
        if self.taken_from.is_some() || self.taken_until.is_some() {
            let Some(taken) = &record.taken else {
                return false;
            };
            if self.taken_from.as_ref().is_some_and(|from| taken < from) {
                return false;
            }
            if self
                .taken_until
                .as_ref()
                .is_some_and(|until| taken.as_str() > until.as_str() && !taken.starts_with(until))
            {
                return false;
            }
        }
        true
    }
}

/// Copies the selected artifacts from session directory `from` into `to`.
///
/// An empty `filter` exports everything the manifest records.
///
/// # Errors
///
/// Fails when the manifest cannot be read, when a hash selection matches no
/// record or more than one, when the destination cannot be created, or when a
/// copy fails for a reason other than the artifact itself being absent or
/// altered.
pub fn run(from: &Path, to: &Path, filter: &Filter) -> anyhow::Result<Exported> {
    let manifest = Manifest::read(from)
        .with_context(|| format!("cannot read the session manifest in {}", from.display()))?;
    let chosen = select(&manifest, filter)?;

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
        // Both describe the run that produced these files rather than the
        // files, and both stay true of a subset: a reader of the export still
        // needs to know the scan omitted eleven thousand artifacts under its
        // floor, and which volumes it found them among (A-CONFIDENCE-HONEST).
        coverage: manifest.coverage.clone(),
        volumes: manifest.volumes.clone(),
        // An export describes the files it carried, and a fragmentation point
        // is a place on the source medium rather than a file. It stays with
        // the session that can still reach that medium. So does a lost file,
        // which is a place on that medium and no file at all.
        fragmentation: Vec::new(),
        lost_files: Vec::new(),
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
fn select<'a>(manifest: &'a Manifest, filter: &Filter) -> anyhow::Result<Vec<&'a ArtifactRecord>> {
    if filter.is_empty() {
        return Ok(manifest.artifacts.iter().collect());
    }
    if filter.hashes.is_empty() {
        return Ok(manifest
            .artifacts
            .iter()
            .filter(|record| filter.admits(record))
            .collect());
    }

    let wanted = &filter.hashes;
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
        .filter(|record| filter.admits(record))
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
    Ok(Digest::new(hasher.finalize().into()).to_string())
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

/// The standing of `record`: the one the scan recorded, or the one its own
/// fields imply.
///
/// Deriving rather than defaulting is what makes this work on a session the
/// running version did not produce. The two agree by construction — the scan
/// computes it from the same four facts it then writes down.
#[must_use]
pub fn standing_of(record: &ArtifactRecord) -> Standing {
    if let Some(recorded) = record
        .standing
        .as_deref()
        .and_then(|text| text.parse::<Standing>().ok())
    {
        return recorded;
    }

    // The picture's measured size, falling back to what its metadata declares
    // when the decode produced nothing — exactly as the export filter does.
    let pixels = match (
        record.width.or(record.declared_width),
        record.height.or(record.declared_height),
    ) {
        (Some(width), Some(height)) => Some((width, height)),
        _ => None,
    };
    let evidence = Evidence {
        pixels,
        camera_named: record.camera_make.is_some() || record.camera_model.is_some(),
        dated: record.taken.is_some(),
        same_size_neighbours: record.same_size_neighbours,
    };
    argos_classify::rank::standing(&evidence)
}

/// Sort order of a record's standing, strongest last.
#[must_use]
pub fn rank(record: &ArtifactRecord) -> u8 {
    match standing_of(record) {
        Standing::CacheNeighbour => 0,
        Standing::Unremarkable => 1,
        Standing::PhotographSized => 2,
        Standing::Dated => 3,
        Standing::CameraNamed => 4,
    }
}

/// Long side of a record's picture, zero when nothing said what it was.
#[must_use]
pub fn long_side(record: &ArtifactRecord) -> u32 {
    record
        .width
        .unwrap_or(0)
        .max(record.height.unwrap_or(0))
        .max(record.declared_width.unwrap_or(0))
        .max(record.declared_height.unwrap_or(0))
}

/// Every artifact a reader should see, strongest evidence first.
///
/// Strongest standing, then the largest picture, then the longest run of bytes,
/// and finally the content hash. That last key is what makes the order *total*:
/// a paged gallery asks for the same list twice and must not shuffle equal
/// artifacts between the pages, and `A-CLI-FIRST` forbids the report and the
/// wire disagreeing about what "first" means.
///
/// The order is presentation only — the manifest's own order is untouched.
#[must_use]
pub fn ordered(
    manifest: &Manifest,
    standing: Option<Standing>,
    include_unwritten: bool,
) -> Vec<&ArtifactRecord> {
    let admits = |record: &&ArtifactRecord| {
        if !include_unwritten && !record.written {
            return false;
        }
        // A session written before standings existed carries none; the standing
        // is derived from its own record rather than waived, so an older session
        // filters exactly as a new one does.
        standing.is_none_or(|floor| standing_of(record) >= floor)
    };
    let mut chosen: Vec<&ArtifactRecord> = manifest.artifacts.iter().filter(admits).collect();
    chosen.sort_by(|left, right| {
        rank(right)
            .cmp(&rank(left))
            .then(long_side(right).cmp(&long_side(left)))
            .then(right.length.cmp(&left.length))
            .then(left.sha256.cmp(&right.sha256))
    });
    chosen
}
