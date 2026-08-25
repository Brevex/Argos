# Open work

What a full run over real media established, what the current build does about it, and what is
left. Every number here is measured, not estimated; every claim about the code carries the file and
line it rests on.

This is a specification of remaining work, in the register of
[DEVELOPMENT-PLAN.md](DEVELOPMENT-PLAN.md): what the end state is and how to reach it. It is not a
record of how anything was decided (`M-NO-META-DESIGN-DOCUMENTATION`). Defects that were found and
fixed are recorded once each in [defects/](defects/); this file is about what has *not* been fixed.
Questions this file cannot settle from inside the work are put to an outside session by the prompts
in [audit/](audit/), which fix an evidence contract before they ask anything.

---

## 1. The measurement

One scan of `/dev/sdc`, 1,000,204,886,016 bytes, 12 workers, 5 h 31 m, state `finished`. A 1 TB
mechanical disk of ten years' use: NTFS under Windows, then re-partitioned and re-formatted to a
Linux install whose current layout is five partitions with an 862 GB data volume. Three of its
partitions were mounted while the scan ran.

### 1.1 What the run produced

| Measurement | Value |
| --- | --- |
| records in the manifest | 348,361 |
| written to the output directory | 47,658 |
| omitted under the 300 px floor | 300,703 |
| collapsed by SHA-256 | 1,085,566 |
| signature hits that failed validation | 388,301 |
| fragmentation points located | 50,355 |
| reassembly: attempted / recovered | 254 / 3, 2 h ceiling reached |
| partial prefixes reported | 3,961 |
| unreadable regions | 98, costing 0 findings |
| volumes located | 15,186 — 15,157 ext4, 29 NTFS, **none current** |
| orphaned `FILE` regions not attributed | 1,512 |
| recoveries from filesystem metadata | **91** (31 at `fs-metadata`, 60 demoted to partial) |

### 1.2 Stage timings

| Stage | Wall clock | Note |
| --- | --- | --- |
| sweep (`carve`) | 2 h 01 m | 138 MB/s over the surface |
| filesystem | 32 m | opening 15,186 located volumes |
| validation | 41 m | 2,026,184 candidates |
| reassembly | **2 h 00 m** | exactly the budget; a ceiling, not a completion |
| report | 12 m | |
| triage | 3 m | |

The reassembly counter's total was 95,670 = 47,106 headers × 2 + **1,458 regions**. At 254
candidates in 7,224 s the observed cost is 28.4 s per candidate, measured on the most expensive
slice: regions are ordered by decoder progress, so the highest-progress candidates — the ones with
the largest first fragments — went first.

### 1.3 What the fragmentation points are

| Set | Count |
| --- | --- |
| PNG (reports no unit count) | **42,484** |
| JPEG declaring a frame ≥ 300 px | 4,622 |
| JPEG below the floor, skipped | 3,249 |

Of the 4,622 JPEG: 1,332 at 75–99 % decoded, 899 at 25–50 %, 491 at 50–75 %, 1,241 at 5–25 %, 659
below 5 %. 1,966 declare a frame of 640 px or more. Mean first fragment: 261 KB.

### 1.4 What the photographs look like

952 artifacts carry an EXIF `DateTimeOriginal`; **859 of them were written**, 783 of those above
2048 px on the long side. Cameras named: Samsung `SM-G610M`, `SM-J100M`, `SM-G600FY`, `SM-G3502T`,
`SM-A235M`; Canon `PowerShot SX50 HS` and `EOS 5D Mark II`; Olympus `FE170`; Sony `DSC-W610`,
`DSC-W170`; Nikon `D80`, `D4S`, `E5700`; OnePlus `A5010`; Nokia `X2-01` — and HP `pstc3100`,
`pstc4400` and `Scanjet G2710`, which are **flatbed scanners**. Scanner output carries the date of
the scan, not of the photograph.

Capture years, from the 952: 2 in 2003, 3 in 2006, 7 in 2007, 5 in 2008, 18 in 2009, 2 in 2010,
5 in 2011, 3 in 2012, 113 in 2013, 8 in 2014, 21 in 2015, 276 in 2016, 164 in 2017, 224 in 2018,
88 in 2024.

### 1.5 The conclusions those numbers force

- **The pipeline recovers photographs in full resolution.** They were unfindable among 47,658
  files, not missing. This is what the ordering and the results view exist for.
- **The filesystem-metadata path is nearly dead.** 91 recoveries — the same number, exactly, as the
  run that preceded every fix in `defects/` — while 1,512 run lists were discarded for want of a
  volume to resolve them against.
- **The search was never tested; its budget was.** 254 of 50,355 points is 0.5 %.
- **90 % of the search queue was PNG**, and before §2.6 every one of them cleared every size floor.

---

## 2. What the current build already does

Stated so the backlog below is read against the right baseline. Each of these landed with tests.

| | Capability | Where |
| --- | --- | --- |
| 2.1 | `argos acquire` copies a medium to a raw image, multi-pass, with progress | [acquire.rs](../crates/argos/src/acquire.rs) |
| 2.2 | The manifest records what a run reached: `coverage`, `volumes` | [lib.rs](../crates/argos_report/src/lib.rs) |
| 2.3 | Findings lost to damage are counted, not silently dropped | [finding.rs](../crates/argos_engine/src/finding.rs) |
| 2.4 | Artifacts are ordered by named evidence (`Standing`), CLI and window | [rank.rs](../crates/argos_classify/src/rank.rs) |
| 2.5 | NTFS anchors are confirmed against their `$MFT` before anything resolves against them | [ntfs.rs `locate`](../crates/argos_fs/src/ntfs.rs) |
| 2.6 | PNG candidates declare their `IHDR` size, so a floor can act on them | [png.rs `header_dimensions`](../crates/argos_carve/src/png.rs) |
| 2.7 | `$UsnJrnl:$J` is read; artifacts carry `deleted_unix` | [ntfs.rs `change_journal`](../crates/argos_fs/src/ntfs.rs) |
| 2.8 | `--range` scans a neighbourhood instead of a disk | [main.rs](../crates/argos/src/main.rs) |
| 2.9 | A finding that runs into damage keeps the part before it | [finding.rs `head_before_damage`](../crates/argos_engine/src/finding.rs#L457) |

Measured guarantee that must survive every change below: `crates/argos_carve/tests/recovery_rate.rs`
reports 87 % on two and three fragments, 25 % on four, and **0 fabricated on all six patterns**.

---

## 3. Open work

Ordered by expected recall against effort. Each item states where it lives, what it must do, how to
tell it worked, and what it risks.

### 3.1 The PNG oracle has no resume

**Where.** [`reassemble.rs:570`](../crates/argos_carve/src/reassemble.rs#L570) — `Oracle::probe`
falls through to a full `probe(src, Format::Png, trial, scratch)` for every hypothesis, which runs
`crate::validate` over the whole assembled path from its first byte.

**What is wrong.** JPEG hypotheses cost their own bytes because `mcu::resume_at` snapshots the
decoder and `mcu::scan_resumed` carries it forward — the change recorded in
[defects/05](defects/05-reassembly-never-ran.md) that took a hypothesis from 10.6 ms to ~7 µs. PNG
got none of it. Every PNG hypothesis re-walks every chunk and re-inflates every `IDAT` from the
start, so its cost is linear in the path, which is what the resume exists to avoid.

**How to implement.** Mirror the JPEG design in `png`:

```rust
/// A PNG walk stopped at a verified chunk boundary, ready to carry on.
pub struct Resume {
    /// Assembled offset of the first byte not yet verified.
    at: u64,
    /// Inflate state after the payload verified so far.
    inflate: miniz_oxide::inflate::core::DecompressorOxide,
    /// Decompressed bytes accepted, for the cap and for progress.
    inflated: u64,
    /// `IHDR` fields, so a resumed walk keeps checking against the frame.
    header: Ihdr,
}

pub fn resume_at<R: Read + Seek>(src, start, until, scratch) -> Result<Option<Resume>, CarveError>;
pub fn scan_resumed<R: Read + Seek>(resume, src, len, splices, scratch) -> Result<Outcome, CarveError>;
```

The chunk walk is easy to snapshot — a chunk boundary is a clean state. The inflate stream is the
work: `miniz_oxide`'s `DecompressorOxide` is `Clone`, so a snapshot is a clone plus the output
window, and the existing incremental call in `png.rs` already threads state through
`inflate_out`. Keep the decompressed cap where it is.

**Acceptance.** A benchmark over a fragmented PNG showing per-hypothesis cost flat in the first
fragment's size, as `defects/05` records for JPEG. `recovery_rate.rs` unchanged.

**Risk.** Contained: PNG's per-chunk CRC32 remains the oracle, so a faster search cannot become a
looser one. This changes cost, never acceptance.

### 3.2 A PNG has no reportable partial prefix

**Where.** [`reassembly.rs:491`](../crates/argos_engine/src/pipeline/reassembly.rs#L491) — `worth_reporting`
returns `false` for `Format::Png`, deliberately and with the reason in its doc.

**What is wrong.** Nothing yet — it is honest as it stands. PNG verifies per chunk: a CRC32
confirms a whole chunk or says nothing about any of it, so a file whose tail is gone has a
truncated `IDAT` that cannot verify and a confirmed prefix that stops 33 bytes in, at the `IHDR`.
Writing that would be writing a description of a picture and calling it the picture.

**What would make it reportable.** The `IDAT` payload is a zlib stream, and zlib is
self-describing: it can be inflated incrementally and the decompressed bytes that came out are
*proven* to be this image's data, whatever the chunk CRC could not confirm. That is the exact
counterpart of the JPEG entropy decoder confirming MCU by MCU.

**How to implement.** This subsumes §3.1 — both need the same incremental inflate with position
reporting. Once it exists:

1. `png` reports `inflated` bytes and the raw size the `IHDR` implies (`Ihdr::raw_bytes`, already
   there), which gives a genuine fraction of the frame.
2. `locate_break`'s PNG arm fills `Broken::decoded` and `Broken::required` from those instead of
   the zeroes at [`reassemble.rs:324`](../crates/argos_carve/src/reassemble.rs#L324).
3. `worth_reporting` collapses back to one rule for both formats: a share of the frame.
4. The extent runs to the last byte that inflated, not to the chunk boundary.

**Acceptance.** A truncated PNG fixture yields a partial whose extent holds the medium's own bytes
and whose recorded fraction matches the rows a tolerant decoder renders.

**Why it matters here.** 42,484 of the disk's 50,355 fragmentation points are PNG, and today every
one of them can only ever be a recorded break.

### 3.3 Restart markers are computed and unused

**Where.** [`reassemble.rs:1448`](../crates/argos_carve/src/reassemble.rs#L1448) — `restart_points`
finds every `RSTn` re-entry point in a block. Its own doc, at line 1367, says the walk does not use
them: candidates are offered block-aligned only, so an orphaned fragment is reachable only through
its predecessor (Uzun & Sencar, 2015).

**The constraint that bounds this.** Decoding entropy data needs the Huffman tables, and those live
in the file's header. So the reachable case is a **hole in the middle** — header present, first
fragment decoded, an intermediate fragment gone, a later fragment starting at a restart marker. The
case where the header itself is gone is *not* reachable without borrowing tables from another file,
which is a fabrication risk this project should not take.

**How to implement.**

1. `reassemble::Candidate` gains an optional entry offset: the byte after an `RSTn`, rather than the
   block's own start.
2. `Region::load` ([`region.rs`](../crates/argos_engine/src/pipeline/region.rs#L126)) offers restart-aligned
   candidates alongside block-aligned ones, for blocks whose profile already counts restarts
   (`BlockProfile::restarts`, measured and discarded today).
3. `mcu::scan_resumed` accepts a splice that resets the DC predictor and realigns the restart
   phase, which is what an `RSTn` means. The restart interval comes from the frame's `DRI`.
4. Only for candidates whose frame declares `DRI`; without it there are no re-entry points and the
   extra candidates are pure cost.

**Acceptance.** A new pattern in `recovery_rate.rs`: three fragments with the middle one absent,
the survivor starting at a restart marker. It must recover, and **`fabricated` must stay 0 on every
pattern** — that is the bar, not the new rate.

**Risk. The highest in this document.** It widens what the oracle will accept, in the one place the
project cannot afford a wrong answer. Land it behind the rate suite and read the fabrication column
first.

### 3.4 The join cost stops discriminating at four fragments

**Where.** [`reassemble.rs:86`](../crates/argos_carve/src/reassemble.rs#L86) `MAX_SEAM_RATIO`, and
[`reassemble.rs:1000`](../crates/argos_carve/src/reassemble.rs#L1000) `MAX_BRANCHING_FRAGMENTS = 3`,
which exists because of it.

**What is known.** Branching past three fragments produced an assembly of the right length,
decoding end to end, whose three seams all passed the ratio test, and which was not the planted
bytes. Tightening the ratio does not separate that case: 2.5 and 2.0 leave it, and 1.6 removes it
only by refusing about a third of the true recoveries with it. So the bound is depth, and it is a
statement about the oracle rather than about the budget.

**How to implement.** More terms, each measured and thresholded on its own, all of which must pass:

- **Gradient continuity.** The seam ratio compares row means. A splice between two photographs of
  similar brightness passes it; the *derivative* across the stitch does not behave, because a real
  image's vertical gradient is continuous and a splice's is not.
- **Chroma coherence.** Luma is what `decode_jpeg_luma` gives today
  ([`decode.rs`](../crates/argos_carve/src/decode.rs)). Chroma planes are subsampled and change more
  slowly, so a discontinuity in Cb/Cr across a row is harder for a wrong splice to satisfy.
- **DC drift.** JPEG's DC coefficient is differential per MCU row. A wrong continuation makes the
  predictor drift systematically, not randomly — the drift is a signature, and it is available from
  `mcu` without a full pixel decode.

**Acceptance.** `recovery_rate.rs` four-fragment rate above the recorded 25 % **with `fabricated`
still 0 on all six patterns**. Only then raise `MAX_BRANCHING_FRAGMENTS`. Each threshold gets its
derivation recorded beside it (`M-DOCUMENTED-MAGIC`).

### 3.5 Progressive and arithmetic JPEG recover nothing when fragmented

**Where.** [`mcu.rs:421`](../crates/argos_carve/src/mcu.rs#L421) — `SOF2`, `SOF3`, `SOF5..=SOF7`,
`SOF9..=SOF11`, `SOF13..=SOF15` return `ScanStop::Unsupported`. Then
[`reassemble.rs:297-301`](../crates/argos_carve/src/reassemble.rs#L297-L301) returns `None` for an
unsupported outcome, so such a candidate produces **no `Broken` at all** — no reassembly and no
partial prefix. A contiguous progressive JPEG still validates and is recovered; only the fragmented
ones are lost.

**The open question.** How much of this disk is progressive is unmeasured. It cannot be counted
from the manifest, because these candidates leave no fragmentation record — they are inside the
388,301 rejected.

**Cheap first step, no disk access.** Count `SOF` markers among the artifacts already written in the
session directory: a scan of `~/Imagens/Camera` for `FF C2` in the marker segment of each recovered
JPEG gives the progressive share of what *did* come back, which is a usable proxy. Do this before
committing to the rest.

**How to implement.** A progressive coefficient decoder: spectral selection and successive
approximation across multiple scans, accumulating coefficients per block until `EOI`. It is the
largest item here — weeks, not days — and it duplicates a good deal of `mcu`'s structure.

**Recommendation.** Do not start it until the count above says it is worth it.

### 3.6 The ext4 residue sweep has a catastrophic false-positive rate

**Where.** [`residue.rs:277`](../crates/argos_fs/src/residue.rs#L277) — an ext superblock anchor is
accepted on `Ext4::from_superblock` alone, with no confirmation that a volume is behind it.

**What was measured.** 15,157 ext4 "volumes" against 29 NTFS, on a disk with a handful of
historical filesystems. Every one of them costs the filesystem stage an open and a journal walk;
that stage took 32 minutes.

**How to implement.** The same medicine as §2.5. An ext4 superblock states the blocks per group and
the inode table location of each group; confirm the anchor by reading the first group's inode table
and checking that it holds plausible inodes, exactly as `ntfs::locate` reads the `$MFT`. An anchor
that cannot be confirmed is not reported.

**Acceptance.** A fixture with a stray superblock and no filesystem behind it yields no volume, and
the existing ext4 recovery tests are unchanged. Against the disk: `volumes` falls from 15,186 to a
number a person can read.

### 3.7 Reassembly runs on one core — **done**

The parallel unit turned out to be the header, not the region: a region is memory once loaded, so
every worker takes its own view of the held bytes and its own buffers, and `in_parallel` in
[`pipeline.rs`](../crates/argos_engine/src/pipeline.rs) hands them the region's headers.

The gap search consults nothing the headers share, so it splits exactly. The walk does consult the
claimed set, so every worker is given the set as it stood when the phase began and the region's
outcome is settled afterwards in header order; a walk that a neighbour's recovery invalidated is run
again there against everything claimed by then. That is what makes the region's result the
sequential one for any number of threads, asserted by
`the_search_finds_the_same_images_however_many_threads_ran_it`.

**Measured**: 28.78 s → 5.02 s, **5.74×** on six physical cores, with identical artifacts and
identical recovery counts on 1 and 12 workers. The remaining serial work is region loading, which is
the medium's read speed and not a core count.

### 3.8 A volume whose anchors are all gone is still unreachable

**Where.** [`filesystem.rs`](../crates/argos_engine/src/pipeline/filesystem.rs#L201) `confirm_ntfs` — it corrects a
backup boot sector to the volume it belongs to and drops coincidences, which covers the case where
*an* anchor survived. It does not cover a volume whose primary and copy are both overwritten while
its `$MFT` records survive.

**How to implement.** Infer the geometry from the orphaned records themselves. Each `FILE` record
carries its own record number at offset 44, so a run of consecutive records gives the stride and
lets record 0's position be computed. Cluster size comes by voting: for each candidate size
(4/8/16/32/64 KiB), resolve the run lists and count how many land on offsets holding an image
signature. The size that maximises hits is the volume's.

**The gate that makes it safe.** Never act on an inference the double confirmation in
`finding_from` would not accept — signature at the first extent *and* the assembled extents passing
the format state machine. An inference that produces no confirmed recovery produces nothing.

**Whether it is needed** depends on the next run: if `unattributed_residue` falls from 1,512 to
near zero after §2.5, the anchors were there and this is unnecessary.

### 3.8b Cancelling discards what the run already established

**Where.** `emit` in [`output.rs`](../crates/argos_engine/src/pipeline/output.rs#L89) breaks on
`control.is_cancelled()` at the top of its loop, and `report_findings` runs *after* `carve` — which
contains the reassembly stage.

**What is wrong.** Cancel during a search means the flag is already raised when the report stage
starts, so the loop breaks on its first iteration and **no artifact is written**. The manifest is
written with `artifacts: []`. The comment above that line says "cancelling still writes the
manifest", and it does — empty. On the 12-hour run of `defects/06`, pressing Cancel would have
discarded every one of the ~47,000 artifacts the reading stages had already established.

**What survives, and why that made the fix optional.** `report.fragmentation` is filled before
reassembly starts and reaches the manifest through a `Summary` field that does not pass through
`emit`, so a cancelled run still records where every fragmentation point is. That is what makes a
later `argos reassemble --from` cheap, and it is why cancelling that run was the right call rather
than an expensive one.

**How to implement.** The honest meaning of Cancel is "stop searching and write what you have", so
the report stage must distinguish a stop asked for during the search from one asked for during the
writing. Give `Control` a cancellation generation that `report_findings` reads on entry and compares
against, stopping only if it increases — a second press. Keep the existing behaviour for a Cancel
pressed *while* artifacts are being written, which is the case the current check was written for.

**Acceptance.** A scan cancelled during reassembly writes every artifact the earlier stages
established; one cancelled during the report stage stops between two artifacts, as it does today.

### 3.8c Pause does nothing outside the sweep

**Where.** `wait_while_paused` is consulted in exactly one place,
[`pipeline.rs:674`](../crates/argos_engine/src/pipeline.rs#L674), inside the reader loop.

**What is wrong.** The reassembly stage is where a scan of a large medium spends most of its wall
clock, and the Pause button is inert for all of it — it reports paused and the stage carries on.

**How to implement.** Consult `Control::wait_while_paused` in the region loop of `search_region`,
beside the cancellation and budget checks, so a pause takes effect within one candidate. Failing
that, the window must disable the button for stages that cannot honour it, because a control that
reports a state the engine is not in is worse than no control.

### 3.9 Smaller open items

| | Item | Where |
| --- | --- | --- |
| a | The `$UsnJrnl` stream name and `USN_RECORD_V2` layout are **unverified against real NTFS media** — every fixture writes what the constants say, so the tests prove reader and fixture agree and nothing more. A scan of a 630 GiB residual NTFS volume returned `journal_deletions: 0`, which the two explanations — no journal survived, or the reader does not read a real one — both predict; a targeted read of one live volume separates them | [ntfs.rs](../crates/argos_fs/src/ntfs.rs), same standing as the ioctl codes in [DEVICE-SMOKE-CHECKLIST.md](DEVICE-SMOKE-CHECKLIST.md) |
| b | The results gallery has not been seen running; the Rust builds and the frontend type-checks, but no window has drawn it. Verifiable against a fixture image without touching a disk | [Gallery.svelte](../crates/argos_ui/ui/src/layout/parts/Gallery.svelte) |
| c | The window exposes no size floor and no reassembly budget. The gallery reduced the need; whether to add them is a deliberate choice against the one-screen layout of §6.1-6 | [Shell.svelte](../crates/argos_ui/ui/src/layout/Shell.svelte) |
| d | HPA/DCO are not addressed: capacity comes from `BLKGETSIZE64`, so sectors hidden behind a host-protected area are outside every scan and are not declared as such | [linux.rs](../crates/argos_device/src/device/linux.rs) |
| e | Only JPEG and PNG are carved. TIFF matters here specifically: three of the cameras in §1.4 are flatbed scanners, and scanner output is often TIFF | [lib.rs](../crates/argos_carve/src/lib.rs) |
| i | The entropy decoder's remaining cost is 5.2 cycles per bit, spread thin: no single item above 3% survives in the hypothesis path. Further gain there is micro-optimisation, and the stage's shape has changed enough that where it would come from needs measuring again before anything is written | [defects/08](defects/08-the-entropy-decoder-read-one-bit-at-a-time.md) |
| j | Reassembly still wants about 25 h over the 46,345-step queue, in an interval of 9.6 h to 49 h. The width is 18 regions nobody has timed, and no quantity available before the search predicts what one will cost — four proxies were tested and refuted, so a run is bounded by its budget rather than planned around | [defects/09](defects/09-the-counter-counted-steps-of-unequal-cost.md) |
| k | The reassembly budget cannot interrupt an item, only stop the next one being taken: workers check `spent` before they take, and a candidate at the 262,144 ceiling over a dense region runs for as long as it runs. Measured on a 192 MiB fixture of whole photographs, a 120 s budget was still going at 1,125 s — **9x over, and not finished when the run was stopped**; the field's longest single item was 4,112 s, so a 2 h budget can become 3 h. The progress bar now correctly reads `120/120 seconds` throughout that, which is true and still not what a reader wants. Bounding it means checking the clock inside the hypothesis loop, which is the oracle's hot path and needs its own measurement and differential test | [reassembly.rs `in_parallel`](../crates/argos_engine/src/pipeline/reassembly.rs#L187) |
| h | Orphaned-fragment carving reaches the CLI as `graft` (C51) and **not the scan pipeline**, which is deliberate — a graft is pixels in a container this tool built, and a scan's artifacts are files the medium held. What is open is whether the sweep should also run inside a scan, which needs the writing stage to carry a synthetic header without weakening the digest it computes from the stream it hands the sink. Was: [reference.rs](../crates/argos_carve/src/reference.rs) lends a surviving sibling's header to a headerless fragment and [reassemble.rs `restart_points`](../crates/argos_carve/src/reassemble.rs) says where such a fragment may be entered, both tested end to end — but no pipeline stage offers an orphan to them and no CLI flag names a reference. What is missing is the wiring and the reporting: a graft is pixels in a container this tool built, never a recovered file, so it needs the weakest tier and a provenance field naming the reference (`A-CONFIDENCE-HONEST`) | [reference.rs](../crates/argos_carve/src/reference.rs) |
| g | Recall against a published corpus is **measured by nothing yet**: [corpus_recall.rs](../crates/argos_engine/tests/corpus_recall.rs) reads image/answer pairs from `ARGOS_CORPUS_DIR` and reports per-case recall, and its own instrument is self-tested, but no corpus has been supplied. The DFRWS 2006/2007 carving challenges and NIST `CFReDS` FC-01..FC-05 are what it is for; until one is measured, no recall figure in this file has an external reference | [corpus_recall.rs](../crates/argos_engine/tests/corpus_recall.rs) |
| l | The btrfs deleted-file diff is proven on fixtures and against real btrfs structures, but not against real deletion residue: producing it needs a mount, and `mkfs.btrfs --rootdir` writes the filesystem tree once, so an unmounted image carries no copy-on-write history of a removal. Same standing as (a) above; [BTRFS.md §4](BTRFS.md) states what closes it | [btrfs.rs](../crates/argos_fs/src/btrfs.rs) |
| f | Thumbnail matching as a second acceptance path for a reassembly — a candidate whose decoded image matches a known EXIF thumbnail is confirmed regardless of its seams. Sequenced behind a completed search, which tells whether it is needed | [reassemble.rs `score`](../crates/argos_carve/src/reassemble.rs) |

---

## 4. What blocks what

Three distinct blockers. Only the first stops code from being written.

**A — There is no forensic image.** 193 GB free against 1 TB needed. Every experiment that reads
the medium is therefore serialised, expensive, and wears a ten-year-old disk that already reports
98 unreadable regions. `argos acquire` exists and cannot be used. This is the constraint under
everything else.

**B — A search currently holds the device.** Nothing may read `/dev/sdc` while it runs. This blocks
*validating* §3.6 and §3.8 against reality, and blocks measuring the overwrite question at all. It
does not block writing any of the code: every item above is driven by fixtures.

**C — The gallery is unverified.** Self-inflicted and self-removable: a scan of a small fixture
image, then open the window.

---

## 5. What cannot be fixed

Unchanged from the analysis that produced this file, and worth restating so no item above is read
as a route around it.

- **Overwritten bytes do not come back.** If the region that held the batch was reallocated by a
  later format, nothing on the host-visible surface recovers it.
- **A thumbnail does not reconstruct its original.** It proves a photograph existed and gives its
  name, date and appearance. It is an input to search and to validation, never to reconstruction.
- **A hole in a baseline JPEG without `DRI` is permanent from that point on.** DC prediction is
  differential and Huffman coding has no resynchronisation; only `RSTn` creates re-entry points, and
  only if the camera wrote them. §3.3 exploits them where they exist and cannot invent them where
  they do not.
- **Exact n-fragment reassembly is intractable in general**, and the greedy walk with pruning is the
  right answer. But the binding limit here is nearer than the computational one: at four fragments
  the join cost stops telling a real assembly from a plausible one. §3.4 is an attempt at that
  limit, not a way around it.
