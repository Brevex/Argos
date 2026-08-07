# Recovery algorithms — rule and specifications

Argos-specific rules (`A-*` namespace). Index: [../../guidelines-index.md](../../guidelines-index.md).

## Algorithms are implemented from this spec (A-ALGORITHM-FROM-SPEC) { #A-ALGORITHM-FROM-SPEC }

<why>recovery techniques have precise invariants from the forensics literature; improvised
variants silently lose correctness, and undocumented variants cannot be reviewed.</why>

Every recovery technique is implemented from the specification below. Deviations update the spec in
the same change. Reviews compare code against this file. Each spec fixes: inputs, procedure,
invariants, outputs (always findings with `A-PROVENANCE` extents and an `A-CONFIDENCE-HONEST`
tier), and the source literature.

---

## Spec: acquisition (multi-pass imaging)

Copies a `BlockSource` into a raw image before analysis (stage A).

- **Procedure** (ddrescue-derived): pass 1 sweeps sequentially in fixed chunks; a failed chunk is
  zero-filled in the image and recorded as suspect, and the sweep moves on so a dying medium
  yields its healthy majority first. Pass 2 revisits each suspect chunk sector by sector, writing
  every sector that still reads at its absolute image offset; sectors that stay unreadable form
  the final unreadable map (sorted, merged, disjoint runs).
- **Invariants**: zeros written for unreadable sectors are placeholders and every such run appears
  in the report (`A-CONFIDENCE-HONEST` — zeros are never presented as read data); the destination
  is caller-chosen and must not resolve onto the source medium (`A-READ-ONLY`); the source is
  never retried destructively — reads only.
- **Output**: the image plus a report of total/recovered sectors and the unreadable runs.
- Reference: GNU ddrescue's sweep/refine strategy.

## Spec: residue sweep

Recovers the geometry of filesystems destroyed by re-formatting.

- **Input**: full device surface (sequential sweep, shared with stage D's single pass).
- **Procedure**: at each sector boundary test for volume anchors: NTFS boot sector (`0xEB 0x52 0x90
  "NTFS    "`, valid BPB sanity ranges), NTFS `FILE` record signature, ext2/3/4 superblock (magic
  `0xEF53` at offset 56 within the block, valid `s_log_block_size`, plausible counts) at primary
  and backup-superblock positions, APFS `NXSB` container superblock (checksummed), FAT/exFAT boot
  sectors. Group hits into hypothesized volumes: an anchor plus self-consistent geometry fields
  defines a candidate region `[start, start + size)`.
- **Invariants**: anchors are validated by internal consistency (checksums, sanity ranges), never
  by position; overlapping candidate volumes are all kept — later stages decide by yield.
- **Output**: candidate volumes tagged `residual`, fed to stage C parsers.

## Spec: NTFS deleted-file recovery

- Walk the live `$MFT`: records with the `FILE` signature and the in-use flag clear are deleted
  files; parse `$STANDARD_INFORMATION`, `$FILE_NAME` (name, timestamps), and `$DATA` (resident
  payload, or non-resident run list decoded with checked arithmetic — runs are signed deltas).
- Independently, signature-scan the surface for **orphaned `FILE` records** (records whose LBA is
  outside any known `$MFT`): after a re-format the new `$MFT` lands elsewhere, so old records
  survive in place. Validate by fixup array before trusting any field.
- Mine `$I30` index slack for directory entries of deleted files (names + parent references that
  may have no surviving MFT record).
- Parse `$UsnJrnl:$J` for delete records (names, timestamps, file reference numbers).
- On a live Windows source, enumerate Volume Shadow Copies as additional read-only block sources.
- **Tier**: `FsMetadata` when extents come from a validated record; names from `$I30`/`$UsnJrnl`
  alone attach as metadata to carved artifacts, they do not create extents.

## Spec: ext4 deleted-file recovery

- Read the primary superblock; on failure, backup superblocks (block group boundaries per the
  sparse_super layout).
- Deleted inodes in place have zeroed extent trees — do not bother. Instead scan the **jbd2
  journal**: descriptor blocks map journaled buffers to filesystem blocks; stale copies of
  inode-table blocks contain pre-deletion inodes whose extent trees are intact. Parse extent
  trees (checked depth ≤ 5, bounded entries) to extents.
- Carve directory-entry blocks (recognizable by `rec_len`/`name_len` self-consistency chains) for
  orphaned names.
- **Tier**: `JournalResidue` for journal-mined extents; `FsMetadata` only for extents from
  currently valid metadata (e.g. snapshot-like cases).

## Spec: FAT32/exFAT deleted-file recovery

- FAT32: directory entries starting `0xE5` are deleted; first cluster (high+low words) and size
  survive, the FAT chain does not. exFAT: entry sets with the in-use bit clear; `NoFatChain`
  entries retain exact extents.
- Assume contiguity from the start cluster for `ceil(size / cluster_size)` clusters — valid for
  most camera/SD writes. Hand the resulting byte range to the stage D validator; if it fails
  validation, demote to stage E with the start cluster as the known first fragment.
- **Tier**: `FsMetadata` only if validation passes end-to-end; otherwise the reassembly tier.

## Spec: APFS deleted-file recovery

- Parse the checkpoint descriptor area: it retains a ring of recent container superblocks
  (`NXSB`, Fletcher-64 checksummed). For each recent checkpoint, resolve its object map (omap
  B-tree) and volume superblocks, then walk filesystem-record B-trees.
- Diff inode sets across checkpoints: an inode present in checkpoint *n−k* but absent in *n* is
  recently deleted; its file extents are in the older tree. Enumerate local snapshots the same way.
- All B-tree walks are bounded (checked depth, node counts) per `A-UNTRUSTED-ONDISK`.
- **Tier**: `FsMetadata` (checkpoint data is genuine filesystem metadata).

## Spec: JPEG validation state machine

- States: `SOI` → marker-segment loop (`APPn`/`COM`/`DQT`/`DHT`/`DRI`/`SOF0|2`) → `SOS` →
  entropy-coded scan → (`RSTn` | `EOI`). Every segment length is bounds-checked against remaining
  candidate range before use.
- In the entropy-coded scan, `0xFF` must be followed by `0x00` (stuffing), `RSTn` in cyclic order
  when `DRI` set, or a legal marker; anything else marks a **fragmentation point** at the offset
  of the `0xFF` that introduced the violation, and the candidate is queued for stage E carrying
  the decoded-so-far state (the decode-state summary lands with stage E itself).
- EXIF `APP1` thumbnails are extracted as separate artifacts, tier `PartialOrThumbnail`. When the
  same extent also validates as a standalone contiguous image (the thumbnail carved directly out
  of a broken parent), the finding-merge rule applies: the merged finding keeps
  `ContiguousCarve` — it genuinely validated end to end — with the parent offset retained as
  provenance.
- Progressive JPEG: multiple `SOS` scans are legal; track `EOI` only.

## Spec: PNG validation state machine

- 8-byte signature → chunk walk: 4-byte length (bounds-checked, ≤ remaining range), type, data,
  CRC32 verified per chunk. `IHDR` first (dimensions sanity-capped), `IDAT` runs inflated
  incrementally (bounded output per step), `IEND` terminates.
- First CRC or inflate failure marks the fragmentation point; queue for stage E with the offset
  (an inflate failure localizes it exactly; a CRC mismatch cannot localize damage within its
  chunk, so the chunk data start is reported as the earliest offset corruption may begin) and
  the inflate state summary (carried once stage E lands).

## Spec: block classification

Cheap per-block features to shrink the reassembly search space:

- Shannon entropy over the block; byte-histogram chi-square distance to reference profiles.
- JPEG-stream detector: frequency of `0xFF 0x00` stuffing, absence of forbidden marker bytes,
  RST cadence.
- zlib/deflate detector: header check bytes, plausible Huffman block structure at offset 0.
- Output: block class ∈ {jpeg-stream, deflate, text/sparse, other-high-entropy, low-entropy} with
  a score. Classes gate which blocks enter reassembly graphs.
- Reference: Garfinkel (DFRWS 2007); Fitzgerald et al. on fragment classification.

## Spec: bifragment gap carving

For a candidate with header at `h`, fragmentation point at `h + k`, and a matching footer found at
`f > h + k`:

- Hypothesize gap `[g_start, g_end)` with `h + k ≤ g_start < g_end ≤ f`; test gaps in increasing
  size, bounded by a documented maximum (`MAX_BIFRAGMENT_GAP`), validating each hypothesis by
  resuming the real decoder across the splice.
- Accept the first hypothesis that decodes to the footer cleanly; tier `Reassembled`.
- Reference: Garfinkel, "Carving contiguous and fragmented files with fast object validation",
  DFRWS 2007.

## Spec: graph reassembly (PUP)

For candidates with > 2 fragments:

- Nodes: classified candidate blocks/extents. Edge weight between extents = decoder continuation
  success (hard gate) + pixel-row boundary smoothness (sum of absolute luminance differences
  across the stitch row; lower is better).
- Parallel Unique Path: grow the best path per header greedily, all headers in parallel, each
  extent assignable to one path (matching Pal/Sencar/Memon's SmartCarver reassembly).
- JPEG restart markers are independent entry points: an orphaned fragment starting at an `RSTn`
  boundary is decodable standalone (Uzun & Sencar) and joins the graph as a first-class node.
- All outputs tier `Reassembled`, with every extent recorded per `A-PROVENANCE`.
- References: Pal & Memon, "The evolution of file carving", IEEE SPM 2009; Pal, Sencar, Memon,
  DFRWS 2008; Uzun & Sencar 2015.

## Spec: confidence tiers and finding merge

- Tier ladder: `FsMetadata > JournalResidue > ContiguousCarve > Reassembled >
  PartialOrThumbnail`. Assignment rules per spec above; no post-hoc promotion
  (`A-CONFIDENCE-HONEST`).
- Merge: findings from stages C and D/E are deduplicated by (a) overlapping source extents, then
  (b) content hash. On a duplicate, keep the higher tier and attach the other's metadata (e.g. a
  carved artifact gains the filename from an `$I30` hit over the same extents).
