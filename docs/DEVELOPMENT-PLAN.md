# Argos — Development Plan

Target state and phased build order for Argos: forensic recovery of deleted PNG/JPEG images from
block devices (HDD/SSD/NVMe) on Windows, Linux and macOS. Rust engine, hexagonal architecture,
Tauri as a presentation shell only.

This document is written to be executed by AI agents working inside this repository. It never
overrides [CLAUDE.md](../CLAUDE.md) or the 89 `M-*` rules in
[.claude/guidelines-index.md](../.claude/guidelines-index.md); where it names a skill, **invoke that
skill before writing the code for that step**. Every phase ends with the standard checks
(`cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, plus Miri and fuzzing where
stated) and the listed reviewer agents.

---

## 1. Scope and honesty constraints

Argos reads evidence. A wrong answer is worse than no answer, so the engine must model — and
report — what is physically recoverable:

- **HDD, quick format / deletion**: only metadata is destroyed; file content survives until
  overwritten. Filesystem-aware recovery and carving both apply. This is the primary target.
- **HDD, multiple re-formats**: each format overwrites a small metadata region. Content and even
  *previous* filesystem metadata (old MFT records, old superblocks, old inode tables) survive
  elsewhere on the platter. Argos scans the full surface for **residue of prior filesystems**, not
  just the current one (§3, stage B/C).
- **Full format (Windows Vista+ writes zeros) or overwritten regions**: unrecoverable. Argos must
  say so, never invent.
- **SSD/NVMe with TRIM**: after TRIM the controller returns zeros for trimmed LBAs; deleted data is
  usually gone from the host-visible surface. Argos detects the device class and TRIM status,
  scans what is visible, and reports the reduced expectation explicitly. No "chip-off" or
  controller-level access is in scope.

Consequences baked into every layer:

1. Devices are opened **read-only at the lowest layer**; no write path exists (CLAUDE.md
   non-negotiable #4). The one place Argos writes — the output directory — is validated against
   the source's *device identity*, not merely its path, so an output on a partition of the disk
   under analysis is refused before anything is created.
2. Every byte read from the medium is **untrusted input**: no unchecked arithmetic, allocation or
   indexing derived from an on-disk length/offset/count (non-negotiable #5).
3. Every recovered artifact carries a **provenance and confidence** record (§3, stage F). The tool
   distinguishes "recovered with filesystem metadata" from "reconstructed by carving" in all output.

Target formats: JPEG (JFIF/EXIF, baseline + progressive) and PNG. The pipeline is designed so a new
format is one new validator plugged into existing ports, but only these two are in scope.

## 2. Architecture — hexagonal, mapped to crates

Hexagonal (ports & adapters) is the right shape for this program and is kept. The hexagon's center
is pure, sans-IO domain logic; everything that touches an OS, a screen or a model file is an
adapter behind a port. The user-facing consequence: **the entire engine runs and is tested against
in-memory and file fixtures; no test touches a real device.**

### 2.1 Ports (traits, defined in `argos_core`)

Driven ports (the engine calls out through these):

| Port | Purpose | Adapters |
| --- | --- | --- |
| `BlockSource` | Sector-addressed, read-only access: `read_at(Lba, &mut [u8]) -> Result<…>`, geometry (`SectorSize`, capacity), bad-sector reporting, device class (HDD/SSD/image) | OS device HAL (`argos_device`), raw image file, in-memory fixture (`argos_core` test-util) |
| `ArtifactSink` | Receives validated artifacts + metadata; owns output layout | Output-directory writer (`argos_report`), collecting sink for tests |
| `Classifier` | Labels a decoded image: photograph vs synthetic asset | Deterministic rules (`argos_classify`), `AcceptAll` null adapter |
| `ProgressSink` | Structured progress/telemetry events | CLI renderer, Tauri event bridge, test collector |

Where `std` already provides the abstraction, the std trait **is** the port (`M-IMPL-IO`):
`argos_carve` consumes `impl Read + Seek`, exactly as CLAUDE.md states. `BlockSource` exists only
for what `Read + Seek` cannot express (sector addressing, bad sectors, geometry).

Driving port: `argos_engine` exposes a `ScanSession` service (cheap to clone, `M-SERVICES-CLONE`)
with `start / pause / cancel / subscribe`. Two driving adapters use it: the CLI, and the Tauri
shell (§6). Nothing else calls into the engine.

### 2.2 Crate layout (target; extends the CLAUDE.md table)

```
crates/argos/          CLI binary — stdout is its UI; also runs `--serve` mode (§6.2);
                       the only crate allowed an app-error crate (M-APP-ERROR)
crates/argos_core/     domain newtypes (Lba, ByteOffset, SectorSize, DeviceClass, Confidence),
                       port traits, canonical error structs shared across crates
crates/argos_device/   BlockSource adapters: per-OS HAL — the ONLY crate allowed `unsafe`
crates/argos_fs/       partition tables (MBR/GPT incl. backup GPT) + filesystem metadata
                       recovery (NTFS, ext4, FAT/exFAT, APFS) + prior-filesystem residue scan
crates/argos_carve/    signature carving, block classification, fragment reassembly, format
                       validators — sans-IO over `impl Read + Seek`
crates/argos_classify/ triage by deterministic image statistics + perceptual-hash dedup
crates/argos_engine/   the hexagon's center: scan pipeline, session lifecycle, concurrency,
                       merge/dedup of findings, confidence model
crates/argos_report/   findings, manifests, SHA-256 hashing, chain of custody, export
crates/argos_ipc/      the wire format between the engine process and its clients: DTOs,
                       schema version, JSON-RPC framing. Depends on nothing in the workspace
crates/argos_ui/       Tauri shell — presentation adapter only (§6). Excluded from the
                       workspace: the Tauri dependency tree must not decide the outcome of a
                       lint or an audit that exists for the code that reads evidence
```

Dependency DAG (arrows = "depends on"; must stay acyclic — enforce with `cargo tree` in CI):

```
argos    ──▶ argos_engine ──▶ argos_fs ──▶ argos_core
                          ──▶ argos_carve ──▶ argos_core
                          ──▶ argos_classify ──▶ argos_core
                          ──▶ argos_report ──▶ argos_core
argos    ──▶ argos_device ──▶ argos_core           (adapter injected into engine)
argos    ──▶ argos_ipc                             (translates engine ↔ wire)
argos_ui ──▶ argos_ipc                             (+ tauri; and nothing else)
```

`argos_ui` does **not** appear above `argos_ipc`. It cannot reach `argos_engine`, `argos_carve`
or `argos_fs`, so it cannot contain recovery logic — `A-SHELL-NO-DOMAIN` is a property of this
graph rather than something a review has to keep noticing (§6.1). `argos_ipc` depends on nothing
in the workspace, which is what makes it unable to leak an engine type onto the wire.

`argos_fs` and `argos_carve` never depend on each other or on `argos_device`; they meet only in
`argos_engine`. `argos_core` depends on nothing in the workspace. Circular references are
impossible by construction as long as this DAG is respected; any new edge must be justified
against it.

Phase 0 updates the CLAUDE.md **Layout** section to this table.

## 3. Recovery pipeline — algorithms (state of the art)

The engine is a staged pipeline. Stages are ordered by cost and trust: cheap, high-trust evidence
first (filesystem metadata), expensive reconstruction last. All stages emit into one merged
findings set, deduplicated by LBA extent and content hash.

**Stage A — Acquisition.** `BlockSource` over the raw device. Aligned reads with large buffers
(`O_DIRECT` on Linux to bypass page cache; buffer size and alignment come from queried geometry,
never hardcoded — `M-DOCUMENTED-MAGIC`). Bad-sector strategy is ddrescue-style multi-pass:
sequential sweep skipping ahead on error, then bisection refinement of skipped regions on later
passes; unreadable sectors are recorded in the report, never fabricated as zeros. Optional mode:
acquire to a raw image file first, then run all later stages against the image (safer for dying
disks; also the primary test path).

**Stage B — Volume analysis.** Parse MBR/GPT (including backup GPT at the last LBAs). Then a
**residue sweep**: scan the whole surface for orphaned volume anchors — NTFS boot sectors and
`FILE0` records, ext superblocks (magic `0xEF53` at the standard offsets plus backup-superblock
positions), APFS container superblocks (`NXSB`), FAT/exFAT boot sectors. This recovers the
geometry of partitions deleted by earlier re-formats and yields candidate regions for stage C
even when the current partition table shows nothing.

**Stage C — Filesystem-aware recovery** (per filesystem found in B, current or residual):

- **NTFS**: walk `$MFT` for records flagged deleted; independently signature-scan the surface for
  orphaned `FILE` records (survives re-format because record positions move). Parse resident data
  and non-resident data runs; recover names/timestamps from `$FILE_NAME`; mine `$I30` index slack
  for deleted directory entries; parse `$UsnJrnl` for names/paths of deleted files. On live
  Windows systems, enumerate Volume Shadow Copies as additional read-only sources.
- **ext4**: read superblock + backups; scan the jbd2 **journal** for stale copies of inode-table
  blocks — journaled old inodes retain extent trees of deleted files (deleted inodes in place have
  extents zeroed, the journal copy often does not). Parse extent trees; carve directory-entry
  blocks for orphaned names.
- **FAT32/exFAT** (cameras, SD cards — a primary source of user photos): deleted dirents (`0xE5`
  first byte / exFAT in-use bit), start cluster + size known, FAT chain lost → assume contiguity
  (valid for most camera writes) and hand non-validating results to stage E.
- **APFS**: parse the checkpoint descriptor area for **older container superblocks** — APFS's
  copy-on-write leaves a history of checkpoints; mount each recent checkpoint's object map and
  diff the filesystem trees to find recently deleted inodes; enumerate local snapshots.

Everything found here carries the highest confidence tier: exact extents, and usually original
filename and timestamps.

**Stage D — Content carving** (full-surface sequential sweep, sharing stage B's single pass over
the medium — the surface is read once and every chunk is offered to both detectors). Not
magic-bytes-only: each
candidate is driven through a **format state machine**:

- *JPEG*: SOI, then a legal marker-segment sequence (APPn/DQT/DHT/SOF/…) with length fields
  bounds-checked, then the entropy-coded stream tracked byte-by-byte (0xFF byte-stuffing, RSTn
  cadence) to EOI. Embedded EXIF thumbnails are extracted as separate lower-tier artifacts (a
  thumbnail may survive when its parent does not).
- *PNG*: signature, then chunk walk — length bounds-checked, CRC32 verified per chunk, IDAT zlib
  stream incrementally inflated, IHDR dimensions sanity-checked, through IEND.

A candidate that validates end-to-end is a contiguous recovery (high confidence). A candidate that
starts valid and breaks mid-stream marks a **fragmentation point** and is queued for stage E with
the exact break offset — this decode-feedback localization is what makes reassembly tractable.

**Stage E — Fragment reassembly** (the hard 5–10% of real-world cases):

1. **Block classification**: every device block gets a cheap feature vector (Shannon entropy, byte
   histogram distance, JPEG-entropy-stream detector: 0xFF-stuffing statistics and absence of
   forbidden markers; zlib/deflate detector for PNG). This partitions the surface into candidate
   media blocks vs everything else, shrinking the reassembly search space by orders of magnitude.
2. **Bifragment gap carving** (Garfinkel, DFRWS 2007): for a header whose decode broke at offset
   *k* and a matching footer, search gap sizes between the two fragments, validating each
   hypothesis with the real decoder. Covers the dominant fragmentation pattern (two fragments,
   small gap).
3. **Graph reassembly** for >2 fragments (Pal/Sencar/Memon — SmartCarver / Parallel Unique Path):
   candidate fragment-adjacency graph weighted by decoder continuation success and pixel-row
   boundary smoothness (sum of luminance differences across the stitch line); greedy PUP walk
   assembles the best path per header. JPEG restart markers, when present, are exploited as
   independent re-entry points so orphaned mid-file fragments can be decoded standalone
   (Uzun & Sencar's orphaned-fragment technique).
4. Every reassembled artifact is flagged `reconstructed`; the report records the exact extent list
   used, so the result is reproducible and auditable.
5. Because the search tests thousands of hypotheses, its oracle is an **entropy decode** — every
   MCU the frame declares, then `EOI` — not structural validation: the JPEG marker grammar is
   loose enough that spliced random bytes regularly produce a structurally complete, fabricated
   image, and the position such a parse reaches measures disk position rather than evidence. PNG
   is exempt; its per-chunk CRC32 already rules that out.

**Stage F — Validation and scoring.** A finding from stage C is only accepted once the medium
confirms it twice: an image signature at its first extent, and the *assembled* extents passing the
same format state machine a carved candidate must pass. Metadata that survived a format can point
anywhere, so without both checks a stale extent list would be reported at the strongest tier there
is — and the strongest tier would be the least verified one. A recovery whose bytes are present but
whose structure breaks (a spliced hole, a reallocated run) is still reported, because the metadata
is real evidence a file lived there, but at the partial tier and with its expected length recorded
beside what was actually recovered. Then full decode with pure-Rust decoders (`zune-jpeg`, `png`) —
memory-safe against hostile input by construction. Compute: decode completeness, truncation
percentage, structural warnings. Assign the confidence tier: `FsMetadata > JournalResidue >
ContiguousCarve > Reassembled > PartialOrThumbnail`. Hash (SHA-256) at the moment of recovery for
chain of custody.

**Stage G — ML triage.** Perceptual-hash dedup first (blockhash/pHash) so near-duplicates collapse
before inference. Then a small CNN binary classifier — *user photograph* vs *synthetic asset*
(icons, sprites, UI chrome, web cache graphics), which resolution alone cannot decide. A
rule-based pre-filter (dimensions, alpha usage, palette size, edge/color statistics) short-cuts
obvious cases so the CNN only sees ambiguous ones. Inference is **pure Rust** (`tract` or
`candle`), local-only, model file versioned and SHA-256-pinned. **Triage never deletes**: it
orders and labels results; the user decides. Full policy: `argos-ml-triage` skill (§7).

**Stage H — Reporting** (`argos_report`). Manifest of every artifact: provenance (stage, extents,
source filesystem object if any), confidence tier, hashes, timestamps, classifier score, plus the
scan log (device identity, geometry, unreadable sectors, tool version). Machine-readable (JSON,
DFXML-inspired) + human summary. Nothing in logs or the manifest exposes recovered *content*
beyond what the user explicitly exports (`M-LOG-STRUCTURED`).

## 4. Concurrency plan

Invoke `rust-concurrency` before implementing any of this. The engine is threads + channels — no
async runtime in the core (async appears only inside the Tauri adapter, which is where Tauri
requires it).

Where parallelism is fundamental vs forbidden:

| Stage | Model | Why |
| --- | --- | --- |
| Device reads (A) | **One sequential reader thread** per device; chunk buffers travel to and from the workers through a pool channel, which bounds memory and applies backpressure in one mechanism (`M-MEM-REUSE`) | HDD throughput collapses under seek storms; the medium is the bottleneck, sequential sweep maximizes it. SSD sources may raise queue depth later — measured, not assumed (`M-HOTPATH`) |
| FS metadata pass (B/C) | Sequential, targeted random reads, interleaved before the sweep | Tiny I/O volume; parallelism buys nothing and costs seeks |
| Block classification + signature scan (D/E1) | **Scoped worker pool** over chunks from the reader; each worker owns its chunk end to end | Pure CPU on immutable buffers; chunks are equal-sized, so partitioning ahead beats work stealing (`M-THROUGHPUT`) |
| Object validation/decode (D/F) | Scoped worker pool over a candidate queue, one medium view per worker | Decode is the CPU hot path, and per-candidate cost is uneven, so workers pull rather than being handed a slice |
| Reassembly search (E2/E3) | Worker pool per header; the graph search itself is parallel-friendly (PUP) | Search dominates; independent per target image |
| Hashing + report (F/H) | Fold into the validation task per artifact | Avoids a serialization point |
| ML inference (G) | One dedicated batch worker thread | Batching amortizes; isolates the model runtime |
| Stage wiring | **Bounded** `crossbeam-channel`s | Backpressure: the reader must stall rather than buffer the whole disk in RAM |
| Cancellation | One control flag (atomic) checked at chunk granularity everywhere; pausing parks on a condvar rather than spinning | Pause/cancel from CLI/UI must take effect in ≤ one chunk |

All pipeline types are `Send` (`M-TYPES-SEND`); no `static` mutable state (`M-AVOID-STATICS`);
worker counts derive from `available_parallelism`, configurable, never hardcoded.

## 5. Multi-OS device HAL (`argos_device`)

The only `unsafe` crate. Invoke `rust-unsafe-ffi` before every change here; run
`cargo +nightly miri test -p argos_device` whenever unsafe changes. Every syscall goes through a
mockable seam (`M-MOCKABLE-SYSCALLS`) so the crate's logic is testable without a device.

| OS | Open | Geometry | Notes |
| --- | --- | --- | --- |
| Linux | `open(O_RDONLY \| O_DIRECT)` on `/dev/sdX` / `/dev/nvmeXnY` | `BLKGETSIZE64`, `BLKSSZGET`, `BLKPBSZGET` ioctls | Needs root or `disk` group; warn if any partition is mounted |
| Windows | `CreateFileW("\\\\.\\PhysicalDriveN", GENERIC_READ, FILE_SHARE_READ\|WRITE, …)` | `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX`, `IOCTL_DISK_GET_LENGTH_INFO`, `IOCTL_STORAGE_QUERY_PROPERTY` (TRIM/seek-penalty) | Needs Administrator; enumerate via SetupAPI/`IOCTL_STORAGE_GET_DEVICE_NUMBER` |
| macOS | `open(O_RDONLY)` on `/dev/rdiskN` (raw, unbuffered) | `DKIOCGETBLOCKSIZE`, `DKIOCGETBLOCKCOUNT` | Needs root + Full Disk Access (TCC); DiskArbitration to enumerate and to check mount state |

Handles are wrapped in owning newtypes; sector size and capacity are queried once and carried in
the `BlockSource` geometry — 512e vs 4Kn alignment is handled here and nowhere else. Elevation UX
is the shell's job (§6.2); the HAL only reports "permission denied" precisely
(`M-ERRORS-CANONICAL-STRUCTS`).

## 6. Tauri — a shell and nothing more

### 6.1 The contract

1. `argos_ui` contains **zero recovery logic**: no parsing, no scoring, no filesystem knowledge,
   no thresholds. Its Rust side is Tauri commands that translate DTO ↔ `argos_engine` calls, each
   command a thin function (target: no branching beyond error mapping).
2. The **CLI is the proof**: every capability ships CLI-first and works headless. If a feature
   needs the UI to function, the boundary is broken — reject the change.
3. One IPC module defines all DTOs (serde, versioned with a schema version field). DTOs never leak
   engine types (`M-DONT-LEAK-TYPES`) and never contain raw sector data.
4. Progress flows engine→UI via Tauri **events** fed from the `ProgressSink` port; commands never
   poll. Thumbnails/previews are written by `argos_report` into the session output directory and
   served via Tauri's asset-protocol scope restricted to that directory — never base64 over IPC.
5. Frontend stays minimal: **Svelte 5 + TypeScript**, no router, no state library, no CSS
   framework. State is a mirror of engine events and user view preferences; if a value is
   computed in TS, it is display formatting only. The IPC types are **generated** from
   `argos_ipc` — one definition of the wire format, two languages.
6. **The layout is fixed**: one screen, three blocks — the drive table, the destination folder,
   and the live activity block (two progress rings and five figures) — plus one button that
   starts and stops the run, and a gear in the title bar that opens the theme picker. There are
   no other views and no navigation. That button runs exactly what
   `argos scan <source> --out <destination>` runs, with no options of its own; a window that
   could ask for a different recovery than the command line would be a second interface to the
   engine rather than a view of it.
7. **Every figure the window shows has one source, and estimates say so.** Bytes analysed comes
   from the sweep's progress; artifacts and bytes *recovered* are the engine's own counts of what
   reached the output directory, never candidates seen — this pipeline validates after it sweeps,
   so both sit at zero while the surface is read, and that is reported rather than filled in
   (`A-CONFIDENCE-HONEST`). Only "remaining" is arithmetic, and it is marked `≈`.
8. **Every stage reports, and says what it counts.** A run spends most of its time in passes that
   are not the read — validation drives every signature hit through a state machine, triage runs
   inference over every artifact — so each stage announces itself and reports progress in its own
   unit, bytes or items, with the unit on the wire. The scan ring follows the stage in progress
   and the status line names it; one bar spanning passes measured in different things would need
   an invented exchange rate between bytes and candidates. "Remaining" is shown only while the
   medium is being read, because that is the only pass whose rate predicts anything.
9. Themes are presentation modules and nothing more. The base layout owns all structure and
   behaviour and reads only `var(--token)`; a theme supplies a value for every token in a total
   `Record<ThemeToken, string>`, so **a theme missing a token does not compile**. Switching one
   rewrites custom properties on the document root: no remount, no lost state, safe mid-scan.
   One theme ships, `default`.
10. Tauri allowlist/capabilities: only the commands and the one asset scope — no shell, no fs, no
    http permissions to the webview.

### 6.2 Process model and elevation

Raw device access needs root/Administrator, and a GUI should not run elevated wholesale. The
`argos` binary therefore has a `--serve` mode speaking **JSON-RPC over stdio** (newline-delimited,
same DTO module as §6.1-3). The Tauri app spawns `argos --serve` elevated (UAC manifest on
Windows, `pkexec`/`osascript` elevation on Linux/macOS) and bridges stdio ↔ Tauri events. The
engine process is the hexagon; the UI is a client. This makes the "Tauri is only a shell"
requirement structural rather than disciplinary — the shell literally talks to the same interface
as any other client. Scanning a non-privileged image file falls back to running the engine
in-process with no elevation.

## 7. New skills and agents (create in Phase 0 with `skill-creator`)

The 89 `M-*` rules stay untouched. Argos-specific rules get the **`A-*` prefix** in the new
skills' `reference.md` files, listed in a separate section of `guidelines-index.md` so the
89-count integrity check keeps passing. Same conventions: one rule → one file, reviewers cite ids.

| Skill / agent | Kind | Covers |
| --- | --- | --- |
| `argos-evidence-handling` | skill | `A-READ-ONLY` (no write path to source media, ever), `A-UNTRUSTED-ONDISK` (checked arithmetic/allocation/indexing for any medium-derived value — the concrete patterns: `checked_*`, `get()`, capped allocations), `A-PROVENANCE` (every artifact records stage + extents + hashes), `A-CONFIDENCE-HONEST` (never report above the evidence tier), `A-NO-CONTENT-IN-LOGS` |
| `argos-ondisk-parsing` | skill | How on-disk structure parsers are written here: layout structs via `zerocopy`-style checked reads, no `unsafe` in parsers, every parser ships a fixture builder (`M-TEST-UTIL`) and a `cargo-fuzz` target (`A-FUZZ-EVERY-PARSER`), corrupt-input tests are first-class |
| `argos-recovery-algorithms` | skill | Fixed written specs of §3's algorithms (residue sweep, NTFS orphan-MFT, ext4 journal mining, APFS checkpoints, JPEG/PNG state machines, block classification, bifragment gap, PUP) with literature references — agents implement from the spec instead of improvising |
| `argos-ml-triage` | skill | `A-MODEL-PINNED` (local file, SHA-256-pinned, versioned; no runtime downloads), `A-TRIAGE-NOT-VERDICT` (classifier orders/labels, never discards or deletes), eval-set + threshold policy, pure-Rust inference requirement |
| `argos-tauri-shell` | skill | The §6 contract as checkable rules: `A-SHELL-NO-DOMAIN`, `A-CLI-FIRST`, `A-DTO-VERSIONED`, `A-EVENTS-NOT-POLLING` |
| `forensic-boundary-reviewer` | agent (read-only, like the four existing reviewers) | Audits untrusted-input parsing, evidence-handling invariants and the UI/IPC boundary; cites `A-*` and `M-*` ids |

AI-safety guardrails for agents working on this repo (also written into the
`argos-evidence-handling` skill, and enforced via hooks configured with the `update-config`
skill): Bash hooks **deny** any command writing to `/dev/*` or invoking `dd`/`mkfs`/`shred`/
`blkdiscard` with a device target; tests run exclusively against generated fixtures and image
files; real-device runs are manual, human-initiated, behind `#[ignore]`.

## 8. Phases

Every phase: invoke the listed skills *before* coding, run the standard checks, then the listed
reviewers. A phase is done when its exit criterion passes from a clean checkout.

**P0 — Workspace migration and standards plumbing.**
Split into the §2.2 workspace (flat `crates/` folder, workspace lints per `M-STATIC-VERIFICATION`,
edition/MSRV policy, `mimalloc` for the binary, CI wiring: fmt/clippy/test/miri lane/cargo-audit/
cargo-hack). Create the five `A-*` skills + reviewer of §7; add the `A-*` section to
`guidelines-index.md`; update CLAUDE.md's Layout; set up the deny-hooks. Delete the placeholder
root `src/`.
*Skills*: `rust-workspace-setup`, `skill-creator`, `update-config`. *Review*: `rust-design-reviewer`.
*Exit*: empty crates build green in CI on Linux/Windows/macOS runners; index integrity check passes.

**P1 — Domain core and Linux acquisition.**
`argos_core`: newtypes (`Lba`, `ByteOffset`, `SectorSize`, `DeviceClass`, `Confidence`), port
traits, canonical errors. `argos_device`: Linux HAL (`O_DIRECT` reader, geometry ioctls, mockable
syscall seam), raw-image-file `BlockSource`, in-memory fixture source, ddrescue-style multi-pass
bad-sector strategy, acquire-to-image mode.
*Skills*: `rust-type-design`, `rust-api-surface`, `rust-errors-panics`, `rust-unsafe-ffi`,
`rust-testing`, `argos-evidence-handling`. *Review*: `rust-safety-reviewer`,
`forensic-boundary-reviewer`; Miri. *Exit*: acquire a loopback test image with injected read
errors; sector-accurate output, errors mapped in the report structure.

**P2 — Structure-validated carving MVP (contiguous), CLI v0.**
`argos_carve`: JPEG and PNG state-machine validators over `impl Read + Seek`, EXIF-thumbnail
extraction, fuzz targets per parser, fixture builders. `argos_report` v0: manifest + SHA-256.
`argos` CLI: scan an image file, recover contiguous deleted images end-to-end.
*Skills*: `argos-ondisk-parsing`, `argos-recovery-algorithms`, `rust-performance`,
`rust-testing`, `rust-docs`. *Review*: `rust-perf-reviewer`, `rust-test-reviewer`,
`forensic-boundary-reviewer`. *Exit*: on a generated fixture disk (files deleted, fs unmounted),
100% of contiguous images recovered bit-identical; fuzzers run clean for a fixed budget.

**P3 — Filesystem-aware recovery.**
`argos_fs` in order: MBR/GPT + backup GPT + residue sweep → NTFS (deleted MFT walk, orphan `FILE`
scan, data runs, `$I30` slack, `$UsnJrnl`) → ext4 (backup superblocks, jbd2 journal inode mining,
extent trees) → FAT32/exFAT → APFS (checkpoint history, omap, snapshots). Each parser: fixture
builder + fuzz target. Fixtures are scripted images made with `mkfs`/format tools **on loopback
files only**.
*Skills*: `argos-ondisk-parsing`, `argos-recovery-algorithms`, `argos-evidence-handling`,
`rust-testing`. *Review*: `forensic-boundary-reviewer`, `rust-test-reviewer`, `rust-design-reviewer`.
*Exit*: per-filesystem fixture suites recover deleted files with names/timestamps; a
double-reformatted fixture (ext4 → NTFS) still yields the pre-format files via residue sweep.

**P4 — Engine: pipeline, sessions, confidence.**
`argos_engine`: staged pipeline of §4 (reader thread, bounded channels, worker-pool stages,
cancellation, buffer pool), merge/dedup of stage C+D findings by extent and hash, the confidence
model of stage F, `ScanSession` service, `ProgressSink` events. CLI gains progress UI,
pause/cancel, full-device scans (Linux).
*Skills*: `rust-concurrency`, `rust-performance`, `rust-telemetry`, `rust-type-design`.
*Review*: `rust-perf-reviewer`, `rust-design-reviewer`. *Exit*: full pipeline over a multi-GB
fixture saturates either disk or CPU (measured); cancel latency under one chunk; deterministic
manifest across runs.

**P5 — Fragment reassembly.**
In `argos_carve`: block classifier (entropy/histogram/JPEG-stream/zlib detectors), decode-feedback
fragmentation-point detection, bifragment gap carving, PUP graph reassembly with pixel-boundary
smoothness weighting, restart-marker orphan decoding. Benchmarks with criterion; this is the CPU
hot path (`M-HOTPATH` — measure before optimizing).
*Skills*: `argos-recovery-algorithms`, `rust-performance`, `rust-concurrency`.
*Review*: `rust-perf-reviewer`, `forensic-boundary-reviewer`. *Exit*: fixture suite of
synthetically fragmented images (2-fragment and n-fragment, known ground truth) with measured
recovery rates reported per pattern; all reassembled artifacts flagged `reconstructed`.

**P6 — ML triage.** *(delivered)*
`argos_classify`: rule-based pre-filter; CNN (MobileNet-class) exported to a pinned local model
file; inference via `candle` (pure Rust, chosen so training and inference share one graph
definition); batch worker; perceptual-hash dedup. Training pipeline lives outside the workspace
was removed with the model it produced: triage is deterministic image statistics, and what enters
the repo is the rules, the eval harness and the threshold-derivation harness beside it.
Threshold defaults are named constants derived on the trainer's validation range, never on the
eval corpus (`M-DOCUMENTED-MAGIC`).
*Skills*: `argos-ml-triage`, `rust-performance`, `rust-testing`. *Review*: `rust-test-reviewer`,
`forensic-boundary-reviewer`. *Exit*: eval set (photos incl. greyscale and thumbnails vs
icons/sprites/UI chrome/high-res assets) with precision/recall targets met and recorded in the
eval harness, for the shipped pipeline **and** for the model alone; classifier output provably
never filters artifacts out of the manifest.

**P7 — Windows and macOS HALs.** *(code delivered; hardware verification outstanding)*
Per-OS `BlockSource` adapters (§5 table), device enumeration, TRIM/seek-penalty detection, VSS
enumeration (Windows), mount-state warnings. Cross-OS CI matrix runs the full non-privileged test
suite, plus a cross-target lane that type-checks and lints every HAL from one machine.

Each platform's *decisions* — path conventions, whole-disk vs partition, class from the OS's
answer, mount-table parsing — live in `naming.rs`, `class.rs` and `inventory/mount.rs`, which
compile on every target and are therefore tested on every target. Only the syscalls sit behind
`cfg` (`M-MOCKABLE-SYSCALLS`).
*Skills*: `rust-unsafe-ffi`, `rust-testing`, `rust-workspace-setup`. *Review*:
`rust-safety-reviewer`; Miri per platform. *Exit*: image-file scans identical across the three
OSes — mechanised as a recorded provenance digest the whole CI matrix checks; manual elevated
smoke checklist on real hardware ([docs/DEVICE-SMOKE-CHECKLIST.md](DEVICE-SMOKE-CHECKLIST.md))
documented and executed once per OS. **The checklist is written and unexecuted**: the ioctl
request codes and storage-driver descriptors cannot be verified without the hardware, and no
value they produce should be trusted until a row appears in its results table.

**P8 — Tauri shell.** *(delivered; elevation on Windows/macOS outstanding)*
In order:

- **P8.1 — the capabilities the CLI was missing.** `ArtifactSink::preview`, a defaulted port
  method the annotation pass calls with the pixels it already decoded for triage, so previews and
  triage share one decode and previews do not depend on triage running. `argos_report` renders
  them to `previews/<sha256>.jpg`, keyed by content hash exactly as triage annotations are.
  `argos scan --previews` (opt-in: a forensic tool does not write derived files nobody asked
  for), `argos report <session>` and `argos export --from … --to … [--sha256 …]`, which re-hashes
  every artifact while copying and refuses one whose bytes no longer reproduce the recorded
  digest. The scan driver moves out of `main.rs` into `scan.rs`, reporting through a sink — in
  `serve` mode stdout is the protocol and a stray `println!` corrupts it.
- **P8.2 — `argos_ipc` and `argos serve`.** The wire format in a crate that depends on nothing:
  `SCHEMA_VERSION` in a mandatory handshake, JSON-RPC 2.0 one value per line, a `Call` enum so an
  unknown method fails at the edge. The engine reads stdin, dispatches, and pushes progress as
  notifications. Its exit criterion is the **parity test**: a scan driven over the pipe recovers
  byte-for-byte and record-for-record what `argos scan` recovers from the same medium.
- **P8.3/P8.4 — the shell.** `argos_ui` spawns the engine, bridges notifications to Tauri events
  and grants the web view the session's `previews/` directory and nothing else. Frontend per
  §6.1-5: Svelte 5, a base layout, an event mirror, and themes as total token records.
- **P8.5 — elevation.** `pkexec` on Linux. Windows and macOS elevate through a shell verb that
  drops the caller's pipes, which is refused with an explanation rather than silently producing
  an unprivileged scan; see §6.2.

*Skills*: `argos-tauri-shell`, `rust-api-surface`, `rust-telemetry`. *Review*:
`forensic-boundary-reviewer` (boundary audit: zero domain logic in `argos_ui`),
`rust-design-reviewer`. *Exit*: every UI action reproducible via CLI alone — mechanised by the
parity test; UI is a pure client of `serve`, mechanised by the dependency graph; capability
allowlist minimal; `dto.ts` regenerated in CI and diffed, so the two languages cannot drift.

**P9 — Hardening and forensic validation.**
Extended fuzz corpus (real-world file corpora), long-run fuzzing in CI, performance baselines,
end-to-end validation against public forensic test images (DFRWS/NIST CFReDS carving suites),
crash-free guarantee on hostile images, final docs pass (`rust-docs` on every public item),
packaging/signing for the three OSes.
*Review*: all five reviewers over the full diff since P0. *Exit*: recovery-rate report against
the public suites checked into `docs/`; zero clippy/fmt/miri/fuzz findings.

## 9. Engineering directives (mapped to the standard)

These restate the user-facing quality bar in terms of rules agents already enforce — cite the id,
not this section:

- **No magic numbers**: every on-disk constant (magics, offsets, marker bytes) is a named constant
  with its meaning and source documented — `M-DOCUMENTED-MAGIC`.
- **No circular references**: the §2.2 DAG is law; new inter-crate edges require updating §2.2
  first — `M-SMALLER-CRATES`, `M-CARGO-WORKSPACE`.
- **Reuse over duplication**: shared test fixtures live in `test-util` features (`M-TEST-UTIL`);
  shared domain vocabulary lives in `argos_core`, never copy-pasted; abstractions stay simple and
  are introduced on the second use, not speculatively — `M-SIMPLE-ABSTRACTIONS`,
  `M-AVOID-WRAPPERS`.
- **Readability**: naming per `M-SHORT-NAMES`/`M-WEASEL-WORDS`; modules balanced
  (`M-BALANCED-MODULES`); docs on every public item with a meaningful first sentence
  (`M-FIRST-DOC-SENTENCE`).
- **Failure discipline**: corruption, bad sectors and unknown filesystems are `Result`s; panics
  are for bugs only and carry the offending values — `M-PANIC-ON-BUG`, `M-PANIC-MESSAGE`.
- **No design-journey artifacts** in code, docs or commits — `M-NO-META-DESIGN-DOCUMENTATION`.
  This plan describes the target state and build order; it is not a rationale essay and must not
  spawn them.
