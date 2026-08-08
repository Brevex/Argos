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
- **Output**: candidate volumes tagged `residual`, fed to stage C parsers, plus the regions
  holding orphaned NTFS `FILE` records (recognised by signature *and* a verifying fixup array),
  fed to the NTFS orphan scan together with the geometry of the volume they belong to — run lists
  are volume-relative, so the wrong volume start yields extents pointing at the wrong bytes.

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
  validation, demote to stage E with the start cluster as the known first fragment. Every
  reconstructed range is capped at the volume: an on-disk size never claims bytes the volume does
  not contain.
- **Tier**: an exFAT `NoFatChain` stream stores its extents exactly, so it is `FsMetadata` on the
  strength of the metadata alone. Everything else — every FAT32 entry, and exFAT streams that use
  the chain — rests on the contiguity *assumption*, so it is the reassembly tier until a stage D
  validator confirms the range end to end.

## Spec: APFS deleted-file recovery

- Parse the checkpoint descriptor area: it retains a ring of recent container superblocks
  (`NXSB`, Fletcher-64 checksummed). For each recent checkpoint, resolve its object map (omap
  B-tree) and volume superblocks, then walk filesystem-record B-trees.
- Implemented subset: the omap is resolved as a single leaf node, and filesystem-tree index nodes
  are followed as physical block numbers. Multi-level omaps and oid-indirected index nodes are not
  resolved yet; a container using them yields no records rather than wrong ones. Removing this
  limitation updates this paragraph in the same change.
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

Cheap per-block features to shrink the reassembly search space. Classification is a *hint* that
gates which blocks enter a graph; it never decides a recovery, and it is deliberately permissive,
because excluding a block wrongly loses a recovery while including one wrongly only costs search
time:

- Shannon entropy over the block; byte-histogram chi-square distance to reference profiles.
- JPEG-stream detector: frequency of `0xFF 0x00` stuffing, absence of forbidden marker bytes,
  RST cadence.
- zlib/deflate detector: header check bytes, plausible Huffman block structure at offset 0.
- Output: block class ∈ {jpeg-stream, deflate, text/sparse, other-high-entropy, low-entropy} with
  a score. Classes gate which blocks enter reassembly graphs.
- Reference: Garfinkel (DFRWS 2007); Fitzgerald et al. on fragment classification.

## Spec: object validation as the reassembly oracle

Reassembly is a search, and every hypothesis is judged by a decoder. The strength of that decoder
decides whether the results are evidence:

- **Structural validation alone is not sufficient for JPEG.** Roughly a quarter of the byte values
  following an `0xFF` make the marker parser read a segment length *out of the unknown bytes*, skip
  up to 65533 bytes and carry on. Random data therefore produces "valid structure" routinely, and
  the position the parser reaches measures where the candidate sat on the disk rather than whether
  it is a file. For contiguous carving this is harmless — one hypothesis per header — but a search
  that tests thousands is wrong on every scan.
- A reassembled JPEG is accepted only when its **entropy-coded scan decodes**: every MCU the frame
  header requires, then `EOI`. This is exact, not a threshold. Extents are trimmed to the byte the
  decoder confirms.
- **Progress is MCUs decoded**, never a stream position. Garbage fails on the first Huffman code
  outside the table, so progress cannot be inflated.
- A reassembled PNG needs no equivalent: the per-chunk CRC32 the structural validator already
  verifies makes a chance assembly impossible, and its break point is exact for the same reason.
- Among competing complete assemblies — which the exact gate makes rare — the smoothest wins,
  scored by the mean absolute luminance difference between vertically adjacent pixels.
- Scope is baseline and extended sequential Huffman (`SOF0`, `SOF1`). Progressive and
  arithmetic-coded frames are reported as unsupported and are **not reassembled**: claiming a
  recovery from a coding the oracle cannot check would be a guess.

## Spec: bifragment gap carving

For a candidate with header at `h` and fragmentation point at `h + k`:

- The first fragment ends at a block boundary **at or below** `h + k`. The break point is an upper
  bound on the splice, not the splice itself: a wrong continuation frequently parses on for a
  while past the real boundary, so the nearest few block boundaries below it are all candidates
  (`MAX_PREFIX_CANDIDATES`).
- The second fragment starts at a later block boundary; hypotheses are tried in increasing gap
  size, bounded by `MAX_GAP_BYTES`, and the total attempt count by `MAX_HYPOTHESES`.
- The second fragment is offered as running to the end of the searchable region, **not** to a
  located footer, and the accepted extents are trimmed to the decoder's reported length. Footer
  enumeration would make the outcome depend on which of a medium's many false `FF D9` hits was
  tried first.
- Accept the first hypothesis the oracle above confirms; tier `Reassembled`.
- Reference: Garfinkel, "Carving contiguous and fragmented files with fast object validation",
  DFRWS 2007.

## Spec: graph reassembly (PUP)

For candidates with > 2 fragments:

- Nodes: classified candidate blocks/extents. Edge weight between extents = decoder continuation
  success (hard gate) + pixel-row boundary smoothness (sum of absolute luminance differences
  across the stitch row; lower is better).
- Parallel Unique Path: grow the best path per header greedily, all headers in parallel, each
  extent assignable to one path (matching Pal/Sencar/Memon's SmartCarver reassembly).
- A fragment committed to a path is trimmed to the bytes the decoder actually consumed from it.
  Left at the full length it was offered, it would swallow the rest of the medium and no further
  fragment could be reached, so every file would appear to have two pieces.
- The walk does **not** stop at the first assembly that decodes: a shorter, wrong path often
  decodes, so every completion is scored and the smoothest is kept.
- Progress and offsets inside a path are measured in the **assembled** stream, which is a
  different coordinate system from the medium once a path has more than one fragment.

### The seam check

Deciding that bytes *decode* is not deciding that they *belong together*. Two photographs from
one camera share Huffman tables, so a splice from one scan into the other decodes cleanly to the
required MCU count — the entropy decoder cannot separate them, and a photo library is the modal
medium for this tool. What separates them is the picture at the splice:

- The entropy decoder reports which MCU it was on when it crossed the fragment boundary, which
  fixes the **stitch row** exactly.
- That row's mean luminance difference is compared against the frame's **median** row difference,
  so the measure is scale-free — a busy photograph and a smooth one both score near `1.0` with no
  seam.
- Above `MAX_SEAM_RATIO` the assembly is refused. So is one whose frame is too small to have a
  meaningful median: refusing beats guessing.
- Outputs tier `Reassembled`, with every extent recorded per `A-PROVENANCE`.
- References: Pal & Memon, IEEE SPM 2009; Pal, Sencar & Memon, DFRWS 2008; Uzun & Sencar 2015.

### Measured recovery, and the greedy walk's limit

Recovery rates are measured per fragmentation pattern against planted ground truth, and the suite
holds two different things:

- **Zero fabrication is absolute.** Every reported reassembly must be byte-identical to what was
  planted. A tool that recovers more by occasionally assembling bytes that were never a file is
  worse than one that recovers less, because an examiner cannot tell the two apart.
- **The rate is measured, not assumed**, and held to a recorded floor so a regression fails.

The suite includes a **competing photograph from the same encoder** between the fragments, which
is the condition that defeats an entropy-only oracle; a suite that plants one image in noise does
not test the guarantee it asserts.

Two fragments — the dominant real pattern — is recovered in most cases, forwards or backwards, and
the seam check refuses rather than guesses when it cannot judge, which costs some recall. Deeper
fragmentation is where PUP's greediness shows: it commits one fragment per step and never
reconsiders, so a step whose best candidate is not the true continuation loses the path. It gives
up rather than guess, which is why nothing is fabricated, but it gives up often. Backtracking over
committed steps is what would raise the deeper rates and is not implemented.

## Spec: confidence tiers and finding merge

- Tier ladder: `FsMetadata > JournalResidue > ContiguousCarve > Reassembled >
  PartialOrThumbnail`. Assignment rules per spec above; no post-hoc promotion
  (`A-CONFIDENCE-HONEST`).
- Merge: findings from stages C and D/E are deduplicated by (a) overlapping source extents, then
  (b) content hash. On a duplicate, keep the higher tier and attach the other's metadata (e.g. a
  carved artifact gains the filename from an `$I30` hit over the same extents).
