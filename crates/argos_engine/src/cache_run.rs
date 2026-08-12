//! Recognising a thumbnail cache from how its entries sit on the medium.
//!
//! A desktop or a phone keeps previews of every picture it has shown, in one
//! file, written once. That file survives long after the photographs it
//! describes are overwritten, so a recovery of a used disk turns up far more
//! cache entries than photographs — and each one looks, on its own, exactly
//! like a small photograph, because that is what it is a copy of.
//!
//! What gives a cache away is not any one entry but the run: a cache writes
//! one size, so its entries share dimensions to the pixel and sit next to each
//! other. Measured on a 1 TB disk of ten years' use, 51 of 60 artifacts within
//! four megabytes of one offset were exactly 256x192.
//!
//! Naming it is what stops the report from presenting a preview of a lost
//! photograph as the photograph (`A-CONFIDENCE-HONEST`). Nothing here removes
//! or reclassifies anything: it counts neighbours and says how many.

use argos_core::artifact::Digest;

/// Artifacts that must share dimensions in a row before the run is a cache.
///
/// Two pictures of one size are a coincidence and three are a set; a cache
/// holds hundreds. The threshold sits low enough to catch a small one and high
/// enough that a burst of photographs from one camera — which vary in
/// orientation, and whose sizes therefore alternate — never reaches it.
const MIN_RUN: usize = 8;

/// How far apart two entries of one cache may sit and still be one run.
///
/// Entries of a cache file are consecutive; the slack is for the records
/// between them and for entries a scan could not recover.
const MAX_GAP_BYTES: u64 = 4 * 1024 * 1024;

/// One artifact, as this pass needs to see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    /// Where the artifact starts on the medium.
    pub offset: u64,
    /// Its decoded dimensions, when it decoded.
    pub pixels: Option<(u32, u32)>,
    /// Content hash, which is how the manifest is told about it.
    pub sha256: Digest,
}

/// One artifact found among same-sized neighbours, and how many there were.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheRun {
    /// Content hash of the artifact, which is how a manifest is told.
    pub sha256: Digest,
    /// How many artifacts of identical dimensions the run held, this one
    /// included. A large number is a thumbnail cache; there is no size at
    /// which it becomes a verdict about any single picture.
    pub neighbours: u32,
}

/// Every artifact that belongs to a run of same-sized neighbours, with the
/// size of the run it belongs to.
///
/// `entries` are expected in medium order, which is the order the report stage
/// produces them in.
pub(crate) fn runs(entries: &[Entry]) -> Vec<CacheRun> {
    let mut found = Vec::new();
    let mut start = 0;
    while start < entries.len() {
        let Some(pixels) = entries[start].pixels else {
            start += 1;
            continue;
        };
        let mut end = start + 1;
        while end < entries.len()
            && entries[end].pixels == Some(pixels)
            && entries[end].offset.saturating_sub(entries[end - 1].offset) <= MAX_GAP_BYTES
        {
            end += 1;
        }
        let length = end - start;
        if length >= MIN_RUN {
            let size = u32::try_from(length).unwrap_or(u32::MAX);
            found.extend(entries[start..end].iter().map(|entry| CacheRun {
                sha256: entry.sha256,
                neighbours: size,
            }));
        }
        start = end;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{Entry, MIN_RUN, runs};
    use argos_core::artifact::Digest;

    fn entry(offset: u64, pixels: Option<(u32, u32)>, tag: u64) -> Entry {
        Entry {
            offset,
            pixels,
            sha256: Digest::new([u8::try_from(tag % 256).unwrap_or(0); Digest::LEN]),
        }
    }

    #[test]
    fn a_run_of_identical_sizes_is_named_with_its_length() {
        // The shape measured on a real disk: a stretch of one size, packed.
        let entries: Vec<_> = (0..40)
            .map(|index| entry(1_000_000 + index * 8_000, Some((256, 192)), index))
            .collect();
        let found = runs(&entries);
        assert_eq!(found.len(), 40, "every entry of the run is named");
        assert!(found.iter().all(|run| run.neighbours == 40));
    }

    #[test]
    fn a_handful_of_one_size_is_not_a_cache() {
        let entries: Vec<_> = (0..MIN_RUN as u64 - 1)
            .map(|index| entry(index * 5_000, Some((640, 480)), index))
            .collect();
        assert!(
            runs(&entries).is_empty(),
            "a few photographs of one size are a coincidence, not a cache"
        );
    }

    #[test]
    fn photographs_between_two_caches_are_left_alone() {
        let mut entries: Vec<_> = (0..12)
            .map(|index| entry(index * 4_000, Some((258, 258)), index))
            .collect();
        entries.push(entry(60_000, Some((4128, 3096)), 200));
        entries.extend(
            (0..10).map(|index| entry(80_000 + index * 4_000, Some((258, 258)), 100 + index)),
        );

        let found = runs(&entries);
        let named: Vec<_> = found.iter().map(|run| run.sha256).collect();
        assert_eq!(
            found.len(),
            22,
            "both runs are named, the photograph is not"
        );
        assert!(
            !named.contains(&Digest::new([200; Digest::LEN])),
            "the camera frame between two caches is not part of either"
        );
    }

    #[test]
    fn a_distant_neighbour_starts_a_new_run() {
        let mut entries: Vec<_> = (0..10)
            .map(|index| entry(index * 1_000, Some((96, 96)), index))
            .collect();
        // Far enough away to be another file entirely.
        entries.extend((0..3).map(|index| {
            entry(
                500 * 1024 * 1024 + index * 1_000,
                Some((96, 96)),
                50 + index,
            )
        }));
        let found = runs(&entries);
        assert_eq!(
            found.len(),
            10,
            "the three on their own are not a run: {found:?}"
        );
    }

    #[test]
    fn an_artifact_that_did_not_decode_belongs_to_no_run() {
        let mut entries: Vec<_> = (0..10)
            .map(|index| entry(index * 1_000, Some((64, 64)), index))
            .collect();
        entries.insert(5, entry(4_500, None, 99));
        let found = runs(&entries);
        assert!(
            !found
                .iter()
                .any(|run| run.sha256 == Digest::new([99; Digest::LEN])),
            "a size nobody measured cannot match a size"
        );
    }
}
