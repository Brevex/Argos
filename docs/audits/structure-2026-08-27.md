# Structural consolidation audit — Rust backend

Date: 2026-08-27. Scope: the nine workspace crates under `crates/`. `crates/argos_ui` was **not
opened**; it is named only where a boundary depends on it. Support targets — `tests/`, `benches/`,
`examples/`, `fuzz/` — are in scope for their *count*, never for being folded into a library.

Working tree as it stands, including the uncommitted btrfs work (13 modified files) and the three
untracked documents. Read-only audit: no source file was edited; this document is the only write.

The previous audit is [structure-2026-08-23.md](structure-2026-08-23.md) and it was executed as
commit `b556d2b`. This one measures what that pass left standing and where it overshot.

---

## 1. Verdict

The tree is **roughly right and no longer inflated**; what remains is one crate's worth of residue
and one directory pair that mirrors itself. `b556d2b` removed 17 files and split a 2,598-line
`pipeline.rs`; four days on, only 7 `src/` files sit under 150 lines and three of those are crate
roots. There is no `mod.rs` anywhere, no `utils` module, no `Manager`, no `#[allow]`, no unused
dependency, no dead field and no dead variant.

- Now: **9 crates, 28 directories, 124 `.rs` files, 45,309 lines**, median file 262 lines,
  35 files under 150 lines (7 in `src/`), 8 files with exactly one consumer.
- Projected: **9 crates, 27 directories, 109 `.rs` files, 45,309 lines**, median 328, 28 files
  under 150 (4 in `src/`). `src/` goes 72 → 61 files, −15%.
- **Biggest lever:** `argos_device`, 14 `src/` files → 10. Two mirrored per-OS directories
  (`src/device/` and `src/inventory/`) become one `src/platform/`, and `shadow.rs` — a merge the
  2026-08-23 audit proposed and nobody executed — folds into `inventory.rs`.
- Second lever: `argos_core`, 7 files → 3. Four 207–265-line port modules and a 265-line geometry
  module are each nameable but none is large enough that merging would break its host.
- **Counterweight:** nothing here needs a `SPLIT`. `argos_fs/src/fixture.rs` (2,012) is at the
  limit and is flagged, not split; `pipeline/` is one file per stage and must stay that way.
- **No crate merges.** All nine answer at least one of the four questions and seven of them are the
  mechanical enforcement of the §2.2 DAG.
- Regime B holds **one** item, and it is a fixture constant. That is the right outcome for a tree
  where `udeps`, `machete`, clippy-with-nursery and a compiler-verified visibility sweep all came
  back clean.

---

## 2. Target file tree

### 2.1 Crates — all nine survive

Four questions: (1) enforces a dependency direction; (2) isolates a heavy or optional dependency;
(3) consumed by more than one crate, or published; (4) materially improves incremental build.

| Crate | Question answered | Consumers | Verdict |
| --- | --- | --- | --- |
| `argos` | binary entry point; the only crate allowed an app-error crate (`M-APP-ERROR`). Also the engine *process* (`serve`) | — | `KEEP` |
| `argos_core` | **1 + 3** — seven crates depend on it and it depends on none. The four port traits live here and the compiler is what stops an adapter type reaching the domain | 7 workspace crates | `KEEP` |
| `argos_device` | **1 + 2** — the only crate allowed `unsafe`, the only one linking `libc`/`windows-sys`. `argos_engine` cannot see it, which is what makes the engine testable without a device | `argos` | `KEEP` |
| `argos_fs` | **1** — DEVELOPMENT-PLAN §2.2 forbids it depending on `argos_carve`, `argos_classify` or `argos_device`; the crate boundary is the enforcement | `argos_engine` | `KEEP` |
| `argos_carve` | **1 + 2** — same DAG rule; isolates `zune-jpeg`, `zune-png`, `miniz_oxide`, `memchr` | `argos_engine` | `KEEP` |
| `argos_classify` | **1** — same DAG rule. The weakest case at 1,187 `src/` lines, but merging it into `argos_engine` would let triage reach the pipeline, which is exactly what `A-TRIAGE-NOT-VERDICT` is structurally guarding | `argos_engine`, `argos` | `KEEP` |
| `argos_engine` | **1** — the hexagon's centre; `argos_fs`, `argos_carve` and `argos_classify` meet only here | `argos` | `KEEP` |
| `argos_report` | **1 + 2** — `argos_engine` deliberately does *not* depend on it, so the sink stays a port rather than a call; isolates `serde_json` and `jpeg-encoder` from the engine | `argos` | `KEEP` |
| `argos_ipc` | **1 + 3** — depends on nothing in the workspace, which is what makes it unable to name an engine type; consumed by `argos` and by `argos_ui` outside the workspace | `argos`, `argos_ui` | `KEEP` |

`cargo tree --edges normal` confirms the DAG: `argos_fs`, `argos_carve` and `argos_classify` each
depend on `argos_core` and nothing else in the workspace; `argos_ipc` depends on nothing in it.

**Boundary caveat, applied and honoured.** Three merges were considered and all three are refused:
`argos_classify` → `argos_engine`, `argos_report` → `argos`, and `argos_core` → each adapter. Each
dissolves an enforced architectural boundary. Replacement enforcement would be module visibility
plus a `cargo tree`/`cargo deny` dependency test — strictly weaker, because a convention is what
the next contributor breaks silently while a missing crate edge does not compile. `M-SMALLER-CRATES`
points the same way. The build direction is also against merging: the nine crates compile in
parallel and rebuild independently, so collapsing them would *lengthen* incremental builds.
**No crate merge is proposed.**

### 2.2 Files — before and after

Sizes are current lines (before) and projected lines (after). `→` marks a move; `+` marks an
absorbed file. No line is deleted by any move.

```
BEFORE                                       AFTER
crates/argos_core/src/          7 files      crates/argos_core/src/          3 files
  lib.rs                    202                lib.rs                    467  the domain vocabulary: Stage, Format,
  geometry.rs               265  →lib.rs                                      Confidence, Timestamps and the positions
                                                                              they refer to — Lba, ByteOffset,
                                                                              SectorSize, the two ranges. One file, one
                                                                              answer to "what does this crate call things"
  source.rs                 207                ports.rs                  896  the four ports of the hexagon and the
  artifact.rs               263  →ports.rs                                    vocabulary each carries: BlockSource +
  classify.rs               217  →ports.rs                                    Geometry/ReadError, ArtifactSink +
  progress.rs               209  →ports.rs                                    Artifact/Digest/Capture, Classifier +
                                                                              PixelImage/TriageScore, ProgressSink +
                                                                              ScanEvent/Unit/RunState. The crate doc
                                                                              already tabulates the four together
  fixture.rs                110                fixture.rs                110  feature-gated MemDisk. Stays a file
                                                                              because five crates look for
                                                                              `<crate>::fixture` by that exact name

crates/argos_device/src/       14 files      crates/argos_device/src/       10 files
  lib.rs                     22                lib.rs                     22  crate doc + the public surface
  device.rs                 560                device.rs                 560  every medium this crate opens behind
                                                                              BlockSource, and the byte-addressed view
                                                                              over any of them
  inventory.rs              206                inventory.rs              348  what media this machine offers: disks,
  shadow.rs                 142  →inventory.rs                                partitions, their mounts, and the shadow
                                                                              copies a platform keeps
  inventory/mount.rs        248                mount.rs                  248  mount-table parsing, compiled and tested
                                                                              on every target — a decision, not a syscall
  class.rs                  212                class.rs                  212  device class + TRIM from an OS answer
  naming.rs                 390                naming.rs                 390  path conventions + whole-disk rules
  acquire.rs                463                acquire.rs                463  ddrescue-style multi-pass acquisition
  device/linux.rs           260                platform/linux.rs         368  the Linux syscall layer: O_DIRECT opens,
  inventory/linux.rs        108  →platform/                                   geometry ioctls, sysfs enumeration
  device/macos.rs           198                platform/macos.rs         306  the macOS syscall layer
  inventory/macos.rs        108  →platform/
  device/windows.rs         358                platform/windows.rs       663  the Windows syscall layer
  inventory/windows.rs      305  →platform/

crates/argos_fs/src/            9 files      crates/argos_fs/src/            9 files   unchanged
  lib.rs                    300                lib.rs                    300  crate vocabulary + the checked reads all
                                                                              six parsers are built on
  part.rs                   284                part.rs                   284  MBR/GPT incl. backup GPT
  residue.rs                343                residue.rs                343  prior-filesystem anchor sweep
  ntfs.rs                  1386                ntfs.rs                  1386  one file per on-disk format, one fuzz
  btrfs.rs                 1084                btrfs.rs                 1084  target each (A-FUZZ-EVERY-PARSER)
  ext4.rs                   761                ext4.rs                   761
  fat.rs                    624                fat.rs                    624
  apfs.rs                   543                apfs.rs                   543
  fixture.rs               2012                fixture.rs               2012  synthetic volume builders (test-util).
                                                                              At the size limit — see §8

crates/argos_carve/src/        10 files      crates/argos_carve/src/         9 files
  lib.rs                    704                lib.rs                    704  the carving surface: Carver, Detector,
                                                                              validate, Candidate, Finding, CarveError,
                                                                              and the bounded cursor its validators read
                                                                              the medium through
  jpeg.rs                   265                jpeg.rs                   535  the JPEG marker state machine and the
  exif.rs                   270  →jpeg.rs                                     TIFF/IFD payload it mines out of APP1 —
                                                                              EXIF is a JPEG segment, not a format
  png.rs                    405                png.rs                    405  PNG chunk walk + per-chunk CRC
  classify.rs               318                classify.rs               318  block classification
  decode.rs                 332                decode.rs                 332  pixel decode, the reassembly oracle
  mcu.rs                   1456                mcu.rs                   1456  entropy-stream decode
  reassemble.rs            1613                reassemble.rs            1613  the search: gap carve, PUP walk, Assembled
  reference.rs              235                reference.rs              235  a surviving header lent to orphans
  fixture.rs               1135                fixture.rs               1135  decodable image and disk builders

crates/argos_classify/src/      4 files      crates/argos_classify/src/      4 files   unchanged
  lib.rs                    198                lib.rs                    198  what triage does to one image: label it,
                                                                              hash it so near-duplicates collapse
  rules.rs                  330                rules.rs                  330  the rules and their thresholds
  rank.rs                   176                rank.rs                   176  standing: ordering by the evidence a
                                                                              record carries — pixels never enter it
  fixture.rs                483                fixture.rs                483  labelled corpus generator (test-util)

crates/argos_engine/src/       13 files      crates/argos_engine/src/       12 files
  lib.rs                    139                lib.rs                    139  crate doc, public surface, ScanError
  session.rs                390                session.rs                390  the driving port: start/pause/cancel
  config.rs                 412                config.rs                 412  what a scan covers, how much machine
  finding.rs                699                finding.rs                699  findings, their merge, cache runs
  annotate.rs               554                annotate.rs               554  the post-emit pass: one decode feeding
                                                                              preview and triage
  graft.rs                  212                graft.rs                  212  the orphan sweep — deliberately not a stage
  fixture.rs                160                fixture.rs                160  Collector sink + event collector
  pipeline.rs               869                pipeline.rs               869  stage order, progress counters, and the one
                                                                              sequential pass over the medium
  pipeline/carving.rs       260                pipeline/carving.rs       260  stage D: candidates → findings
  pipeline/filesystem.rs    479                pipeline/filesystem.rs    479  stage C: volumes, MFT, journal, slack
  pipeline/output.rs        329                pipeline/output.rs        329  stages F/H: measure, hash, write, count
  pipeline/reassembly.rs    641                pipeline/reassembly.rs   1071  stage E: the region a search reads from,
  pipeline/region.rs        430  →reassembly                                  the plan, the budget and the search

crates/argos_report/src/        2 files      crates/argos_report/src/        2 files   unchanged
  lib.rs                    879                lib.rs                    879  one output directory and everything
                                                                              written into it: the Store sink, previews,
                                                                              ownership handback, ReportError
  manifest.rs               527                manifest.rs               527  the manifest schema and its read/write —
                                                                              the thing four separate readers parse

crates/argos_ipc/src/           3 files      crates/argos_ipc/src/           3 files   unchanged
  lib.rs                     88                lib.rs                     88  SCHEMA_VERSION + the contract doc
  dto.rs                    581                dto.rs                    581  the data both sides agree on
  wire.rs                   407                wire.rs                   407  JSON-RPC framing

crates/argos/src/              10 files      crates/argos/src/               9 files
  main.rs                   507                main.rs                   507  the commands clap accepts, and dispatch
  console.rs                881                console.rs                881  stdout: everything a person is shown, the
                                                                              live line a run redraws, and the keys that
                                                                              pause and stop it
  scan.rs                   856                scan.rs                   856  driving one scan without deciding how any
                                                                              of it is shown; the manifest it writes
  medium.rs                 392                medium.rs                 660  the medium a run touches: opening it
  acquire.rs                268  →medium.rs                                   read-only, refusing an output that would
                                                                              sit on it, and copying it to an image
                                                                              before anything else reads it
  results.rs                431                results.rs                431  reading a finished session back
  serve.rs                  721                serve.rs                  721  the JSON-RPC dispatch loop and the one
                                                                              thing allowed to write to stdout
  serve/pace.rs             357                serve/pace.rs             357  event pacing so a client is not drowned
  serve/trace.rs            220                serve/trace.rs            220  where a session spends its time (stderr)
  serve/translate.rs        288                serve/translate.rs        288  engine vocabulary → wire vocabulary
```

### 2.3 Support targets

```
BEFORE                                       AFTER
argos_core/tests/               2 files      argos_core/tests/               1 file
  geometry.rs                49                core.rs                    98  the newtypes and the in-memory source,
  fixture.rs                 49  →core.rs                                     the two things this crate has to prove

argos_device/tests/             3 files      argos_device/tests/             2 files
  sources.rs                 84                sources.rs                191  every BlockSource adapter behaves
  reader.rs                 107  →sources.rs                                  identically through the port, and
                                                                              BlockReader yields the medium's bytes at
                                                                              every alignment
  acquire.rs                217                acquire.rs                217  unchanged (phase exit criterion)

argos_classify/tests/           5 files      argos_classify/tests/           4 files
  rules.rs                  158                rules.rs                  366  what the rules may claim, where each
  eval.rs                   208  →rules.rs                                    direction must stay silent, and the eval
                                                                              gate every change has to pass
  thresholds.rs             161                thresholds.rs             161  unchanged — CI runs it by name with
                                                                              --ignored (ci.yml:142); merging it would
                                                                              hide an ignored harness inside an ordinary
                                                                              binary
  phash.rs / rank.rs                           phash.rs / rank.rs             unchanged: dedup is about pixels, standing
                                                                              is about manifest evidence

argos_engine/tests/             5 files      argos_engine/tests/             4 files
  pipeline.rs              1628                pipeline.rs              1724  end-to-end scans over synthetic media,
  merge_scale.rs             96  →pipeline.rs                                 including the shape of medium that makes
                                                                              merging expensive
  corpus_recall / graft / triage               unchanged

argos/tests/ 4 · argos_fs/tests/ 1 · argos_report/tests/ 1     unchanged
benches/ 3 · examples/ 4 · fuzz/ 17 targets                    unchanged
```

All 17 fuzz targets stay: `A-FUZZ-EVERY-PARSER` requires one per parser and `.github/workflows/ci.yml`
lines 114–127 run each by name. All four examples stay; each carries an `A-SUPPORT-DECLARED` line
naming what runs it. Nothing in `tests/`, `benches/`, `examples/` or `fuzz/` is folded into a
library — only the *count* of integration binaries falls, four times, each time by joining two
files that share a subject.

### 2.4 Directories

28 → 27, and the change is a shape change rather than an arithmetic one. `argos_device/src/device/`
and `argos_device/src/inventory/` are two directories of the same shape — `linux.rs`, `macos.rs`,
`windows.rs` in each — holding the same crate's syscalls for the same three operating systems. They
become one `argos_device/src/platform/` with three siblings, and the crate's top level then reads
exactly as CLAUDE.md describes it: *decisions* (`class.rs`, `naming.rs`, `mount.rs`) compiled on
every target, *syscalls* (`platform/`) behind `cfg`.

Surviving directories and the three-plus siblings each groups:

| Directory | Siblings | Subject |
| --- | --- | --- |
| `argos_device/src/platform/` | 3 | one file per operating system, holding that OS's syscalls and nothing else |
| `argos_engine/src/pipeline/` | 4 | one file per pipeline stage: C (filesystem), D (carving), E (reassembly), F/H (output) |
| `argos/src/serve/` | 3 | the engine process's wire surface: pacing, tracing, translation |

No `src/` directory holds one file, before or after. The remaining single-file directories
(`argos/examples`, `argos_fs/examples`, `argos_fs/tests`, `argos_report/tests`,
`argos_classify/benches`, and after the merge `argos_core/tests`) are Cargo-mandated target
directories and cannot be dissolved.

There is **no `mod.rs` in the workspace**, before or after. Maximum path depth stays 4
(`crates/argos_device/src/platform/linux.rs`).

---

## 3. Capability inventory — the invariant contract

The contract already exists and is maintained: **`docs/CAPABILITIES.md`**, 51 numbered capabilities
in user language with where each is reachable from. That file, not this one, is the contract; every
proposal below was checked against it. Restating it here would create a second copy that drifts.

Where each capability group lives after the plan:

| Capabilities | Held after consolidation by |
| --- | --- |
| C01–C06 open a medium, list media, class/TRIM/mounts/shadow copies, warn | `argos_device/src/{device,inventory,class,naming,mount}.rs`, `argos_device/src/platform/*`, `argos/src/{medium,console}.rs` |
| C07–C08 acquire to image, zero-fill and list what stayed unreadable | `argos_device/src/acquire.rs`, `argos/src/medium.rs` |
| C09, C13, C15 carve, validate, partial prefix | `argos_carve/src/{lib,jpeg,png}.rs`, `argos_engine/src/pipeline/{carving,output}.rs` |
| C10–C12, C16 filesystem recovery, residue, partition tables, `$UsnJrnl` | `argos_fs/src/*`, `argos_engine/src/pipeline/filesystem.rs` |
| C14, C20, C22 reassembly, its budget, resuming from fragmentation points | `argos_carve/src/{reassemble,mcu,classify,decode}.rs`, `argos_engine/src/pipeline/reassembly.rs` |
| C51 graft | `argos_carve/src/reference.rs`, `argos_engine/src/graft.rs`, `argos/src/scan.rs` |
| C17–C19, C21, C23, C29 stage switches, range, jobs, size floor | `argos_engine/src/config.rs`, `argos/src/main.rs` |
| C24–C28 triage, perceptual dedup, SHA dedup, standing | `argos_classify/src/{lib,rules,rank}.rs`, `argos_engine/src/annotate.rs` |
| C30–C33 manifest, recorded-not-written, previews, refuse output on source | `argos_report/src/{lib,manifest}.rs`, `argos/src/medium.rs` |
| C34–C41 report, gallery page, export and its five filters | `argos/src/{results,console}.rs`, `argos/src/serve/translate.rs` |
| C42–C47, C50 progress, pause/stop, damage total, warnings, serve | `argos_core/src/ports.rs`, `argos/src/{console,scan,serve}.rs`, `argos_ipc/src/*` |
| C48–C49 elevation, theme | `argos_ui` — out of scope, untouched |

**No proposal in this document removes, relocates out of reach, or narrows the visibility of
anything on that list.** The one `REMOVE` in §7 is a `usize` constant in a test-only fixture module
that nothing reads and that produces no bytes, so no capability line depends on it.

---

## 4. Structural census

### 4.1 Files with exactly one consumer (the `INLINE` list)

Produced by naming every non-root `src/` module and counting the distinct files that write its
module path. Eight results:

| File | Lines | Sole consumer | Disposition |
| --- | --- | --- | --- |
| `crates/argos_engine/src/pipeline/region.rs` | 430 | `pipeline/reassembly.rs` | `MERGE` — the two references in `argos_carve/benches/replay.rs:49,182` are prose in doc comments, not code |
| `crates/argos_report/src/manifest.rs` | 527 | `lib.rs:25` (`mod manifest;` + `pub use`) | `KEEP` — see §6.9 |
| `crates/argos_engine/src/pipeline/carving.rs` | 260 | `pipeline.rs:43` | `KEEP` — one file per stage; see §6.7 |
| `crates/argos_engine/src/pipeline/filesystem.rs` | 479 | `pipeline.rs:44` | `KEEP` — one file per stage |
| `crates/argos_engine/src/pipeline/output.rs` | 329 | `pipeline.rs:45` | `KEEP` — one file per stage |
| `crates/argos/src/console.rs` | 881 | `main.rs:12` | `KEEP` — the only place printing happens; 881 lines, one responsibility |
| `crates/argos/src/serve.rs` | 721 | `main.rs:16` | `KEEP` — the engine process; a command, like `console` is a mode |
| `crates/argos/src/serve/pace.rs` | 357 | `serve.rs:21` | `KEEP` — the rate limiter is a named, tested mechanism |

Next tier (two consumers), for completeness: `argos/src/medium.rs` (acquire.rs, scan.rs — the
`MERGE` in §6.10), `argos/src/serve/{trace,translate}.rs`, `argos_device/src/device.rs`,
`argos_engine/src/graft.rs`, `argos_device/src/inventory/mount.rs`.

### 4.2 Single-file directories

None under any `src/`. Six exist and all are Cargo target directories that cannot be dissolved:
`crates/argos/examples`, `crates/argos_fs/examples`, `crates/argos_fs/tests`,
`crates/argos_report/tests`, `crates/argos_classify/benches`, and — after §6.11 —
`crates/argos_core/tests`.

### 4.3 Re-export-only shells

`find crates -name mod.rs` returns **nothing**: the workspace has no `mod.rs` at all, so pattern 3
of the doctrine has no instances. `rg '^\s*pub use'` returns 16 lines in 7 files, and every one of
them is a deliberate crate-surface statement rather than a path level:

- `argos_engine/src/lib.rs:47–57` (6) — the crate surface plus two `M-FOREIGN-REEXPORTS`
  re-exports (`argos_carve::reassemble::Broken`, `argos_fs::{FsKind, Origin, Volume}`) so callers
  need no direct dependency on crates the DAG keeps below the engine. `KEEP`.
- `argos_device/src/lib.rs:16–22` (5) — the crate surface. `KEEP`.
- `argos_report/src/lib.rs:29` (1) — lifts all eleven manifest records to the root, which makes
  the module path `argos_report::manifest` invisible. See §6.9.
- `argos_carve/src/lib.rs:45`, `argos_fs/src/lib.rs:41`, `argos_engine/src/graft.rs:28` — shared
  vocabulary re-exports. `KEEP`.

### 4.4 Size histogram

| Band | All `.rs` | `src/` only |
| --- | --- | --- |
| 0–49 | 19 | 0 |
| 50–99 | 6 | 1 |
| 100–149 | 10 | 6 |
| 150–299 | 35 | 23 |
| 300–599 | 34 | 26 |
| 600–999 | 12 | 10 |
| 1,000–1,499 | 4 | 4 |
| 1,500+ | 4 | 2 |

The 0–49 band is 17 fuzz targets plus two 49-line `argos_core` integration tests. The `src/` files
under 150 lines are seven: `argos_device/src/lib.rs` (22), `argos_ipc/src/lib.rs` (88),
`argos_core/src/fixture.rs` (110), `argos_engine/src/lib.rs` (139), `argos_device/src/shadow.rs`
(142), `argos_device/src/inventory/{linux,macos}.rs` (108 each). Three are crate roots. The
remaining four are the whole of the small-file problem, and three of them are absorbed by §6.2.

### 4.5 Crate consumer counts

`argos_core` 7 · `argos_carve` 2 (`argos_engine` lib, `argos`/`argos_classify` dev-only) ·
`argos_fs` 1 · `argos_classify` 2 · `argos_engine` 1 · `argos_device` 1 · `argos_report` 1 ·
`argos_ipc` 2 (one of them `argos_ui`, outside the workspace) · `argos` 0 (binary).

Single-consumer crates are not merge candidates here: each is the mechanical enforcement of a DAG
edge. `argos_classify`'s dev-dependency edge to `argos_carve` is the documented threshold harness
and does not appear in `--edges normal`.

### 4.6 Probes run

| Probe | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | clean, 1.71 s incremental |
| `cargo clippy --workspace --all-targets -- -W clippy::nursery` | no `-D warnings` failure; 105 `missing_const_for_fn`, 21 `derive_partial_eq_without_eq`, **17 `redundant_pub_crate`** (the only structural signal), 6 `significant_drop_tightening`, 5 `redundant_clone`, 5 `use_self` |
| `cargo machete` | clean for all nine workspace crates (one hit in `argos_ui`, out of scope) |
| `cargo +nightly udeps --workspace --all-targets` | *"All deps seem to have been used."* |
| `cargo tree --workspace --edges normal` | DAG holds; no forbidden edge |
| `rg '^\s*pub use' --stats` + `find -name mod.rs` | 16 re-exports, 0 `mod.rs` — see §4.3 |
| `find -type d` with per-directory `.rs` counts | 28 directories hold Rust; no `src/` directory holds one file |
| `tokei` + size histogram | §4.4 |
| Visibility narrowing (throwaway copy outside the tree) | run; see below |
| `cargo modules structure` | available but not needed — the module tree is 1:1 with the file tree, there is no inline `mod` block anywhere |

**Visibility narrowing, as run.** A copy of the workspace was made under the scratchpad, outside the
working tree. For each library crate in turn, every line-initial `pub ` on an item was rewritten to
`pub(crate) ` in that crate only, and the workspace was recompiled. The E0603/E0624 errors are then
exactly the externally reached surface. For `argos_device` the answer is: modules `acquire`,
`inventory`, `naming`, `class`, `shadow`, and types `Device`, `BlockReader`, `ImageSource`,
`TrimState` — nothing else. The probe is module-granular, because a private module masks the items
inside it, so item-level narrowing needs a second pass per module; that pass is specified in §10
Wave 5 rather than reported here, and the item candidates below come from a name census that was
verified by hand.

**Item census.** 102 `pub` items are never named outside their own crate's `src/`. Filtering the
false positives — types that are only ever *returned* and bound by inference (`Attempt`,
`TiffThumbnail`, `Checkpoint`, `Anchored`, `Live`, `JournalDeletion`, `UsnDeletion`, `NameGhost`,
`Features`, `GeometryError`, `UnknownFormat`, `AcquireError`, `ReportError`), the whole of
`argos_ipc` (its surface is the wire contract and `argos_ui` consumes it), and the `test-util`
fixture surfaces that exist for other crates' tests — leaves **35 genuine `NARROW` candidates**,
listed in §6.12. Exactly one item is referenced nowhere at all; it is §7.

---

## 5. Metrics

| field | before | after (projected) |
| --- | --- | --- |
| crates | 9 | 9 |
| directories (holding `.rs`) | 28 | 27 |
| `.rs` files | 124 | 109 |
| — of which `src/` | 72 | 61 |
| — of which `tests/` | 28 | 24 |
| max path depth | 4 | 4 |
| median file size (lines, all) | 262 | 328 |
| median file size (lines, `src/`) | 338 | 431 |
| files under 150 lines | 35 | 28 |
| — of which `src/` | 7 | 4 |
| single-file directories (`src/`) | 0 | 0 |
| re-export-only `mod.rs` | 0 | 0 |
| files with exactly one consumer | 8 | 7 |
| `pub` items with no external consumer | 102 raw / 35 genuine | 67 raw / 0 genuine |
| traits with 1 impl outside a port | 0 | 0 |
| `#[allow(dead_code)]` | 0 | 0 |
| `#[allow(...)]` of any kind | 0 | 0 |
| `#[expect(...)]` with a reason | 32 | 32 |
| pure forwarding wrappers | 0 | 0 |
| error types (canonical structs; public error enums: 0) | 11 | 11 |
| `clippy::redundant_pub_crate` | 17 | 0 |
| **total lines** | **45,309** | **45,309** |

Total lines are identical by construction: every Regime A item is a move. The single Regime B
removal takes 4 lines (a constant and its three doc lines) and is counted separately so the
consolidation figure stays honest — if a wave lands and the total drops, code was lost in a move
and that is a defect.

---

## 6. Consolidation plan (Regime A)

Grouped by destination file. Every item is a move plus a visibility change; no behaviour changes,
and the compiler proves each move complete because a missed reference does not compile.

**Preserve history.** Each item is `git mv` where a file becomes the destination or moves
unchanged, and `git mv` + edit where a file is absorbed — never delete-and-recreate. Concretely:
`git mv crates/argos_device/src/device/linux.rs crates/argos_device/src/platform/linux.rs`, then
append `inventory/linux.rs`'s body and `git rm` it; `git mv crates/argos_core/src/source.rs
crates/argos_core/src/ports.rs`, then append the other three.

### 6.1 → `crates/argos_core/src/lib.rs` and `ports.rs`

- `MERGE` `geometry.rs` (265) into `lib.rs` (202). **Result's responsibility (467 lines):** the
  domain vocabulary — how a recovery is described (`Stage`, `Format`, `Confidence`, `Timestamps`)
  and where on a medium it sits (`Lba`, `ByteOffset`, `SectorSize`, `SectorRange`, `ByteRange`,
  `GeometryError`). **Why:** `geometry` is nameable but not large enough that merging breaks its
  host, and the root already holds the other half of the same vocabulary. `M-BALANCED-MODULES`
  wants the essential items in the root. Paths shorten: `argos_core::geometry::ByteRange` →
  `argos_core::ByteRange`, matching `argos_core::Confidence` today. 62 files touch `geometry::`;
  every edit is mechanical and compiler-checked.
- `MERGE` `source.rs` (207), `artifact.rs` (263), `classify.rs` (217) and `progress.rs` (209) into
  one `ports.rs` (896). **Result's responsibility:** the four ports of the hexagon and the
  vocabulary each carries. **Why:** the crate doc at `lib.rs:6–11` already presents them as one
  table with one column for direction and one for adapters; each file is 207–265 lines, well under
  the size at which merging would make the host incoherent; and every one of them is *the same
  kind of thing*, which is what a module should group.
  **Judgement call, stated as one.** The counter-argument is real: `argos_core::progress::ProgressSink`
  reads better than `argos_core::ports::ProgressSink`, and four ports in one file is four sections
  a reader scrolls past. If the author prefers four files, that is a defensible tree and the rest
  of this plan is unaffected — it costs 3 files and nothing else. This item is sequenced last for
  exactly that reason.
- `KEEP` `fixture.rs` (110) as a file despite being under 150 lines. Five crates carry a
  `<crate>::fixture` module and a reader looks for it by that name; making `argos_core` the one
  exception buys one file and costs a workspace convention.

### 6.2 → `crates/argos_device/src/platform/`

- `MERGE` `device/linux.rs` (260) + `inventory/linux.rs` (108) → `platform/linux.rs` (368);
  `device/macos.rs` (198) + `inventory/macos.rs` (108) → `platform/macos.rs` (306);
  `device/windows.rs` (358) + `inventory/windows.rs` (305) → `platform/windows.rs` (663).
  **Result's responsibility, one per file:** everything this crate asks *that* operating system,
  and nothing else. **Why:** two directories of identical shape holding the same crate's syscalls
  for the same three platforms is the mirrored-parallel-structure pattern. The merge is clean —
  neither family references the other; both depend only on `crate::class` and `crate::naming` plus
  their parent's types (`device/linux.rs:15`, `inventory/linux.rs:11–14`,
  `device/windows.rs:33–34`, `inventory/windows.rs:27–28`). The result is also the better review
  surface for `M-UNSAFE`: one file per OS to audit instead of two.
  **Follow-on edits:** `device.rs:29–34` and `inventory.rs:31–37` lose their `mod` blocks;
  `lib.rs` gains `mod platform;`; `use super::DeviceError` becomes `use crate::device::DeviceError`
  and `use super::{DeviceInfo, MountPoint, mount}` becomes `use crate::inventory::{DeviceInfo,
  MountPoint}; use crate::mount;`.
- `DISSOLVE` the directories `crates/argos_device/src/device/` and
  `crates/argos_device/src/inventory/`.

### 6.3 → `crates/argos_device/src/inventory.rs`

- `MERGE` `shadow.rs` (142) into `inventory.rs` (206). **Result's responsibility (348 lines):**
  what media this machine offers — disks, partitions, their mounts, and the shadow copies a
  platform keeps. **Why:** 142 lines is a fragment; `argos/src/console.rs:129,160` and
  `argos/src/serve/translate.rs` already consume the two together as one answer to "what can I
  scan"; and `ShadowCopy` is a medium in an inventory. **This is the one item the 2026-08-23 audit
  proposed (§6.3 there) that was not executed** — it is the clearest residue of that pass.
  `argos_device::shadow::shadow_copy_index` is named by `argos/src/medium.rs:257`, so the path
  becomes `argos_device::inventory::shadow_copy_index` (or stays reachable through the existing
  `pub use shadow::ShadowCopy` line, rewritten).

### 6.4 → `crates/argos_device/src/mount.rs`

- `MERGE` — strictly a move: `git mv crates/argos_device/src/inventory/mount.rs
  crates/argos_device/src/mount.rs`. **Why:** `mount.rs` is parsing, compiled and tested on every
  target by design (`M-MOCKABLE-SYSCALLS`), exactly like `class.rs` and `naming.rs` which already
  sit at the crate's top level. Leaving it inside a directory whose other three files are `cfg`'d
  syscalls says the opposite of what it is. The `inventory/` directory disappears in §6.2 anyway,
  so this move is forced; what §6.4 decides is only that it goes *up* rather than into
  `inventory.rs`. Folding its 248 lines into `inventory.rs` (giving 596) is the alternative and
  would cost one more file — it is not proposed, because "how each OS states its mount table" is a
  responsibility a reader looks for by name.

### 6.5 → `crates/argos_carve/src/jpeg.rs`

- `MERGE` `exif.rs` (270) into `jpeg.rs` (265). **Result's responsibility (535 lines):** the JPEG
  marker state machine and the TIFF/IFD payload it mines out of `APP1`. **Why:** EXIF is not a
  format Argos recovers; it is a segment inside one, and `exif.rs` exists only because `jpeg.rs`
  and `lib.rs` walk into it (`jpeg.rs` for the thumbnail, `lib.rs:232` for capture metadata).
  Neither file is large enough that merging breaks the host.
  **Cost, stated:** the public path becomes `argos_carve::jpeg::exif::{thumbnail, metadata,
  TiffThumbnail}`, which means editing `crates/argos_carve/fuzz/fuzz_targets/exif_metadata.rs`,
  `exif_thumbnail.rs` and `crates/argos_carve/tests/carve.rs`. The fuzz crate is outside the
  workspace, so `cargo +nightly fuzz build --fuzz-dir crates/argos_carve/fuzz` must run in the same
  wave — `cargo check --workspace` will not catch it.
- Not proposed: merging `jpeg.rs` and `png.rs` into one `validate.rs`. One file per on-disk format
  is the shape `argos_fs` uses for its six filesystems and `A-FUZZ-EVERY-PARSER` assumes; keeping
  it costs one file and buys a convention that holds across two crates.

### 6.6 → `crates/argos_engine/src/pipeline/reassembly.rs`

- `MERGE` `region.rs` (430) into `reassembly.rs` (641). **Result's responsibility (1,071 lines):**
  stage E — the region of the medium a search is allowed to reach, held in memory, and the search
  that runs over it. **Why:** sole consumer (§4.1); `region.rs` is not a stage but stage E's
  infrastructure, so it is the one file in `pipeline/` that breaks the one-file-per-stage rule.
  `REGION_BYTES`, `Plan`, `plan_regions`, `Region` and `RegionView` are all `pub(crate)` and drop
  to private on the way in, clearing 5 of the 17 `redundant_pub_crate` hits.

### 6.7 `crates/argos_engine/src/pipeline/` — considered and refused

`carving.rs` (260), `filesystem.rs` (479) and `output.rs` (329) each have exactly one consumer and
each opens with `use super::*;`, which normally marks a file cut out of its parent for size rather
than for responsibility. They are **`KEEP`** anyway, and the reason is the directory: after §6.6 it
holds one file per pipeline stage — C, D, E, F/H — and the parent holds the stage order and the one
sequential pass. Merging `carving.rs` back into `pipeline.rs` would put stage D in the parent while
C, E and F/H stayed children, which is worse than either shape. This is where the 2026-08-23 pass
slightly overshot in the other direction: it designed four children and five were built. Four is
right, and §6.6 gets there by moving `region.rs` rather than by unpicking a stage.

The `use super::*;` glob is a separate, smaller finding: it is what makes each child unreadable
alone. Replacing it with explicit `use super::{...}` lists is a one-file-at-a-time change with no
structural effect, and is listed in §10 Wave 6 as optional polish.

### 6.8 → `crates/argos/src/medium.rs`

- `MERGE` `acquire.rs` (268, of which 124 are `#[cfg(test)]`) into `medium.rs` (392).
  **Result's responsibility (660 lines):** the medium a run touches — opening it read-only,
  refusing an output that would sit on it, and copying it to an image before anything else reads
  it. **Why:** `acquire.rs` already reaches into its destination twice
  (`acquire.rs:41` → `medium::refuse_writing_onto_source`, `acquire.rs:97` →
  `medium::is_device_node`), and `is_device_node` is `pub(crate)` at `medium.rs:230` for no reason
  other than that sibling — a `pub(crate)` that exists for exactly one caller is the signature of a
  file that should not be separate. The `Notice` trait and its test `Collected` move with it.
- `KEEP` the two same-named `Notice` traits (`scan.rs:89`, `acquire.rs:137`, the latter arriving in
  `medium.rs`). Each has two real implementations plus a test double — `console::Console` and
  `serve::Notices` for the scan one, `console::Console` and `serve::Acquisition` for the acquire
  one — and their existence is what keeps `println!` out of the serve path where stdout is the
  protocol. They are not ports of the hexagon but they are the same kind of seam, and neither is
  speculative. After the merge the two live in different modules with the same name, which is a
  readability hazard worth a rename (`medium::AcquireNotice`) — noted, not required.

### 6.9 `crates/argos_report/src/manifest.rs` — considered and refused

`manifest.rs` (527) is a private module (`lib.rs:25`) whose entire surface is lifted to the crate
root by `lib.rs:29`, so its module path is invisible and it has exactly one consumer. By pattern 1
it is an `INLINE`. It is **`KEEP`** for two reasons: the merged host would be 1,406 lines carrying
four things (the `Store` sink, the preview encoder, the ownership handback, and the schema), which
is where a file starts to lose a single nameable responsibility; and `b556d2b` created this split
four days ago on the stated ground that the manifest is a *format* four separate readers parse
(`argos report`, `argos export`, the serve gallery, and the characterization snapshot) while the
`Store` is a *writer*. Reversing that within the week is churn, not consolidation.

What should change is the visibility, not the file: `mod manifest;` → `pub mod manifest;`, so the
path carries the meaning the file already has, and the eleven `pub use` names at `lib.rs:29–32`
become optional rather than load-bearing. That is `NARROW`'s inverse and is listed in §6.12.

### 6.10 `crates/argos/src/serve/` — considered and refused

Three siblings — `pace.rs` (357), `trace.rs` (220), `translate.rs` (288) — sharing one subject: the
engine process's wire surface. The directory earns its place at exactly three. `trace.rs` has two
consumers (`serve.rs`, `pace.rs`), so it is not a fragment of either; folding `pace.rs` and
`trace.rs` into `serve.rs` would make a 1,298-line file whose responsibility takes a compound
sentence to state, and folding only `trace.rs` into `pace.rs` would leave a two-file directory,
which the doctrine treats as one file that has not noticed. **`KEEP` all three.**

### 6.11 Support targets

- `MERGE` `argos_core/tests/{geometry.rs, fixture.rs}` (49 + 49) → `core.rs` (98). Two 49-line
  integration binaries over one crate; one subject — the newtypes and the in-memory source.
- `MERGE` `argos_device/tests/reader.rs` (107) into `sources.rs` (84) → 191. `sources.rs` asserts
  the adapters behave identically *through the port*; `reader.rs` asserts `BlockReader` yields the
  medium's bytes at every alignment. Same subject: the adapter layer seen from outside.
- `MERGE` `argos_classify/tests/eval.rs` (208) into `rules.rs` (158) → 366. The eval harness is the
  gate on the rules; both are ordinary (non-`ignored`) tests of the same subject.
  **Do not** merge `thresholds.rs`: `.github/workflows/ci.yml:142` runs it by name with
  `-- --ignored`, and hiding an ignored harness inside a binary CI also runs normally is a way to
  lose it silently.
- `MERGE` `argos_engine/tests/merge_scale.rs` (96) into `pipeline.rs` (1,628) → 1,724. Merging at
  the scale a real medium produces is a property of the pipeline, and 96 lines is not a binary.
- Not proposed: merging the two `argos_carve/examples/` generators, the 17 fuzz targets, or any
  `tests/` file into a library. Each fuzz target is named individually in `ci.yml:114–127` and each
  example carries an `A-SUPPORT-DECLARED` line naming what runs it.

### 6.12 `NARROW` — visibility

35 items are `pub` with no consumer outside their own crate's `src/`. Each is `pub(crate)` or
private. Grouped, with anchors:

- `argos_core`: `SectorSize::MIN_BYTES`, `MAX_BYTES` (`geometry.rs:94,97`) — used only by
  `SectorSize::from_u32`/`new` in the same file. *(Also proposed by the 2026-08-23 audit §6.1 and
  not executed.)*
- `argos_fs`: `residue::MAX_VOLUMES`, `MAX_RECORD_REGIONS` (`residue.rs:44,48`) — read only by
  `scan_window` at `residue.rs:177–179`; `apfs::fletcher64` (`apfs.rs:521`) — its doc says
  "exposed for fixture builders", but `crates/argos_fs/src/fixture.rs` is in the same crate, so
  `pub(crate)` is the correct level. *(All three carried over unexecuted from 2026-08-23 §6.4.)*
- `argos_fs::fixture` (test-util, `pub` is correct at the module level, wrong per item): 20
  builders and constants used only by other builders in the same file — `gpt_header:220`,
  `NTFS_RECORD:242`, `EXT4_INODES_PER_GROUP:741`, `EXT4_INODE_BYTES:744`, `ext4_root_inode:932`,
  `ext4_extent_leaf:1022`, `jbd2_descriptor:1047`, `fat32_boot_sector:1171`, `fat_dir_deleted:1189`,
  `apfs_nxsb:1359`, `apfs_omap:1382`, `apfs_omap_root:1391`, `apfs_volume:1401`, `FsRecord:1411`,
  `apfs_fs_tree:1426`, `btrfs_chunk_item:1813`, `btrfs_chunk_tree:1838`, `btrfs_root_tree:1853`,
  `BtrfsExtent:1863`, `BtrfsFile:1867`, `btrfs_fs_tree:1880`, `btrfs_fs_tree_compressed:1886`,
  `btrfs_extent_item:1925`. All verified by `rg` across every crate including `fuzz/` and
  `examples/`.
- `argos_carve`: `classify::MIN_BLOCK_BYTES` (`classify.rs:26`), `reassemble::MAX_PREFIX_CANDIDATES`
  (`reassemble.rs:94`), `reference::MAX_HEADER_BYTES` (`reference.rs:40`) — each read only inside
  its own file; `fixture::next_byte` (`fixture.rs:391`).
- `argos_classify`: `rank::PHOTOGRAPH_MIN_LONG_SIDE` (`rank.rs:38`), `rules::Features`
  (`rules.rs:150`) — `Features` is returned by `features()` and consumed by `screen()`, both in
  `rules.rs`, and by nothing else; `fixture::{next_u64:28, jitter:47}`.
- `argos_device`: `shadow::{shadow_copy_path:41, MAX_SHADOW_COPY_INDEX:65}`,
  `inventory::mount::{parse_linux_mountinfo:28, mounts_of:107}`,
  `class::{from_rotational:21, from_seek_penalty:35, from_solid_state:48}`,
  `naming::{windows_physical_drive:35, macos_raw_path:160}`. *(Carried over unexecuted from
  2026-08-23 §6.3.)*
- `argos_engine`: `Finding::{is_covered_by:88, intersects:102}` (`finding.rs`) — used only by
  `consolidate` and `head_before_damage` in the same file.

The inverse, once: `argos_report/src/lib.rs:25` `mod manifest;` → `pub mod manifest;` (§6.9).

Clearing `clippy::redundant_pub_crate` (17 hits) is the same wave: `argos_engine/src/pipeline.rs:135,250,343`,
`pipeline/region.rs:33,43,57,114,198`, `pipeline/output.rs:21,28`, `annotate.rs:78,95,117`,
`session.rs:284`, `argos_report/src/manifest.rs:19`, `argos/src/medium.rs:230`,
`argos_device/src/device/linux.rs:31`. Five of them disappear with §6.6 and one with §6.8.

### 6.13 Not found — declared absences

Silence is not an answer, so: **no** `Manager`/`Service`/`Handler`/`Helper`/`Util`/`Provider`/
`Factory`/`Coordinator`/`Processor` type exists (`rg` over every crate returns only
`ScanConfigBuilder`, which validates and returns `Result` per `M-BUILD-RESULT` over eight fields).
**No** `utils`/`helpers`/`common`/`misc`/`shared` module. **No** `#[allow]` anywhere — all 32 lint
overrides are `#[expect(..., reason = "...")]` per `M-LINT-OVERRIDE-EXPECT`. **No** pure forwarding
wrapper: `argos_carve::validate` dispatches on `Format`, `validate_thumbnail` bounds the limit
before delegating, `argos_device::inventory::list` selects a platform. **No** conversion layer
between near-identical types: `argos/src/serve/translate.rs` is a hand-written boundary that exists
precisely so `argos_ipc` can depend on nothing, which is a structural guarantee rather than
ceremony. **No** unused dependency (`udeps` and `machete` both clean). **No** dead struct field and
**no** dead enum variant (both swept by construction-site count). **No** broken doc link into
`docs/`. **Two** `Box<dyn ...>`, both in error types where the source is genuinely open
(`argos_engine/src/lib.rs:82`). **Three** `Arc<Mutex<...>>`, all genuinely crossing a
`std::thread::spawn` boundary (`argos/src/serve.rs:84,92` with spawns at `serve.rs:270,440`;
`argos_device/src/device.rs:280` is the `test-util` mock's shared state). **Seven**
`Result<Option<T>>` sites in `argos_carve`, all distinguishing "I/O failed" from "not there or not
valid", which is the distinction the whole crate is built on. **Four** `impl AsRef<Path>` in
internal APIs (`argos_device/src/device.rs:83,343`, `argos_report/src/{lib.rs:75,manifest.rs:292,309}`)
— low-priority, listed for completeness, no file depends on them.

---

## 7. Removal list (Regime B)

One item. Built for item-by-item human review; nothing else in this document deletes a line.

### 7.1 `BTRFS_MIRROR_AT`

**Location:** `crates/argos_fs/src/fixture.rs:1591` —
`pub const BTRFS_MIRROR_AT: usize = 0x400_0000;` plus its three doc lines (1589–1591).

**Evidence 1 — no reachable caller.** `rg -n '\bBTRFS_MIRROR_AT\b' crates` returns exactly one
line: the definition itself. No CLI path, no serve method, no test, no bench, no example, no fuzz
target names it. Its sibling constants are all reached — `BTRFS_SUPERBLOCK_AT` and `BTRFS_SECTOR`
are imported at `crates/argos_fs/tests/recovery.rs:12`, and `BTRFS_MIRROR_AT` is absent from that
import list.

**Evidence 2 — no test exercises it.** The btrfs suite in `crates/argos_fs/tests/recovery.rs:861`
sizes its fixture volume at `BTRFS_VOLUME = 1 << 20` (1 MiB). `0x400_0000` is 64 MiB, so no
fixture the suite builds is large enough to contain the offset this constant names; a test using it
could not currently exist. `crates/argos_fs/fuzz/fuzz_targets/btrfs_parse.rs` does not name it
either.

**Evidence 3 — no unique behaviour.** The value is the parser's own, and the parser has its own
copy that is actually used: `crates/argos_fs/src/btrfs.rs:43`
`const MIRROR_OFFSETS: [u64; 4] = [0x1_0000, 0x400_0000, 0x40_0000_0000, 0x4_0000_0000_0000];`,
read at `btrfs.rs:272` and `btrfs.rs:405`. Removing the fixture constant changes no byte any
builder emits and no path any parser walks.

**Disposition:** `REMOVE`. **Caveat the reviewer must settle:** the btrfs work is the newest code in
the tree (`f4d0392`, plus uncommitted changes to `fixture.rs`), and a mirror-offset constant in a
fixture module is exactly what one would add *first* when about to write a test that plants a
superblock at the first mirror and checks that recovery finds the volume when the primary is
destroyed. If that test is coming, the constant is scaffolding and should stay — but then the test
should land in the same wave. See Question 3.

---

## 8. Findings by module

Entries are grouped by crate; each is self-contained. Lines/public-items/external-callers/consumers
are as measured. "Reachable from" uses the CLI as the proof surface (`A-CLI-FIRST`) — CLI-only
reachability is full reachability here, never dead code.

### 8.1 `argos_core`

- **Path** `crates/argos_core/src/lib.rs` — **Responsibility:** the recovery vocabulary every crate
  names. **Layer:** core. **202 lines**, 5 public items + 5 module decls, consumed by all 8 other
  crates. **Destination:** absorbs `geometry.rs` → 467 lines. **If it vanished:** every capability
  line, C01–C51. **Findings:** `MERGE` §6.1. **Confidence:** high.
- **Path** `crates/argos_core/src/geometry.rs` — **Responsibility:** positions and sizes on a
  medium, sector- and byte-addressed, never mixed. **Layer:** core. **265 lines**, 6 public types,
  62 consuming files — the most-referenced module in the workspace. **Destination:** `lib.rs`.
  **If it vanished:** C01, C02, C09–C16, C30. **Findings:** `MERGE` §6.1; `NARROW`
  `SectorSize::{MIN_BYTES, MAX_BYTES}` (`geometry.rs:94,97`). **Confidence:** high.
- **Path** `crates/argos_core/src/source.rs` — **Responsibility:** the read-only port to a medium.
  **Layer:** port. **207 lines**, 4 public items, 13 consuming files. **Destination:** `ports.rs`.
  **If it vanished:** C01, C02, C07. **Findings:** `KEEP` the `BlockSource` trait itself — it is a
  driven port at the boundary, the dependency points inward, it has four implementations
  (`Device`, `ImageSource`, `Ctrl`'s mock, `MemDisk`), and its deliberate absence of a write method
  is how "never write to the source medium" is enforced structurally rather than by review.
  `MERGE` the file per §6.1. **Confidence:** high.
- **Path** `crates/argos_core/src/artifact.rs` — **Responsibility:** the port recovered artifacts
  leave the engine through. **Layer:** port. **263 lines**, 5 public items, 17 consuming files.
  **Destination:** `ports.rs`. **If it vanished:** C30–C33. **Findings:** `KEEP` `ArtifactSink` —
  port, two implementations (`argos_report::Store`, `engine::fixture::Collector`), and the reason
  `argos_engine` does not depend on `argos_report`. `MERGE` per §6.1. **Confidence:** high.
- **Path** `crates/argos_core/src/classify.rs` — **Responsibility:** the triage port.
  **Layer:** port. **217 lines**, 7 public items, consumed by `argos_classify`, `argos_engine`,
  `argos_report`, `argos`. **Destination:** `ports.rs`. **If it vanished:** C24, C25, C27.
  **Findings:** `KEEP` `Classifier` — port with two implementations (`argos_classify::Triage`,
  `AcceptAll`) and no filtering method, which is how `A-TRIAGE-NOT-VERDICT` is enforced by type
  rather than by rule. `MERGE` per §6.1. **Confidence:** high.
- **Path** `crates/argos_core/src/progress.rs` — **Responsibility:** the port a running scan reports
  through. **Layer:** port. **209 lines**, 5 public items, 16 consuming files. **Destination:**
  `ports.rs`. **If it vanished:** C42–C46. **Findings:** `KEEP` `ProgressSink` — port, four
  implementations (`console::Renderer`, `serve::Events`, `scan::Tee`, `Discard`, plus
  `fixture::Events`). `MERGE` per §6.1. **Confidence:** high.
- **Path** `crates/argos_core/src/fixture.rs` — **Responsibility:** an in-memory `BlockSource` with
  injectable bad sectors. **Layer:** shared test utility. **110 lines**, 1 public type, consumed by
  `argos_device`, `argos_engine`, `argos_core`'s own tests. **Destination:** survives standalone.
  **If it vanished:** no capability, but every damage-handling test loses its fixture.
  **Findings:** `KEEP` — see §6.1. **Confidence:** high.

### 8.2 `argos_device`

- **Path** `crates/argos_device/src/lib.rs` — 22 lines, 5 `pub use`, the crate surface. It is the
  thinnest root in the workspace and the only library crate whose canonical error
  (`DeviceError`, `device.rs:186`) does *not* live in its root, unlike `argos_fs::FsError`,
  `argos_carve::CarveError`, `argos_engine::ScanError` and `argos_report::ReportError`. **Finding:**
  consistency only, no disposition — moving it would touch nothing but taste. **Confidence:** high.
- **Path** `crates/argos_device/src/device.rs` — **Responsibility:** every medium this crate opens
  behind `BlockSource`, and the byte-addressed view over any of them. **Layer:** adapter.
  **560 lines**, 5 public types, consumed by `argos` (`medium.rs:19`, `acquire.rs:21`) and
  `orphan_census.rs:35`. **Destination:** survives; loses its `device/` submodule declarations to
  `platform/`. **If it vanished:** C01, C02, C07. **Findings:** `KEEP`. **Confidence:** high.
- **Path** `crates/argos_device/src/inventory.rs` — **Responsibility:** what disks this machine has
  and whether any is in use. **Layer:** adapter. **206 lines**. **Destination:** absorbs
  `shadow.rs` → 348. **If it vanished:** C03, C04, C05, C06. **Findings:** `MERGE` §6.3.
  **Confidence:** high.
- **Path** `crates/argos_device/src/shadow.rs` — **Responsibility:** Volume Shadow Copies as
  additional read-only sources. **Layer:** adapter. **142 lines**, 5 public items; consumers
  `console.rs:160`, `medium.rs:257`, `serve/translate.rs`. **Destination:** `inventory.rs`.
  **If it vanished:** C05 — the only capability line that names shadow copies. **Findings:**
  `MERGE` §6.3; `NARROW` `shadow_copy_path:41`, `MAX_SHADOW_COPY_INDEX:65`. **Confidence:** high.
- **Path** `crates/argos_device/src/inventory/mount.rs` — **Responsibility:** reading the operating
  system's mount table, on every target. **Layer:** adapter (pure parsing). **248 lines**,
  2 public fns, consumers `inventory/linux.rs`, `inventory/macos.rs`. **Destination:**
  `src/mount.rs`. **If it vanished:** C06. **Findings:** `MERGE`-as-move §6.4; `NARROW`
  `parse_linux_mountinfo:28`, `mounts_of:107`. **Confidence:** high.
- **Path** `crates/argos_device/src/class.rs` — **Responsibility:** what each platform's answer
  about a medium means for recovery. **Layer:** adapter decision, compiled everywhere.
  **212 lines**, 5 public fns + `TrimState`, 10 consuming files. **Destination:** survives.
  **If it vanished:** C04, and the honesty rule that a missing answer is `Unknown` rather than a
  guess. **Findings:** `KEEP`; `NARROW` `from_rotational:21`, `from_seek_penalty:35`,
  `from_solid_state:48` (each called only from one platform module inside this crate).
  **Confidence:** high.
- **Path** `crates/argos_device/src/naming.rs` — **Responsibility:** what an operating system's
  device paths mean. **Layer:** adapter decision, compiled everywhere. **390 lines**, 8 public fns,
  10 consuming files including `argos/src/medium.rs` and `serve/translate.rs`. **Destination:**
  survives. **If it vanished:** C03, C06. **Findings:** `KEEP`; `NARROW`
  `windows_physical_drive:35`, `macos_raw_path:160`. **Confidence:** high.
- **Path** `crates/argos_device/src/acquire.rs` — **Responsibility:** ddrescue-style multi-pass
  acquisition of a `BlockSource` into a raw image. **Layer:** adapter. **463 lines**, 6 public
  items, consumers `argos/src/acquire.rs`, `console.rs`, `serve.rs`, `tests/acquire.rs`.
  **Destination:** survives. **If it vanished:** C07, C08, C50. **Findings:** `KEEP`.
  **Confidence:** high.
- **Path** `crates/argos_device/src/device/{linux,macos,windows}.rs` and
  `src/inventory/{linux,macos,windows}.rs` — **Responsibility:** *two sentences each*, which is the
  finding: "this OS's open and geometry syscalls" and "this OS's enumeration syscalls".
  **Layer:** infrastructure. 260/198/358 and 108/108/305 lines. Each depends only on
  `crate::class`, `crate::naming` and its own parent's types; the two families never reference each
  other. **Destination:** `platform/{linux,macos,windows}.rs`, one sentence each: "everything this
  crate asks Linux". **If they vanished:** C01, C03, C04. **Findings:** `MERGE` + `DISSOLVE` §6.2;
  `NARROW` `device/linux.rs:31` (`redundant_pub_crate`). **Confidence:** high for Linux and macOS
  (read in full); **medium for Windows**, which was read structurally only — see §11.

### 8.3 `argos_fs`

- **Path** `crates/argos_fs/src/lib.rs` — **Responsibility:** the crate's shared vocabulary and the
  checked reads all six parsers are built on. **Layer:** core-adjacent (sans-IO parsing).
  **300 lines**. **Destination:** survives unchanged. **If it vanished:** C10–C12, C16.
  **Findings:** `KEEP`. **Confidence:** high.
- **Path** `ntfs.rs` (1,386) / `btrfs.rs` (1,084) / `ext4.rs` (761) / `fat.rs` (624) /
  `apfs.rs` (543) — **Responsibility:** one on-disk format each. **Layer:** adapter (parsing).
  Each has a matching fuzz target and a matching fixture builder. **Destination:** survive
  unchanged; this is the shape the tree would be designed with from scratch. **If any vanished:**
  C10 loses a named filesystem. **Findings:** `KEEP` all five; `NARROW` `apfs::fletcher64:521`.
  **Confidence:** high for `ntfs`/`ext4`/`fat`/`apfs`; **medium for `btrfs`**, which is the newest
  code and carries uncommitted changes.
- **Path** `part.rs` (284) / `residue.rs` (343) — **Responsibility:** current partition tables; and
  anchors of filesystems an earlier format left behind. **Destination:** survive. **If they
  vanished:** C11, C12 — and C11 is the capability that makes a re-formatted disk recoverable at
  all. **Findings:** `KEEP`; `NARROW` `residue::{MAX_VOLUMES:44, MAX_RECORD_REGIONS:48}`.
  **Confidence:** high.
- **Path** `crates/argos_fs/src/fixture.rs` — **Responsibility:** synthetic filesystem-image
  builders. **Layer:** shared test utility. **2,012 lines** — the largest file in the workspace —
  with 60+ public builders, consumed by `argos_fs/tests/recovery.rs`, 7 fuzz targets,
  `argos_engine/tests/*` and `argos/examples/orphan_census.rs`. **Destination:** survives.
  **Findings:** `KEEP`, at the limit and flagged. It is *one* responsibility (build a structurally
  valid volume with a known deleted file) but it is the union of six formats, and it is the one
  file in the tree where the counterweight argument could be made. The `SPLIT` that would be
  correct is not a split at all — it is moving each format's builders into
  `#[cfg(feature = "test-util")] pub mod fixture { ... }` inside the matching parser file, which
  removes the file entirely and puts each builder beside the parser it feeds, as
  `argos-ondisk-parsing` asks. That changes ~200 call sites from `argos_fs::fixture::ntfs_volume`
  to `argos_fs::ntfs::fixture::ntfs_volume` across three crates and two out-of-workspace fuzz
  crates. **`INVESTIGATE`** — see Question 2. `NARROW` the 20+ internal-only builders listed in
  §6.12. **Confidence:** medium.

### 8.4 `argos_carve`

- **Path** `crates/argos_carve/src/lib.rs` — **Responsibility:** the carving surface and the
  bounded cursor its validators read the medium through. **Layer:** core technique (sans-IO).
  **704 lines**. **Destination:** survives. **If it vanished:** C09, C13, C15. **Findings:**
  `KEEP`. One observation without a disposition: `metadata` (`lib.rs:209–242`) walks JPEG marker
  segments in the crate root while `jpeg.rs` exists. It is a public entry point and moving its body
  would leave a forwarding wrapper, which is worse; noted so a later reader does not mistake it for
  an oversight. **Confidence:** high.
- **Path** `jpeg.rs` (265) / `exif.rs` (270) — **Responsibility:** the JPEG state machine; and the
  TIFF/IFD walk for EXIF thumbnails and capture metadata. **Layer:** adapter (parsing).
  `exif.rs` consumers: `jpeg.rs`, `lib.rs`, two fuzz targets, `tests/carve.rs`. **Destination:**
  one `jpeg.rs` (535). **If they vanished:** C09, C13, C15, and the thumbnail half of C15.
  **Findings:** `MERGE` §6.5. **Confidence:** high.
- **Path** `png.rs` (405) — **Responsibility:** the PNG chunk walk with per-chunk CRC.
  **Destination:** survives. **If it vanished:** C09, C13 for PNG. **Findings:** `KEEP` — one file
  per format, matching `argos_fs`. **Confidence:** high.
- **Path** `classify.rs` (318) / `decode.rs` (332) / `mcu.rs` (1,456) / `reassemble.rs` (1,613) /
  `reference.rs` (235) — **Responsibility:** block classification; pixel decode as the oracle;
  entropy-stream decode; the search; a surviving header lent to orphans. **Destination:** all
  survive. **If any vanished:** C14, C20, C22, C51. **Findings:** `KEEP` all five; `NARROW`
  `classify::MIN_BLOCK_BYTES:26`, `reassemble::MAX_PREFIX_CANDIDATES:94`,
  `reference::MAX_HEADER_BYTES:40`. **Confidence:** high for `classify`/`decode`/`reference`;
  **medium for `mcu`/`reassemble`**, the two files whose internals I read structurally rather than
  line by line.
- **Path** `fixture.rs` (1,135) — genuinely decodable image and disk builders. **`KEEP`**; `NARROW`
  `next_byte:391`. **Confidence:** high.

### 8.5 `argos_classify`

- **Path** `lib.rs` (198) — **Responsibility:** what triage does to one image: label it, and hash it
  so near-duplicates collapse. **Layer:** adapter (implements the `Classifier` port).
  **Destination:** survives. **If it vanished:** C24, C25. **Findings:** `KEEP`. `Triage` is a
  field-less struct with a `new()` and a static `decide`, which is the shape the Java-ism catalogue
  flags — and it is correct here: it exists to *be* the `Classifier` implementation the engine is
  injected with, so the type is the port's inhabitant rather than a costume on a function.
  **Confidence:** high.
- **Path** `rules.rs` (330) — the rules and their thresholds. **`KEEP`**; `NARROW` `Features:150`.
- **Path** `rank.rs` (176) — **Responsibility:** where a recovered artifact stands, by the evidence
  about it. **Layer:** core technique. 8 consuming files across `argos_engine` and `argos`.
  **Destination:** survives standalone despite being 176 lines: its input is a manifest record, not
  a pixel, so merging it into `lib.rs` would put two unrelated inputs behind one door.
  **If it vanished:** C27, C35, C39. **Findings:** `KEEP`; `NARROW` `PHOTOGRAPH_MIN_LONG_SIDE:38`.
  **Confidence:** high.
- **Path** `fixture.rs` (483) — labelled corpus generator. **`KEEP`**; `NARROW` `next_u64:28`,
  `jitter:47`.

### 8.6 `argos_engine`

- **Path** `lib.rs` (139) — crate surface + `ScanError`. **`KEEP`**. **Confidence:** high.
- **Path** `session.rs` (390) — **Responsibility:** the driving port: a scan you can start, watch,
  pause and stop. **Layer:** port (driving side). **Destination:** survives. **If it vanished:**
  C43, C44, C47. **Findings:** `KEEP`; `NARROW` `Control` (`session.rs:284`, `redundant_pub_crate`).
  **Confidence:** high.
- **Path** `config.rs` (412) — **Responsibility:** what a scan covers and how much machine it may
  use. **Destination:** survives. **If it vanished:** C17–C21, C23, C29. **Findings:** `KEEP`.
  `ScanConfigBuilder` is the one builder in the workspace and it earns its place: eight fields,
  `build() -> Result` doing the validation (`M-BUILD-RESULT`), and private fields on `ScanConfig`
  so a struct literal cannot bypass it. The six getters that "only return the field" are what that
  costs and are not a finding. **Confidence:** high.
- **Path** `finding.rs` (699) — findings, their consolidation, cache runs. **`KEEP`**; `NARROW`
  `Finding::{is_covered_by:88, intersects:102}`. **If it vanished:** C26, C30, C31.
  **Confidence:** high.
- **Path** `annotate.rs` (554) — **Responsibility:** the post-emit pass: one decode feeding both the
  preview and the classifier. **Destination:** survives. **If it vanished:** C24, C25, C32.
  **Findings:** `KEEP`; `NARROW` `annotate.rs:78,95,117`. **Confidence:** high.
- **Path** `graft.rs` (212) — **Responsibility:** recovering pixels from entropy-coded fragments no
  header reaches, deliberately outside the pipeline. **Destination:** survives. **If it vanished:**
  C51. **Findings:** `KEEP` — its separateness is the point: what it produces is not a file the
  medium held, and the module boundary is what keeps it out of a scan's artifact set.
  **Confidence:** high.
- **Path** `pipeline.rs` (869) + `pipeline/{carving,filesystem,output,reassembly,region}.rs` —
  **Responsibility:** the staged scan; then one file per stage. **Layer:** core.
  **Destination:** `region.rs` merges into `reassembly.rs`; the other four stand. **If they
  vanished:** C09–C16, C19, C20, C30. **Findings:** `MERGE` §6.6; `KEEP` §6.7; `NARROW` at
  `pipeline.rs:135,250,343`, `output.rs:21,28`. The `use super::*;` glob at the head of
  `carving.rs:7`, `filesystem.rs:7`, `output.rs:7`, `reassembly.rs:7` is what makes each child
  unreadable alone — optional polish, §10 Wave 6. **Confidence:** high for `pipeline`/`carving`/
  `output`; **medium for `reassembly`**, whose budget arithmetic I did not follow end to end.
- **Path** `fixture.rs` (160) — `Collector` sink + `Events` collector. **`KEEP`**.

### 8.7 `argos_report`

- **Path** `lib.rs` (879) — **Responsibility:** one output directory and everything written into it.
  **Layer:** adapter (implements `ArtifactSink`). **Destination:** survives. **If it vanished:**
  C30–C33, C36. **Findings:** `KEEP`. Borderline observation, no disposition: it holds four things
  — the `Store` sink, the preview JPEG encoder (`lib.rs:649–782`), the ownership handback
  (`Owner`/`Handback`, `lib.rs:560–648`) and `ReportError`. At 879 lines that is a large file with
  a plural responsibility, and the honest reading is that `b556d2b` folded `preview.rs` and
  `handback.rs` in here and stopped one file short. Splitting would raise the file count, so it is
  recorded rather than proposed. **Confidence:** high.
- **Path** `manifest.rs` (527) — **Responsibility:** the manifest schema and its read/write.
  **Destination:** survives; `mod` → `pub mod`. **If it vanished:** C30, C31, C34, C35, C37–C41 —
  every capability that reads a finished session. **Findings:** `KEEP` §6.9; `NARROW`-inverse at
  `lib.rs:25`; `NARROW` `manifest.rs:19`. **Confidence:** high.

### 8.8 `argos_ipc`

- **Path** `lib.rs` (88) / `dto.rs` (581) / `wire.rs` (407) — **Responsibility:** the schema
  version and the contract; the data both sides agree on; JSON-RPC line framing.
  **Layer:** shared contract, depends on nothing in the workspace. **Destination:** all three
  survive unchanged. **If they vanished:** C47, and with it C34–C41 and C48 through the GUI.
  **Findings:** `KEEP` all. **Explicitly excluded from the `NARROW` pass:** the entire public
  surface of this crate is the wire contract, `argos_ui` consumes it from outside the workspace,
  and the `bindings` feature (`ci.yml:273`) generates TypeScript from it. A visibility narrowing
  here would be measured against a consumer this audit is not allowed to open. **Confidence:** high
  for `wire`/`lib`; **medium for `dto`**, whose consumers are half outside my scope.

### 8.9 `argos`

- **Path** `main.rs` (507) — **Responsibility:** the eight commands clap accepts, and dispatch.
  `scan`, `acquire`, `serve`, `devices`, `report`, `reassemble`, `graft`, `export`.
  **Destination:** survives. **Findings:** `KEEP`. **Confidence:** high.
- **Path** `console.rs` (881) — **Responsibility:** stdout, and the only place printing happens.
  Sole consumer `main.rs`, and that is correct: it is a mode of the binary, not a fragment.
  **If it vanished:** C03–C06, C34, C42–C46. **Findings:** `KEEP`. **Confidence:** high.
- **Path** `scan.rs` (856) — **Responsibility:** driving one scan without deciding how any of it is
  shown. **Destination:** survives. **If it vanished:** C09–C23, C29–C31, C51. **Findings:**
  `KEEP`; one `ARCHITECTURE` item at `scan.rs:466` (§9.1). **Confidence:** high.
- **Path** `medium.rs` (392) + `acquire.rs` (268) — **Responsibility:** where a run reads from and
  where it may write; and copying a medium to an image first. **Destination:** one `medium.rs`
  (660). **If they vanished:** C01, C02, C06–C08, C33, C50. **Findings:** `MERGE` §6.8; `NARROW`
  `medium.rs:230`. **Confidence:** high.
- **Path** `results.rs` (431) — reading a finished session back: standings and export.
  **`KEEP`**. **If it vanished:** C34–C41. **Confidence:** high.
- **Path** `serve.rs` (721) + `serve/{pace,trace,translate}.rs` — **Responsibility:** the engine as
  a process. **Destination:** all four survive (§6.10). **If they vanished:** C47, and every GUI
  line C35, C39, C48. **Findings:** `KEEP`. **Confidence:** high for `serve`/`translate`;
  **medium for `pace`**, whose lossless-dropping argument I checked by reading rather than by test.

---

## 9. Architecture violations

No removal is proposed for anything in this section.

### 9.1 `ARCHITECTURE` — a forensic figure computed in the CLI binary

`crates/argos/src/scan.rs:466–503`, `fn residue_census`, with `BATCH_WINDOW_SECS` at
`scan.rs:457`. Everything else in that neighbourhood (`fragmentation:403`, `lost_files:427`,
`coverage:513`, `volumes:539`, `annotations:554`, `triage_record:569`) is field-by-field
translation from `argos_engine::ScanReport` into `argos_report::*Record`, which is exactly what the
composition root is for — the DAG deliberately keeps `argos_engine` from naming a report type.
`residue_census` is different: it sorts the recovered creation timestamps, counts distinct instants,
and slides a 60-second window over them to find the largest batch, then reports `distinct_instants`
and `largest_minute`. That answers a forensic question — "did a heap of files stop existing in one
action?" — and it is domain reasoning, not translation. It belongs beside the census the engine
already keeps (`ScanReport::orphan_census`, `finding.rs`), where it would be unit-testable without
a manifest and where a second client could not compute it differently.

Both current clients reach it (`argos scan` and `serve` both go through `crate::scan::run`), so no
capability is at risk today; what is at risk is the next client. **Disposition:** `ARCHITECTURE` —
describe, do not remove. Moving it is a behaviour-preserving relocation but it crosses a crate
boundary and adds two fields to `ScanReport`, so it is a change for the author to make deliberately,
not a consolidation wave.

### 9.2 Checked and clean

- `argos_core` names no adapter type: it has zero workspace dependencies.
- `argos_fs`, `argos_carve` and `argos_classify` name each other nowhere and name `argos_device`
  nowhere; `cargo tree --edges normal` confirms it. The documented `argos_classify` →
  `argos_carve` dev-dependency is present and correctly invisible to `--edges normal`.
- `argos_engine` does not depend on `argos_report`; results leave through
  `argos_core::artifact::ArtifactSink` and `argos_report::Store` is injected by the binary.
- `argos_ipc` depends on nothing in the workspace, so it cannot name an engine type.
- No library crate prints: `rg 'println!|dbg!'` outside `crates/argos/src` returns nothing
  (`M-LOG-NOT-PRINT`).
- No write path to a source medium exists to review: `BlockSource` has no write, discard or
  passthrough method, and every `File::options()` in `argos_device` sets `.read(true)` only.

---

## 10. Execution waves

Leaves to centre, Regime A before Regime B. Every wave leaves `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` green, and
that is the acceptance criterion for all of them; per-wave criteria below are what to check *in
addition*. Each wave is one commit, so `git revert <sha>` is the revert.

**Wave 1 — `argos_device` per-OS files.** §6.2 + §6.4. Touches nothing outside
`crates/argos_device/src/`. *Extra acceptance:* the crate still cross-compiles —
`cargo check -p argos_device --target x86_64-pc-windows-msvc` and
`--target x86_64-apple-darwin` (check only; no linker needed). Miri is not required, because no
`unsafe` block changes, only which file it sits in: `cargo +nightly miri test -p argos_device`
should nevertheless run once, since this is the crate the rule names.
*Revert:* `git revert <sha1>`.

**Wave 2 — `argos_device` inventory.** §6.3. Touches `argos_device` plus the three call sites in
`argos` that name `argos_device::shadow`. *Extra acceptance:* `argos devices` still lists shadow
copies where the platform has them (C05); on Linux that is the empty list, which is the case the
regression test covers. *Revert:* `git revert <sha2>`.

**Wave 3 — leaf merges in the technique crates.** §6.5 (`exif` → `jpeg`) and §6.6
(`region` → `reassembly`). Two independent moves; can be one commit or two.
*Extra acceptance:* `cargo +nightly fuzz build --fuzz-dir crates/argos_carve/fuzz` — the fuzz crate
is outside the workspace and `cargo check --workspace` will not see the path change.
*Revert:* `git revert <sha3>`.

**Wave 4 — the binary.** §6.8 (`acquire` → `medium`). *Extra acceptance:*
`cargo test -p argos --test cli` and `--test serve`, both of which drive acquisition end to end.
*Revert:* `git revert <sha4>`.

**Wave 5 — visibility.** §6.12, all 35 `NARROW` items plus the `mod manifest` → `pub mod manifest`
inverse, plus the 17 `redundant_pub_crate` hits that survive waves 1–4. Do this *after* the moves,
because three of the merges make items private for free and narrowing them first would be work
done twice. *Procedure, per crate:* demote every candidate, run
`cargo check -p <crate> --all-targets --all-features`, then `cargo check --workspace --all-targets`;
anything that errors was reachable and goes back. Run
`cargo +nightly fuzz build` for both fuzz crates at the end, since neither is in the workspace.
*Extra acceptance:* `cargo clippy --workspace --all-targets -- -W clippy::nursery` reports zero
`redundant_pub_crate`. *Revert:* `git revert <sha5>`.

**Wave 6 — support targets.** §6.11, four test merges. Independent of everything above.
*Extra acceptance:* the test count reported by `cargo test --workspace` is unchanged — a merge that
loses a `#[test]` is a defect. `ci.yml:142` must still name a `thresholds` target.
*Optional polish in the same wave:* replace `use super::*;` at `pipeline/{carving,filesystem,
output,reassembly}.rs:7` with explicit import lists. *Revert:* `git revert <sha6>`.

**Wave 7 — `argos_core`.** §6.1, last because it touches 62 files across every crate and because
the `ports.rs` half is the one judgement call in the plan. Split it in two commits so the
`geometry` → `lib` merge can stand alone if the ports merge is declined.
*Extra acceptance:* `cargo doc --workspace --no-deps` produces no broken intra-doc link — the crate
doc table at `lib.rs:6–11` links every port by path and all four paths change.
*Revert:* `git revert <sha7b> <sha7a>`.

**Wave 8 — Regime B.** §7, the single removal, on its own commit so it can be reverted without
touching a move. *Extra acceptance:* `cargo test -p argos_fs` and
`cargo +nightly fuzz build --fuzz-dir crates/argos_fs/fuzz`. *Revert:* `git revert <sha8>`.

**Line-count gate, every wave.** `git show --stat` for waves 1–7 must net to within ±20 lines of
zero (module declarations, `use` lines and the odd re-export). A wave that loses 100 lines lost
code during a move; stop and inspect.

---

## 11. Self-critique

**Which module I understood worst.** `crates/argos_device/src/device/windows.rs` and
`inventory/windows.rs`. I read their imports, their item lists and their cross-references, and I
verified they do not name each other — which is what the merge in §6.2 rests on — but I did not
read the 663 combined lines of `windows-sys` calls. If those two files share a `#[repr(C)]` struct
or an ioctl constant through a path I did not see, the merge is still correct but easier than I
described. If either declares an item the *other* directory's file uses via a path my grep did not
match, the merge is still correct, because the compiler would catch it immediately. Low downside,
but the confidence is medium and I have marked it so. Second worst: `argos_carve/src/mcu.rs` and
`reassemble.rs`, 3,069 lines I read structurally. Neither carries a proposal, which is the reason
that is acceptable.

**Which finding is most likely a false positive.** The `NARROW` list for
`crates/argos_fs/src/fixture.rs` — twenty-odd builders that are `pub` and used only by other
builders in the same file. My evidence is a name search across every crate including the two
out-of-workspace fuzz crates, which is sound for a name as distinctive as `apfs_omap_root`, but
a fixture module's whole purpose is to be called from tests that do not exist yet. Narrowing them
is cheap to revert and the compiler makes the mistake obvious, so the risk is low — but if the
author is mid-way through a btrfs or APFS test suite, several of these will need to go back to
`pub` within the week, and that churn is a real cost I am imposing.

**Which merge risks making a file incoherent.** `argos_core/src/ports.rs` at 896 lines, four ports
in one file. I argued it from the doctrine's two-part test and I believe the argument, but I also
notice that I reached for the crate's own doc table as evidence — and a doc table listing four
things is not the same as four things being one thing. `argos_core::progress::ProgressSink` names
its subject; `argos_core::ports::ProgressSink` names its category. That is a real loss for a real
gain of three files, and it is the one item in this plan I would expect a thoughtful author to
decline. I have sequenced it last and split its wave so declining it costs nothing else.

**Which removal I would revert first.** The only one: `BTRFS_MIRROR_AT`. The three evidences are
solid and I verified each by hand, but the constant sits in the newest, still-uncommitted region of
the tree, and "a mirror offset in a fixture module" is precisely the artefact of someone about to
write the test that plants a superblock at the first mirror. If a btrfs mirror-recovery test lands
in the next fortnight, reverting the removal is the right move and the wave was pure cost. That is
why it is alone in wave 8.

**What I did not do.** I did not evaluate performance, allocation or algorithmic cost anywhere —
where a doc comment argues from throughput (`region.rs:1–14`, `pipeline.rs:17–21`) I took the
argument as given and reasoned only about where the code sits. I did not open any file under
`crates/argos_ui`, which means `argos_ipc`'s public surface was assessed from one side only, and I
excluded it from the `NARROW` pass for that reason rather than because it is clean.

---

## 12. Questions

1. **`argos_core::ports`** — do you want the four port modules as one file? §6.1 argues yes from the
   doctrine; §11 argues that `argos_core::progress::ProgressSink` names its subject while
   `argos_core::ports::ProgressSink` names its category. The `geometry` → `lib.rs` half is
   uncontroversial either way. Three files turn on this answer and nothing else does.

2. **`argos_fs/src/fixture.rs` at 2,012 lines** — is the union of six formats' builders one file on
   purpose, or is it the residue of nobody deciding? The alternative is a
   `#[cfg(feature = "test-util")] pub mod fixture` inside each of `ntfs.rs`, `ext4.rs`, `fat.rs`,
   `apfs.rs`, `btrfs.rs`, with the shared `Image`/`FilePlan`/GPT scaffolding staying at the crate
   root. It removes the file and puts each builder beside the parser it feeds, at the cost of
   ~200 call-site path changes across three crates and two out-of-workspace fuzz crates. Only you
   know whether that path change is worth it.

3. **`BTRFS_MIRROR_AT`** (`crates/argos_fs/src/fixture.rs:1591`) — is this scaffolding for a
   btrfs mirror-recovery test you are about to write, or left over from one you decided against?
   If the former, §7 should be dropped and the test should land instead; if the latter, wave 8
   stands.

4. **`residue_census`** (`crates/argos/src/scan.rs:466`) — was computing `distinct_instants` and
   `largest_minute` in the binary a deliberate choice (they are only ever written to a manifest,
   and the engine has no manifest), or should the batch-window analysis move beside
   `ScanReport::orphan_census` where a second client could not compute it differently? §9.1
   describes it; I propose nothing because the answer decides the shape.

5. **`argos_device`'s canonical error** — `DeviceError` lives in `device.rs:186` while every other
   library crate keeps its canonical error in its root (`argos_fs::FsError`,
   `argos_carve::CarveError`, `argos_engine::ScanError`, `argos_report::ReportError`). Is that a
   deliberate exception, or the one place `M-ERRORS-CANONICAL-STRUCTS` was applied by re-export
   rather than by placement?

6. **`argos_report/src/lib.rs` at 879 lines** — the `Store` sink, the preview encoder, the
   ownership handback and `ReportError` in one file. `b556d2b` merged the first three deliberately.
   Is that where you want it, or did that pass stop one file short of splitting the preview encoder
   back out? I recorded it rather than proposing either way, because both directions are
   defensible and only you know which reader you are optimising for.

7. **The `test-util` fixture surfaces** — five crates expose a `fixture` module whose `pub` items
   exist for *other* crates' tests. §6.12 narrows only the ones no file anywhere names, but the
   general question stands: is `<crate>::fixture` a public API you intend to keep stable for
   downstream test suites, or an implementation detail that should be narrowed aggressively every
   time a consumer disappears?
