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
- A file whose run list outgrew its record keeps the rest in **extension records**, named by an
  `$ATTRIBUTE_LIST` (0x20) in the base one. Follow every entry of type `$DATA` with no attribute
  name, in the order the list gives them — that is starting-VCN order, so the runs append in file
  order — resolving each record number through the `$MFT`'s own extents rather than at a fixed
  stride, since a fragmented `$MFT` puts record *n* wherever its runs place it. Bounded by
  `MAX_EXTENSION_RECORDS`. This is the shape NTFS gives its most fragmented files, and reading only
  the base record reports them truncated at whatever fitted — which looks like a success.
- Independently, signature-scan the surface for **orphaned `FILE` records** (records whose LBA is
  outside any known `$MFT`): after a re-format the new `$MFT` lands elsewhere, so old records
  survive in place. Validate by fixup array before trusting any field.
- Mine `$I30` index slack for directory entries of deleted files. A directory that removes an entry
  leaves it in the index buffer's slack, so a name can outlive the `$FILE_NAME` in the file's own
  record. The residue sweep locates `INDX` buffers the same way it locates `FILE` records.
  An entry numbers an MFT record while a finding is identified by where its record sat, and the two
  meet only through the geometry of the volume the index belongs to — so an index no located volume
  covers names nothing, and a fragmented `$MFT` means a miss rather than a wrong name. A finding
  that still has its own name keeps it: its record is the better evidence.
- Parse `$UsnJrnl:$J` for delete records (names, timestamps, file reference numbers). The parser
  exists and is tested; it is **not yet wired**, because reaching `$J` means reading a *named*
  `$DATA` stream and the record walker deliberately reads only the unnamed one. Removing this
  limitation updates this paragraph in the same change.
- On a live Windows source, enumerate Volume Shadow Copies as additional read-only block sources.
- **Tier**: `FsMetadata` when extents come from a validated record; names from `$I30`/`$UsnJrnl`
  alone attach as metadata to carved artifacts, they do not create extents.

## Spec: ext4 deleted-file recovery

- Read the primary superblock; on failure, backup superblocks (block group boundaries per the
  sparse_super layout).
- Deleted inodes in place have zeroed extent trees — do not bother. Instead scan the **jbd2
  journal**: descriptor blocks map journaled buffers to filesystem blocks; stale copies of
  inode-table blocks contain pre-deletion inodes whose extent trees are intact. Parse extent
  trees to extents at **any depth**: ext4 keeps them inline until a file has more than the inode
  holds and then pushes them into index blocks, so a deep tree marks a heavily fragmented file —
  the very file this stage exists for. Each node is accepted only with the extent magic and a
  depth exactly one below its parent's, and the walk is bounded by `MAX_EXTENT_DEPTH` and
  `MAX_TREE_BLOCKS`, so a crafted tree cannot descend for ever.
- Carve directory-entry blocks (recognizable by `rec_len`/`name_len` self-consistency chains) for
  orphaned names. The parser exists and is tested; it is **not yet wired**, because nothing yet
  offers it directory blocks — the journal walker reads inode tables, not directories. Removing
  this limitation updates this paragraph in the same change.
- **Tier**: `JournalResidue` for journal-mined extents; `FsMetadata` only for extents from
  currently valid metadata (e.g. snapshot-like cases).

## Spec: FAT32/exFAT deleted-file recovery

- **The walk leaves the root.** A person's files sit in folders, and folders nest, so recovery
  follows the root's cluster chain and descends into every live subdirectory it names, breadth
  first — bounded by `MAX_DIRECTORIES`, `MAX_DIRECTORY_DEPTH` and `MAX_DIRECTORY_BYTES`, and by a
  visited set so a chain that loops terminates. Reading only the root's first cluster, which is
  what this did, makes a whole photo library invisible on a FAT volume.
  Only *live* subdirectories are descended into: a deleted directory has lost its chain, so
  following it would read whatever now occupies those clusters and report the names found there as
  if they were its.
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
- **A candidate that cannot be completed is reported as the part of itself that decodes**, when its
  frame clears the size floor and at least `MIN_PARTIAL_PROGRESS` of it decoded. A photograph whose
  remainder was overwritten is not recoverable, but its beginning is on the medium and decodes: a
  3072x2304 frame the decoder walked 58% of is the top thirteen hundred rows of the picture, which
  is the difference between recognising a photograph and having nothing.
  - The extent runs from the header to the last byte of the last **whole** MCU, not to where the
    stream stopped being this file: between those two are the bytes the decoder read on its way to
    finding out, and they belong to whatever followed on the medium.
  - The bytes reported are the medium's own. No `EOI` is appended and nothing is padded, so the
    digest stays the digest of what was there (`A-PROVENANCE`).
  - Tier `PartialOrThumbnail`, and the record carries how much of the frame decoded. A candidate
    reassembly completed leaves no partial behind.

## Spec: PNG validation state machine

- 8-byte signature → chunk walk: 4-byte length (bounds-checked, ≤ remaining range), type, data,
  CRC32 verified per chunk. `IHDR` first (dimensions sanity-capped), `IDAT` runs inflated
  incrementally (bounded output per step), `IEND` terminates.
- First CRC or inflate failure marks the fragmentation point; queue for stage E with the offset
  (an inflate failure localizes it exactly; a CRC mismatch cannot localize damage within its
  chunk, so the chunk data start is reported as the earliest offset corruption may begin) and
  the inflate state summary (carried once stage E lands).

## Spec: the search grid

A fragment begins where an allocator put it, which is a multiple of the filesystem's allocation
unit **counted from the volume's start**:

- The residue sweep already reports each located volume's range; it also reports its allocation
  unit, which the same boot sector or superblock states. The search steps on that unit, from that
  origin. A volume that does not begin on a multiple of its own cluster size puts every real
  boundary off the medium's absolute grid, so an absolute grid coarser than 4 KiB would step over
  all of them.
- Stepping on the real unit loses nothing — every fragment boundary is one of its multiples — and
  costs proportionally fewer hypotheses: eight times fewer on a volume of 32 KiB clusters, for the
  same reach.
- When no located volume contains the region, or when two do and disagree on the grid, the search
  falls back to `BLOCK_BYTES` from zero: a finer grid tries more than it needs to, while a wrong
  coarser one would step over the boundary it was looking for. A resumed run has located no
  volumes and therefore always uses the fallback.

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
- The decode is **resumed, not repeated**. Both searches ask one question thousands of times over:
  given the path so far, how far does appending these bytes carry the decoder? Decoding the path
  from `SOI` each time makes the answer cost the path rather than the candidate — linear in a
  fragment that can be megabytes, so a full sweep costs hours. The path is decoded once to the last
  MCU boundary inside it; each hypothesis restores that position (predictors, bit accumulator,
  restart phase, MCU index) and decodes only the bytes between it and the path's end — at most one
  MCU — plus the continuation. The resume is rebuilt whenever the path grows, so a walk's later
  steps cost no more than its first. Measured against planted photographs, this holds a rejected
  hypothesis at roughly 7 us whatever the path's size, against 166 us for a 6 KB fragment and
  10.6 ms for a 315 KB one before it.
- A resumed decode cannot see the splices before its resume point, and an assembly is a file only
  when *every* join holds. So the cheap answer settles **how far**, and a candidate it reports as
  complete is then re-decoded from the start over the whole assembly to settle **whether it is** —
  which is where the seam check below runs. Completions are rare, so the exact pass is not a cost.
- A reassembled PNG needs no equivalent: the per-chunk CRC32 the structural validator already
  verifies makes a chance assembly impossible, and its break point is exact for the same reason.
- Among competing complete assemblies — which the exact gate makes rare — the smoothest wins,
  scored by the mean absolute luminance difference between vertically adjacent pixels.
- Scope is baseline and extended sequential Huffman (`SOF0`, `SOF1`). Progressive and
  arithmetic-coded frames are reported as unsupported and are **not reassembled**: claiming a
  recovery from a coding the oracle cannot check would be a guess.

## Spec: which candidates are searched, and in what order

A search bounded by a clock spends itself on whatever it reaches first, so what it reaches first is
part of the algorithm rather than an implementation detail.

- **A candidate is searched only if its frame declares a picture at least `min_long_side` pixels on
  its long side.** A frame states its size before its data, so this costs a header read and no
  decode. It is what stops a used disk's thumbnail cache — which outnumbers its photographs by two
  orders of magnitude — from spending the budget: a cache entry is a whole small file, and no
  reassembly of one could produce anything but the small file it already is. Candidates below the
  floor are counted and reported, never silently dropped; a format whose parser does not report
  dimensions is never read as "too small".
- **Regions are searched in order of the most units any of their candidates decoded** — MCUs for
  JPEG, counted absolutely and not as a share of the frame. A frame the decoder walked thousands of
  MCUs into is a photograph whose first fragment survived; one it walked three into is a signature
  that landed on plausible bytes. A share does not separate those, because it measures the frame and
  not the evidence: a cache entry three quarters decoded has walked fewer units than a photograph a
  tenth decoded, and on a medium whose photographs are the large frames a share orders the budget
  against them. A format that counts no units contributes zero, so its regions sort last. Ties keep
  medium order, so two runs over one medium agree.
- When the budget runs out, what was not tried is reported as not tried (`A-CONFIDENCE-HONEST`).

## Spec: a known reference, and carving orphaned fragments

An entropy-coded fragment with no header is undecodable in principle: the Huffman tables, the
sampling factors and the frame geometry all live in the header, and nothing in the fragment states
them. The published attack estimates them, which needs a corpus of encoder settings wide enough to
cover the camera that wrote the file (Uzun & Sencar, *Carving Orphaned JPEG File Fragments*, IEEE
TIFS 10(8), 2015; *JpgScraper*, IEEE TIFS 15, 2020, which recovers 24% more image data than prior
carvers on used media).

**When a file from the same batch survives, the parameters are not estimated but known.** A
reference is any recovered image the examiner can point at; its header is the header its siblings
were written with, because one camera at one setting writes one header.

- **A reference is a byte range, not a parse.** Everything from `SOI` to the end of the `SOS`
  segment is copied verbatim and never re-encoded: re-encoding is a chance to write a table that
  differs from the one the camera wrote, and the whole value of a reference is that it does not.
- **A reference must be baseline or extended sequential** (`SOF0`/`SOF1`), single scan. A
  progressive frame's scans each carry their own parameters, so a prefix of one does not decode the
  data of another, and offering one would produce confident nonsense.
- **Grafting** is reference prefix, then the orphan's entropy bytes, then `EOI`. The result is a
  syntactically whole JPEG that a decoder renders.
- **An orphan is entered immediately after `RST7`, and after no other marker.** `RSTn` resets the DC
  predictors and byte-aligns the stream, which is the state a decoder is in at the start of a scan —
  but the eight markers are *cyclic*, and a decoder leaving `SOS` counts one restart interval and
  then expects `RST0`. Entering after `RST5` presents it `RST6` at the first marker it meets, and the
  stream breaks there. Only `RST7` is followed by the marker the decoder is waiting for. A fragment
  with no `RST7` in it is not a candidate, and none can be invented for it.
- **What a graft recovers is pixels, not a file.** The frame's dimensions are the reference's, the
  strip's position within the frame is unknown, and the bytes as a whole never existed on the medium
  in that order. It is therefore reported at the weakest tier, never as a recovered file, and its
  provenance names the reference it was grafted onto (`A-PROVENANCE`, `A-CONFIDENCE-HONEST`). An
  examiner looking at one is looking at real pixels from the medium in a container this tool built.
- **Acceptance is the entropy decoder, not the pixel decoder**, and the measure is MCUs decoded. A
  graft is a fragment from the middle of a scan: it can never fill the reference's frame and its tail
  is always cut mid-MCU, so the strict pixel decode that turns *decoded* into evidence for a whole
  file rejects every graft by design. What the entropy decoder reports instead is how much of the
  picture genuinely decoded before the stream stopped being it, which is the number that describes
  the strip — the frame's dimensions being the reference's and not its own.

## Spec: resuming a search

Locating a fragmentation point costs the sweep and the validation pass — hours on a terabyte —
and establishes the same point every time. Searching from one costs minutes. So the points are
**recorded**, and the search can be run again from them alone:

- Every fragmentation point carving localizes is written to the manifest with what the decoder
  established about it: header, break point, last whole unit decoded, declared dimensions, and the
  decoded/required counts. They are recorded whether or not the search that followed found
  anything — a point the budget never reached is exactly the one worth trying again.
- A resumed run reads them back and starts at stage E. The sweep, the filesystem pass and the
  validation pass do not run.
- The medium is still read. Every extent a resumed run reports is fetched back and hashed exactly
  as a scan's is, so a session pointed at the wrong medium recovers nothing rather than something
  wrong.
- A record naming a format this tool does not recover, or whose break point does not lie past its
  header, is skipped rather than guessed at.

## Spec: region-resident search

Every hypothesis reads a few dozen bytes from a different offset. Served from the medium that is a
seek apiece, and on rotational media a seek outweighs the decode it feeds by three orders of
magnitude — the difference between a hypothesis costing microseconds and costing milliseconds.

- The stretch a candidate can reach is read **once, sequentially, and held**; every hypothesis then
  reads memory. Block classification runs over the held bytes, so the surface is not read twice.
- A region is built to hold each of its headers' whole reach either side, *and* that header's first
  fragment — the splices a gap search tries run from the header to the break point, so a region
  ending between the two would offer the decoder a prefix it cannot read.
- Consecutive regions therefore overlap by at most that reach, which caps the medium at being read
  twice over — against once per header before (`docs/defects/01`).
- Hypotheses are bounded to the held region at both ends. An offset outside it reads as
  end-of-input, so a hypothesis that reaches past what was held fails rather than being answered
  with anything else.

## Spec: bifragment gap carving

For a candidate with header at `h` and fragmentation point at `h + k`:

- The first fragment ends at a block boundary **at or below** `h + k`. The break point is an upper
  bound on the splice, not the splice itself: a wrong continuation frequently parses on for a
  while past the real boundary, so the nearest few block boundaries below it are all candidates
  (`MAX_PREFIX_CANDIDATES`). Each gets its own share of `MAX_HYPOTHESES`, and whatever a share
  leaves unspent rolls forward; a ceiling consumed entirely by the first splice would leave the
  rest untried, which is the same as not having them.
- The second fragment starts at another block boundary, tried in increasing distance from the
  first: **ahead** of it to `MAX_GAP_BYTES` (an allocator that split a file usually put the
  remainder just past it), then **behind the header** to the same bound, which is where one lands
  when the allocator fills a hole it had passed over. A trial fragment always runs forward, so one
  proposed behind the header can reach back across it; an assembly whose trimmed extents overlap
  is refused rather than reported, because it describes a layout no allocator produced.
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
- **Candidates are offered nearest-first, from the end of the path so far, within
  `SEARCH_RADIUS_BYTES`.** A file's next fragment lies near its last one — the allocator split it
  because no single run was long enough and took the nearest runs it had — so distance from the
  path end is the prior. Sweeping the candidate list in medium order instead spends the whole
  ceiling on whatever sits lowest on the disk, which for a header late on a terabyte is never the
  continuation; the list is kept in medium order so the nearest is a binary search and the two
  sides are walked outwards together.
- The walk starts from **every** splice `MAX_PREFIX_CANDIDATES` allows, as the gap search does,
  and keeps the smoothest assembly across all of them. Starting only from the block boundary
  nearest the break cannot reach a file whose real splice is further below it.
- A fragment committed to a path is trimmed to the bytes the decoder actually consumed from it.
  Left at the full length it was offered, it would swallow the rest of the medium and no further
  fragment could be reached, so every file would appear to have two pieces.
- The walk does **not** stop at the first assembly that decodes: a shorter, wrong path often
  decodes, so every completion is scored and the smoothest is kept.
- **The walk reconsiders its steps, up to `MAX_BRANCHING_FRAGMENTS`.** Committing only the
  furthest-reaching candidate and never looking back loses a path whenever that candidate is not
  the true continuation, which at three fragments happens often enough to cost most of them. Each
  step instead keeps the `MAX_BRANCH` furthest-reaching and tries each in turn, depth first, under
  the one shared hypothesis ceiling.
- **How deep it may reconsider is a question about the oracle, not the budget.** Every branch is
  another chance for an assembly that decodes and whose seams look like a photograph's to be the
  wrong one. Measured against planted ground truth, three fragments is where the seam check holds —
  87% recovered, nothing fabricated, against 25% before. At four it does not: the suite produced an
  assembly of the right length, decoding end to end, whose three seams all passed, and which was
  not the planted bytes. No seam threshold separated that case without refusing a third of the true
  recoveries with it. So the search stops reconsidering where it can still tell the difference, and
  past that depth commits greedily as before.
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

### Measured recovery, and where the search stops

Recovery rates are measured per fragmentation pattern against planted ground truth, and the suite
holds two different things:

- **Zero fabrication is absolute.** Every reported reassembly must be byte-identical to what was
  planted. A tool that recovers more by occasionally assembling bytes that were never a file is
  worse than one that recovers less, because an examiner cannot tell the two apart.
- **The rate is measured, not assumed**, and held to a recorded floor so a regression fails.

The suite includes a **competing photograph from the same encoder** between the fragments, which
is the condition that defeats an entropy-only oracle; a suite that plants one image in noise does
not test the guarantee it asserts.

Two and three fragments are recovered in most cases, forwards or backwards, and the seam check
refuses rather than guesses when it cannot judge, which costs some recall. Past three the walk
commits greedily and gives up often — deliberately, because the measurement showed that
reconsidering deeper is where it starts fabricating rather than where it starts finding.

The rates are also what fixes the search's width: it was widened until the suite reported an answer
that was not the planted bytes, and then bounded to the depth before that. A rate is therefore not
just a record of how well the search does — it is the evidence for how far it is allowed to go.

## Spec: confidence tiers and finding merge

- Tier ladder: `FsMetadata > JournalResidue > ContiguousCarve > Reassembled >
  PartialOrThumbnail > Grafted`. Assignment rules per spec above; no post-hoc promotion
  (`A-CONFIDENCE-HONEST`).
- `Grafted` is the floor because it is the one tier whose artifact **is not a file the medium ever
  held**. Every other tier reports bytes that lay on the medium in the order reported; a graft
  reports medium bytes inside a header this tool supplied, under a frame size taken from a different
  file. The pixels are evidence and the container is not, and no tier above the floor can say that.
- Merge: findings from stages C and D/E are deduplicated by (a) overlapping source extents, then
  (b) content hash. On a duplicate, keep the higher tier and attach the other's metadata (e.g. a
  carved artifact gains the filename from an `$I30` hit over the same extents).
