# Structural consolidation audit — Rust backend

Date: 2026-08-23. Scope: every crate under `crates/` except `crates/argos_ui` (opened only to
answer "does the shell consume this symbol?", never audited). Workspace support targets —
`tests/`, `benches/`, `examples/`, `fuzz/` — are in scope.

Read-only audit. No source file was edited. This document is the only write.

---

## 1. Verdict

The tree is **mildly inflated, and inflated in one place**: `crates/argos/src`, which carries 17
files for 4,806 lines, seven of them fragments of their single consumer. Everything else is close
to right, and three crates (`argos_fs`, `argos_carve`, `argos_ipc`) are already at the shape this
audit would design from scratch.

- Now: **9 crates, 27 directories, 140 `.rs` files, 42,599 lines**, median file 209 lines, 49 files
  under 150 lines, 20 of them in `src/`.
- Projected: **9 crates, 28 directories, 120 `.rs` files, 42,438 lines**, median 265, 6 `src/` files
  under 150. `src/` goes 86 → 69 files, −20%.
- **Biggest lever:** collapse `crates/argos/src` from 17 files to 10 by merging each fragment into
  its one consumer. That single crate is 7 of the 20 files removed and pays for itself twice, by
  killing three duplicated implementations (the device-node predicate, the artifact ordering, the
  long-side calculation) that only exist because the code lives in separate files.
- **No crate merges are proposed.** Every crate boundary here either enforces the §2.2 DAG or
  isolates a dependency; `M-SMALLER-CRATES` cuts the same way.
- **Counterweight:** `argos_engine/src/pipeline.rs` is 2,598 lines carrying five pipeline stages and
  must `SPLIT`. The engine's file count stays flat; its largest file drops to 768 lines.
- Regime B is nearly empty and that is the correct outcome — `cargo udeps`, `cargo machete`, clippy
  (with `nursery`) and a compiler-verified visibility sweep all came back clean. One removal is
  proposed, with three anchors.

---

## 2. Target file tree

### 2.1 Crates — all nine survive

Each crate must answer at least one of the four questions. The answers, and the boundary caveat.

| Crate | Question answered | Consumers | Verdict |
| --- | --- | --- | --- |
| `argos` | binary entry point; the only crate allowed an app-error crate (`M-APP-ERROR`) | — | `KEEP` |
| `argos_core` | **#1 direction** — 7 crates depend on it, it depends on none. The four port traits live here; the compiler is what stops an adapter type reaching the domain | 7 workspace crates | `KEEP` |
| `argos_device` | **#1 + #2** — the only crate allowed `unsafe`, and the only one linking `libc`/`windows-sys`. `argos_engine` cannot see it, which is what makes the engine testable without a device | `argos` | `KEEP` |
| `argos_fs` | **#1** — the DAG forbids it depending on `argos_carve`, `argos_classify` or `argos_device`; the crate boundary is the enforcement | `argos_engine` | `KEEP` |
| `argos_carve` | **#1 + #2** — same DAG rule; isolates `zune-jpeg`, `zune-png`, `miniz_oxide`, `memchr` | `argos_engine` | `KEEP` |
| `argos_classify` | **#1** — same DAG rule. The weakest case (1,193 src lines, the smallest library), but merging it into `argos_engine` would let triage reach the pipeline, which is exactly what `A-TRIAGE-NOT-VERDICT` is structurally guarding against | `argos_engine`, `argos` | `KEEP` |
| `argos_engine` | **#1** — the hexagon's centre; the three technique crates meet only here | `argos` | `KEEP` |
| `argos_report` | **#1 + #2** — `argos_engine` deliberately does *not* depend on it, so the sink stays a port rather than a call; isolates `serde_json` and `jpeg-encoder` from the engine | `argos` | `KEEP` |
| `argos_ipc` | **#1 + #3** — depends on nothing in the workspace, which is what makes it unable to name an engine type; consumed by `argos` and by `argos_ui` outside the workspace | `argos`, `argos_ui` | `KEEP` |

**Boundary caveat, applied.** Every candidate merge I considered (`argos_classify` → `argos_engine`;
`argos_report` → `argos`; `argos_core` → each adapter) dissolves an enforced architectural boundary
and would have to be replaced by module visibility plus a dependency test — strictly weaker, since
the next contributor can add a `use` and nothing complains. `M-SMALLER-CRATES` ("err on the side of
too many crates") points the same way, and collapsing crates would *lengthen* incremental builds,
because the nine currently compile in parallel and rebuild independently. **No crate merge is
proposed.**

### 2.2 Files — before and after

Sizes are current lines (before) and projected lines (after). `→` marks a move.

```
BEFORE                                          AFTER
crates/argos_core/src/            8 files       crates/argos_core/src/            7 files
  lib.rs                     28                   lib.rs                     206   crate doc, the four port module decls, and the
  recovery.rs               178   →lib.rs                                          recovery vocabulary (Confidence/Format/Stage/
  artifact.rs               263                                                    Timestamps) every crate names — M-BALANCED-MODULES
  classify.rs               217                                                    wants the essential items in the root
  geometry.rs               265                   artifact.rs                263   ArtifactSink port + Artifact/Digest/Capture
  progress.rs               209                   classify.rs                217   Classifier port + PixelImage/TriageScore
  source.rs                 207                   geometry.rs                265   Lba/ByteOffset/SectorSize and the two ranges
  fixture.rs                110                   progress.rs                209   ProgressSink port + ScanEvent/Unit/RunState
                                                  source.rs                  207   BlockSource port + ReadError/Geometry
                                                  fixture.rs                 110   feature-gated MemDisk; A-ONE-IMPLEMENTATION
                                                                                   explicitly blesses per-crate fixture modules

crates/argos_device/src/         17 files       crates/argos_device/src/         13 files
  lib.rs                     26                   lib.rs                      26   crate doc + the public surface
  device.rs                 259                   device.rs                  573   every medium this crate opens behind BlockSource
  mock.rs                    54   →device.rs                                       — the OS device, a raw image file, the mock —
  image.rs                  119   →device.rs                                       and the byte-addressed view over any of them
  reader.rs                 141   →device.rs                                 
  device/linux.rs           260                   device/linux.rs            260   syscalls only; the decisions live in class/naming
  device/macos.rs           198                   device/macos.rs            198
  device/windows.rs         358                   device/windows.rs          358
  inventory.rs              206                   inventory.rs               348   what media this machine offers: disks, partitions,
  shadow.rs                 142   →inventory.rs                                    mounts and shadow copies
  inventory/linux.rs        108                   inventory/linux.rs         108
  inventory/macos.rs        108                   inventory/macos.rs         108
  inventory/windows.rs      305                   inventory/windows.rs       305
  inventory/mount.rs        248                   inventory/mount.rs         248   mount-table parsing, all targets
  class.rs                  212                   class.rs                   212   device class + TRIM from an OS answer, all targets
  naming.rs                 390                   naming.rs                  390   path conventions + whole-disk rules, all targets
  acquire.rs                463                   acquire.rs                 463   ddrescue-style multi-pass acquisition

crates/argos_fs/src/              9 files       crates/argos_fs/src/              8 files
  lib.rs                    215                   lib.rs                     303   the crate's shared vocabulary: FsKind, Origin,
  bytes.rs                   88   →lib.rs                                          Anchor, Volume, DeletedFile, FsError, and the
                                                                                   checked reads all six parsers are built on
  part.rs                   284                   part.rs                    284   MBR/GPT incl. backup GPT
  residue.rs                322                   residue.rs                 322   prior-filesystem anchor sweep
  ntfs.rs                  1229                   ntfs.rs                   1229   one file per on-disk format, one fuzz target each
  ext4.rs                   627                   ext4.rs                    627
  fat.rs                    624                   fat.rs                     624
  apfs.rs                   543                   apfs.rs                    543
  fixture.rs               1535                   fixture.rs                1535   synthetic volume builders (test-util)

crates/argos_carve/src/          12 files       crates/argos_carve/src/          10 files
  lib.rs                    535                   lib.rs                     710   the carving surface — Carver, Detector, validate,
  stream.rs                 175   →lib.rs                                          Candidate, Finding, CarveError — and the bounded
                                                                                   cursor its validators read the medium through
  jpeg.rs                   265                   jpeg.rs                    265   JPEG marker state machine
  png.rs                    405                   png.rs                     405   PNG chunk walk + CRC
  exif.rs                   270                   exif.rs                    270   TIFF/IFD walk for thumbnails and capture metadata
  classify.rs               318                   classify.rs                318   block classification
  decode.rs                 332                   decode.rs                  332   pixel decode, the reassembly oracle
  mcu.rs                   1456                   mcu.rs                    1456   entropy-stream decode
  reassemble.rs            1466                   reassemble.rs             1623   the search: hypotheses, gap carve, PUP walk, and
  assemble.rs               157   →reassemble.rs                                   the extent list presented as one stream
  reference.rs              235                   reference.rs               235   a surviving header lent to orphaned fragments
  fixture.rs               1135                   fixture.rs                1135   decodable image and disk builders (test-util)

crates/argos_classify/src/        5 files       crates/argos_classify/src/        4 files
  lib.rs                     85                   lib.rs                     204   what triage does to one image: label it, and hash
  phash.rs                  119   →lib.rs                                          it so near-duplicates collapse
  rules.rs                  330                   rules.rs                   330   the rules and their thresholds
  rank.rs                   176                   rank.rs                    176   standing: ordering by the evidence a record carries
  fixture.rs                483                   fixture.rs                 483   labelled corpus generator (test-util)

crates/argos_engine/src/         12 files       crates/argos_engine/src/         12 files
  lib.rs                     66                   lib.rs                     146   crate doc, the public surface, and ScanError —
  error.rs                   80   →lib.rs                                          where argos_fs, argos_carve and argos_report all
                                                                                   already keep their canonical error
  session.rs                390                   session.rs                 390   the driving port: start/pause/cancel/subscribe
  config.rs                 412                   config.rs                  412   what a scan covers and how much machine it uses
  finding.rs                299                   finding.rs                 690   findings: what the stages produce, how overlapping
  merge.rs                  209   →finding.rs                                      claims consolidate, and what a run of same-sized
  cache_run.rs              182   →finding.rs                                      neighbours means
  annotate.rs               553                   annotate.rs                553   the post-emit pass: one decode, preview + triage
  graft.rs                  212                   graft.rs                   212   the orphan sweep — deliberately not a stage
  fixture.rs                160                   fixture.rs                 160   Collector sink + event collector (test-util)
  pipeline.rs              2598   SPLIT           pipeline.rs                768   stage order, progress counters, and the one
                                                                                   sequential pass over the medium
                                                  pipeline/filesystem.rs     690   stage C: volumes, MFT, journal, index slack
                                                  pipeline/reassembly.rs     570   stage E driver: planning, budget, parallel search
                                                  pipeline/emit.rs           440   stages F/H: measure, hash, write, count
  search.rs                 430   →pipeline/       pipeline/region.rs         430   the in-memory region a search reads from

crates/argos_report/src/          3 files       crates/argos_report/src/          2 files
  lib.rs                    988   SPLIT           lib.rs                     897   one output directory and everything written into
  preview.rs                209   →lib.rs                                          it: the Store sink, previews, ownership handback,
  handback.rs               150   →lib.rs                                          ReportError
                                                  manifest.rs                450   the manifest schema and its read/write — the
                                                                                   thing four different readers parse

crates/argos_ipc/src/             3 files       crates/argos_ipc/src/             3 files
  lib.rs                     88                   lib.rs                      88   SCHEMA_VERSION + the contract doc
  dto.rs                    581                   dto.rs                     581   the data both sides agree on
  wire.rs                   444                   wire.rs                    429   JSON-RPC framing (−15: see Regime B)

crates/argos/src/                17 files       crates/argos/src/                10 files
  main.rs                   534                   main.rs                    450   the commands clap accepts and their dispatch
  console.rs                498                   console.rs                 875   the terminal interface: what a run prints, the
  progress.rs               294   →console.rs                                      live progress it redraws, and the keys that
                                                                                   pause and stop it
  scan.rs                   498                   scan.rs                    791   driving one scan from the CLI: its options, the
  scanlog.rs                190   →scan.rs                                         run, the graft sweep that can follow it, and
  invoker.rs                 36   →scan.rs                                         everything written beside the artifacts
  graft.rs                   67   →scan.rs
  source.rs                 251                   medium.rs                  381   where a run reads from and where it may write:
  destination.rs            140   →medium.rs                                       opening a source read-only, refusing a
                                                                                   destination that sits on it
  acquire.rs                280                   acquire.rs                 275   the acquire command driver and its Notice port
  export.rs                 332                   results.rs                 403   reading a finished session back: how its records
  standing.rs                71   →results.rs                                      rank, and how its artifacts are exported
  serve/mod.rs              672                   serve.rs                   729   the JSON-RPC dispatch loop and the one thing
  serve/wire.rs              57   →serve.rs                                        allowed to write to stdout
  serve/pace.rs             357                   serve/pace.rs              357   event pacing so a client is not drowned
  serve/trace.rs            220                   serve/trace.rs             220   where a session spends its time (stderr, opt-in)
  serve/translate.rs        309                   serve/translate.rs         309   engine vocabulary → wire vocabulary
```

### 2.3 Support targets

```
BEFORE                                          AFTER
argos_carve/tests/                9 files       argos_carve/tests/                7 files
  reassemble_at_scale.rs    217                   reassemble_at_scale.rs     387   recall and cost on media bigger than the search
  reassemble_scale.rs       170   →                                                (two near-identically named binaries, one subject)
  mcu.rs                    185                   mcu.rs                     450   the entropy decoder as an oracle: what it decodes,
  oracle_equivalence.rs     265   →mcu.rs                                          and that it agrees with the reference decoder
  carve.rs                  339                   carve.rs                   339
  decode_rgba.rs            186                   decode_rgba.rs             186
  reassemble.rs             526                   reassemble.rs              526
  recovery_rate.rs          328                   recovery_rate.rs           328
  reference.rs              151                   reference.rs               151

argos_classify/tests/             6 files       argos_classify/tests/             5 files
  probe.rs                   22   →thresholds.rs                                   a 22-line binary with no assertion; its body is a
  thresholds.rs             133                   thresholds.rs              155   diagnostic and thresholds.rs is the declared
                                                                                   diagnostic harness CI runs with --ignored
  eval.rs / phash.rs / rank.rs / rules.rs         unchanged

argos/tests/           4 · argos_core/tests/ 2 · argos_device/tests/ 3 ·
argos_engine/tests/    5 · argos_fs/tests/   1 · argos_report/tests/ 1     unchanged
benches/ 3 · examples/ 4 · fuzz/ 16 targets                                unchanged
```

`tests/` files are separate integration binaries and `benches/`, `examples/`, `fuzz/` targets are
separate compilation units — nothing here is folded into a library. Only the *count* of integration
binaries is reduced, three times, each time by joining two files that share a subject.

All 16 fuzz targets stay: `A-FUZZ-EVERY-PARSER` requires one per parser and `.github/workflows/ci.yml`
runs each by name. All four examples stay: each carries the `A-SUPPORT-DECLARED` line naming what
runs it (`crates/argos/examples/orphan_census.rs:22-27`,
`crates/argos_carve/examples/gen_bench_disk.rs:14-18`).

### 2.4 Directories

27 → 28. Every existing directory already groups three or more siblings and no single-file
directory exists under `src/`. The one new directory, `crates/argos_engine/src/pipeline/`, groups
four siblings and exists because a 2,598-line file must split. `crates/argos/src/serve/` keeps three
siblings after `wire.rs` folds into its parent, and `serve/mod.rs` is renamed `serve.rs` so the
tree contains no `mod.rs` at all — matching `argos_device`'s existing `device.rs` + `device/` and
`inventory.rs` + `inventory/`.

---

## 3. Capability inventory — the invariant contract

The contract already exists and is maintained: **`docs/CAPABILITIES.md`**, 51 numbered capabilities
in user language, each with where it is reachable from. That file, not this one, is the contract.
Every proposal below was checked against it; the mapping from capability group to target file is:

| Capabilities | Held after consolidation by |
| --- | --- |
| C01–C06 open a medium, list media, class/TRIM/mounts/shadow copies, warn | `argos_device/src/{device,inventory,class,naming}.rs`, `argos/src/{medium,console}.rs` |
| C07–C08 acquire to image, zero-fill and list what stayed unreadable | `argos_device/src/acquire.rs`, `argos/src/acquire.rs` |
| C09, C13, C15 carve, validate, partial prefix | `argos_carve/src/{lib,jpeg,png,exif}.rs`, `argos_engine/src/pipeline/emit.rs` |
| C10–C12, C16 filesystem recovery, residue, partition tables, `$UsnJrnl` | `argos_fs/src/*`, `argos_engine/src/pipeline/filesystem.rs` |
| C14, C20, C22 reassembly, its budget, resuming from fragmentation points | `argos_carve/src/{reassemble,mcu,classify,decode}.rs`, `argos_engine/src/pipeline/{reassembly,region}.rs` |
| C51 graft | `argos_carve/src/reference.rs`, `argos_engine/src/graft.rs`, `argos/src/scan.rs` |
| C17–C19, C21, C23, C29 stage switches, range, jobs, size floor | `argos_engine/src/config.rs`, `argos/src/main.rs` |
| C24–C28 triage, perceptual dedup, SHA dedup, standing | `argos_classify/src/{lib,rules,rank}.rs`, `argos_engine/src/annotate.rs` |
| C30–C33 manifest, recorded-not-written, previews, refuse output on source | `argos_report/src/{lib,manifest}.rs`, `argos/src/medium.rs` |
| C34–C41 report, gallery page, export and its five filters | `argos/src/{results,console}.rs`, `argos/src/serve/translate.rs` |
| C42–C47, C50 progress, pause/stop, damage total, warnings, serve | `argos_core/src/progress.rs`, `argos/src/{console,scan,serve}.rs`, `argos_ipc/src/*` |
| C48–C49 elevation, theme | `argos_ui` — out of scope, untouched |

**No proposal in this document removes, relocates out of reach, or narrows the visibility of
anything on that list.** The one `REMOVE` (§7) is a Rust type in `argos_ipc` that no client names
and that changes no JSON field, so C34/C37/C47 are unaffected.

---

## 4. Structural census

### 4.1 Probes run

| Probe | Result |
| --- | --- |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo clippy … -W clippy::nursery` | 41 warnings, all cosmetic (`const fn`, `Self` repetition, early-drop). Nothing structural |
| `cargo clippy … -W clippy::redundant_pub_crate` | 29 hits — `pub(crate)` items inside already-private modules. 21 of them disappear as a side effect of the merges below |
| `cargo +nightly udeps --workspace --all-targets` | *"All deps seem to have been used."* |
| `cargo machete` | one hit, `serde` in `crates/argos_ui/Cargo.toml` — out of scope, reported for completeness |
| `cargo tree --workspace` + manifest read | crate graph in §9 |
| `cargo test --workspace` | green (baseline for the acceptance criteria in §10) |
| `tokei` / size histogram | §4.4 |
| `cargo modules structure` | available but not needed; the module tree is one level deep everywhere except `argos_device` and `argos/serve` |
| **Visibility narrowing** (throwaway copy outside the tree, `pub` → `pub(crate)` per crate, `cargo check --workspace --all-targets --all-features --keep-going`) | ran; results and its one limitation in §4.5 |

`rg` is a shell function in this environment and is not on `PATH` inside scripts; every scripted
search used `grep -r` instead. Nothing was unavailable.

### 4.2 Single-consumer files

Consumers counted as distinct `.rs` files naming the module path, split into same-crate (`i`) and
cross-crate (`X`). Doc-comment mentions were excluded by hand — that correction matters:
`argos_carve/benches/replay.rs` names `argos_engine::search` and `argos_engine::pipeline` only in
`//!` text and does not link `argos_engine` at all, so both are single-consumer.

| File | Lines | Only consumer | Disposition |
| --- | ---: | --- | --- |
| `crates/argos_carve/src/assemble.rs` | 157 | `argos_carve/src/reassemble.rs` | `INLINE` |
| `crates/argos_device/src/mock.rs` | 54 | `argos_device/src/device.rs:54,100` | `INLINE` |
| `crates/argos_device/src/image.rs` | 119 | `argos_device/src/lib.rs:23` (re-export) | `INLINE` |
| `crates/argos_device/src/reader.rs` | 141 | `argos_device/src/lib.rs:25` (re-export) | `INLINE` |
| `crates/argos_report/src/handback.rs` | 150 | `argos_report/src/lib.rs` | `INLINE` |
| `crates/argos_report/src/preview.rs` | 209 | `argos_report/src/lib.rs` | `INLINE` |
| `crates/argos/src/console.rs` | 498 | `argos/src/main.rs` | destination of a `MERGE` |
| `crates/argos/src/invoker.rs` | 36 | `argos/src/scan.rs:158` | `INLINE` |
| `crates/argos/src/scanlog.rs` | 190 | `argos/src/scan.rs` | `INLINE` |
| `crates/argos/src/serve/wire.rs` | 57 | `argos/src/serve/mod.rs:32` | `INLINE` |
| `crates/argos/src/serve/pace.rs` | 357 | `argos/src/serve/mod.rs` | `KEEP` — 357 lines with one nameable job |
| `crates/argos_engine/src/search.rs` | 430 | `argos_engine/src/pipeline.rs` | move under `pipeline/` |
| `crates/argos_engine/src/error.rs` | 80 | 4 files in-crate | `INLINE` into `lib.rs` for consistency |

Two-consumer files where both consumers are one subject: `crates/argos/src/destination.rs` (140,
consumed by `acquire.rs`, `graft.rs`, `scan.rs` — all three also consume `source.rs`),
`crates/argos_engine/src/merge.rs` (209) and `crates/argos_engine/src/cache_run.rs` (182), both
consumed by `finding.rs` and `pipeline.rs`.

### 4.3 Single-file directories and re-export shells

- **Single-file directories:** none under `src/`. Five exist under Cargo-mandated names
  (`crates/argos/examples/`, `crates/argos_classify/benches/`, `crates/argos_fs/tests/`,
  `crates/argos_fs/examples/`, `crates/argos_report/tests/`); these are dictated by Cargo, not by
  the design, and cannot be dissolved.
- **`mod.rs` files:** exactly one, `crates/argos/src/serve/mod.rs`, and it holds 672 lines of
  dispatch logic. Not a shell — but renaming it `serve.rs` leaves the tree with zero `mod.rs`,
  matching the `device.rs`+`device/` convention already used in `argos_device`.
- **Re-export-only shells:** none. 20 `pub use` lines total, all naming individual items.
- **Glob re-exports:** none outside `use super::*` in one `#[cfg(test)]` module
  (`crates/argos_engine/src/pipeline.rs:2432`). `M-NO-GLOB-REEXPORTS` is satisfied.
- **Nearest thing to a shell:** `crates/argos_core/src/lib.rs` is 28 lines — five `pub mod` lines and
  one `pub use recovery::{…}` lifting a private 178-line module wholesale to the root. That is a
  path level with no content, and `M-BALANCED-MODULES` says the crate's essential vocabulary belongs
  in the root anyway. `DISSOLVE` `recovery.rs` into `lib.rs`.

### 4.4 Size histogram

| Bucket | Files (all) | Files (`src/` only) |
| --- | ---: | ---: |
| < 50 | 21 | 2 |
| 50–149 | 28 | 18 |
| 150–299 | 47 | 30 |
| 300–599 | 30 | 22 |
| 600–999 | 6 | 5 |
| 1000–1999 | 7 | 8 |
| 2000+ | 1 | 1 |

The 21 files under 50 lines are almost all fuzz targets (16 of them, 9–55 lines each) — required,
one per parser, and not mergeable. The real signal is the 18 `src/` files in the 50–149 band: two
thirds of them are the single-consumer fragments listed above.

The single 2000+ file is `crates/argos_engine/src/pipeline.rs`.

### 4.5 Visibility narrowing probe

Procedure, run in `/tmp/…/probe` outside the working tree: copy the workspace, rewrite every
`pub` (not `pub(crate)`/`pub(super)`) to `pub(crate)` in one crate's `src/`, `touch` every `.rs`
(`git archive` back-dates mtimes and cargo would otherwise reuse the stale fingerprint), then
`cargo check --workspace --all-targets --all-features --keep-going` and collect `E0603`/`E0616`.
What the compiler names is the minimum surface that must stay `pub`; everything else is a `NARROW`
candidate.

**Limitation, stated rather than papered over:** one pass only reveals the needs of crates whose own
dependencies still compiled. Demoting `argos_core` breaks `argos_carve` and `argos_fs`, so
`argos_engine` and `argos` are never checked and their needs never appear — which is why
`argos_core::progress` is absent from the `argos_core` result despite having 16 consumers. The
procedure must be iterated bottom-up to a fixed point (promote what one pass named, re-run) before
its output is a `NARROW` list. The per-crate results below are therefore a **lower bound on what
must stay public**, not an upper bound on what can be narrowed.

What one pass established, per crate — the surface the compiler proved is load-bearing:

- `argos_core`: modules `artifact`, `classify`, `geometry`, `source`, `fixture`; `Confidence`,
  `Format`, `Timestamps`; `ByteRange::{start,len}`.
- `argos_device`: modules `acquire`, `class`, `inventory`, `naming`, `shadow`; `Device`,
  `ImageSource`, `BlockReader`, `TrimState`; 7 `DeviceInfo` fields, 2 `ShadowCopy` fields.
- `argos_fs`: all six format modules plus `fixture`; `FsKind`, `Origin`, `Volume`, `DeletedFile`;
  `Ntfs` geometry fields, `NameGhost` fields, `Tables::partitions`.
- `argos_carve`: all nine modules; `Carver`, `Detector`, `Candidate`, `Finding`, `Scratch`,
  `Thumbnail`, `Verdict`; `validate`, `identify`, `metadata`, `validate_thumbnail`;
  `MAX_IMAGE_BYTES`, `SIGNATURE_OVERLAP_BYTES`, `MAX_SIGNATURE_BYTES`.
- `argos_classify`: modules `phash`, `rank`, `rules`, `fixture`; `Triage`; `RULES_VERSION`;
  5 `Features` fields.
- `argos_engine`: modules `config`, `graft`, `fixture`; `ScanSession`, `Medium`, `ScanConfig`,
  `Stages`, `ScanReport`, `Finding`, `Broken`; 20 `ScanReport`/`Finding`/`Grafted` fields;
  `DEFAULT_MIN_LONG_SIDE`, `DEFAULT_REASSEMBLY_BUDGET`, `merge_for_test`.
- `argos_report`: every record type and 26 of their fields; `Store`, `Manifest`, `Owner`,
  `Handback`, `Summary`; `PREVIEW_DIR`; `unix_seconds`.
- `argos_ipc`: modules `dto`, `wire`; `SCHEMA_VERSION`.

**A name-level cross-check** (every `pub` item whose identifier is never written outside its own
crate's `src/`, counting the crate's own `tests/`, `benches/`, `examples/` and `fuzz/` as
consumers) returns **105 of 624 pub items in library crates**. A substantial share of those are
types reached only through inference — error types returned through `?`, structs returned from
public functions — so the number is an upper bound on the `NARROW` list, not the list. The
unambiguous subset, all constants and free functions:

`argos_carve`: `MIN_BLOCK_BYTES`, `HIGH_ENTROPY_BITS`, `LOW_ENTROPY_BITS` (`classify.rs:26,33,36`),
`MAX_PIXELS` (`decode.rs:38`), `MAX_FRAGMENTS`, `SEARCH_RADIUS_BYTES`, `MAX_SEAM_RATIO`,
`MAX_PREFIX_CANDIDATES`, `MAX_BRANCH` (`reassemble.rs:56,74,87,95,983`), `MAX_SEAMS`
(`mcu.rs:182`), `MAX_HEADER_BYTES` (`reference.rs:40`), `seam_ratio` (`decode.rs:73`),
`header_dimensions` (`png.rs:279`), `total_len` (`assemble.rs:70`).
`argos_classify`: `PHOTOGRAPH_MIN_LONG_SIDE` (`rank.rs:38`) and five threshold constants in
`rules.rs`.
`argos_core`: `SectorSize::{MIN_BYTES,MAX_BYTES}` (`geometry.rs:94,97`).
`argos_device`: `class::{from_rotational,from_seek_penalty,from_solid_state}` (`class.rs:21,35,48`),
`ImageSource::DEFAULT_SECTOR_SIZE` (`image.rs:26`), `WINDOWS_PHYSICAL_DRIVE` (`naming.rs:31`),
`SHADOW_COPY_PREFIX`, `MAX_SHADOW_COPY_INDEX` (`shadow.rs:37,65`),
`inventory::mount::{parse_linux_mountinfo,mounts_of}` (`mount.rs:28,107`).
`argos_engine`: `CHUNK_OVERLAP_BYTES`, `DEFAULT_CHUNK_BYTES` (`config.rs:15,27`), `queue_depth`,
`range_within` (`config.rs:197,224`).
`argos_fs`: `SUPERBLOCK_OFFSET` (`ext4.rs:28`), `DEFAULT_RECORD_SIZE` (`ntfs.rs:56`), `MAX_VOLUMES`,
`MAX_RECORD_REGIONS` (`residue.rs:43,47`), `has_container_magic`, `fletcher64`
(`apfs.rs:320,521`), plus four `pub fn` fixture helpers used only inside `fixture.rs`
(`fixture.rs:358,946,1113,1234`).

`M-DOCUMENTED-MAGIC` requires these constants to be *named and documented*; it does not require
them to be `pub`. `A-NO-SPECULATIVE-SURFACE` says surface with no consumer is a defect. They cut the
same way: `NARROW` to `pub(crate)`.

### 4.6 Crate consumer counts

| Crate | Workspace consumers | Dev-only consumers |
| --- | ---: | --- |
| `argos_core` | 7 | — |
| `argos_carve` | 1 (`argos_engine`) | `argos` (test-util) |
| `argos_fs` | 1 (`argos_engine`) | — |
| `argos_classify` | 2 (`argos_engine`, `argos`) | — |
| `argos_engine` | 1 (`argos`) | — |
| `argos_device` | 1 (`argos`) | — |
| `argos_report` | 1 (`argos`) | — |
| `argos_ipc` | 1 (`argos`) + `argos_ui` outside the workspace | — |

Six crates have exactly one consumer. That would be a merge signal in a layered codebase; here it
is the expected shape of a hexagon whose composition root is a single binary. See the boundary
caveat in §2.1.

---

## 5. Metrics

| field | before | after (projected) |
| --- | ---: | ---: |
| crates | 9 | 9 |
| directories containing `.rs` | 27 | 28 |
| `.rs` files | 140 | 120 |
| `.rs` files under `src/` | 86 | 69 |
| max path depth (components) | 5 | 5 |
| max module-path depth | 3 | 3 |
| median file size (lines) | 209 | 265 |
| median `src/` file size (lines) | 259.5 | 357 |
| files under 150 lines | 49 | 33 |
| `src/` files under 150 lines | 20 | 6 |
| largest file (lines) | 2,598 | 1,623 |
| single-file directories under `src/` | 0 | 0 |
| re-export-only `mod.rs` | 0 | 0 |
| `mod.rs` files | 1 | 0 |
| files with exactly one consumer | 13 | 2 |
| `pub` items in library crates | 624 | ~570 |
| `pub` items with no consumer outside their crate's `src/` | 105 | ~50 |
| clippy `redundant_pub_crate` hits | 29 | 8 |
| traits with 1 impl outside a port | 0 | 0 |
| `#[allow(dead_code)]` | 0 | 0 |
| `#[expect(dead_code, reason = …)]` | 1 | 1 |
| pure forwarding wrappers | 3 | 0 |
| duplicated implementations of one behaviour | 4 | 0 |
| error enums / canonical error structs | 11 structs + 3 private kind enums | unchanged |
| **total lines** | **42,599** | **42,438** |

**On the −161 lines.** Regime A must be line-neutral, and it very nearly is. The drop is
accounted for exactly: −130 from deleting `ExtentReader` in favour of the identical
`argos_carve::assemble::Assembled` (§6.7), −15 from the one Regime B removal (§7), and ~−16 from
`use` blocks that coalesce when two files become one. Everything else is a move. If an execution of
this plan produces a larger drop, code was deleted during a move and that is a defect — the wave
acceptance criteria in §10 include a line-count check for exactly that reason.

---

## 6. Consolidation plan (Regime A)

Grouped by destination file. Every item is a move plus a visibility change; no behaviour changes.
Each move is a `git mv` where a file becomes the destination, and a `git mv` + edit where a file is
absorbed, so history follows the code.

### 6.1 → `crates/argos_core/src/lib.rs`

- `DISSOLVE` `crates/argos_core/src/recovery.rs` (178) into `lib.rs` (28). **Result's
  responsibility:** the crate root — the port module declarations and the recovery vocabulary
  (`Confidence`, `Format`, `Stage`, `Timestamps`, `UnknownFormat`) every other crate names.
  **Why:** `recovery` is a private module whose entire contents are lifted to the root by
  `lib.rs:28`; it is a path level with no content. `M-BALANCED-MODULES` wants the essential items
  in the root, which is where the re-export already puts them.
- `NARROW` `SectorSize::MIN_BYTES` and `MAX_BYTES` (`geometry.rs:94,97`) to `pub(crate)`.

### 6.2 → `crates/argos_device/src/device.rs`

- `INLINE` `mock.rs` (54) — sole consumer is `device.rs:54` (`Device::Mocked`) and `device.rs:100`
  (`Device::new_mocked`); `mock::Ctrl` is the state of a `Device` variant and belongs beside it.
- `INLINE` `image.rs` (119) — `ImageSource` is a second `BlockSource` implementation; its only
  in-crate reference is the `pub use` at `lib.rs:23`.
- `INLINE` `reader.rs` (141) — `BlockReader<S>` is the `BlockSource` → `Read + Seek` adapter; only
  in-crate reference is `lib.rs:25`.
- **Result's responsibility (573 lines):** every medium this crate can open behind `BlockSource` —
  the OS device, a raw image file, the mock — and the byte-addressed view over any of them. The
  per-OS submodules under `device/` are untouched; the public paths do not change, because all
  three types are already re-exported at the crate root.
- `NARROW` `ImageSource::DEFAULT_SECTOR_SIZE` (`image.rs:26`) to `pub(crate)`.
- Not proposed, deliberately: any change to the `class.rs` / `naming.rs` / `inventory/mount.rs`
  arrangement, which compiles on every target by design so the decisions are tested everywhere.

### 6.3 → `crates/argos_device/src/inventory.rs`

- `MERGE` `shadow.rs` (142) into `inventory.rs` (206). **Result's responsibility (348 lines):** what
  media this machine offers — disks, partitions, their mounts, and the shadow copies a platform
  keeps. `argos/src/console.rs` and `argos/src/serve/translate.rs` already consume the two together.
- `NARROW` `SHADOW_COPY_PREFIX`, `MAX_SHADOW_COPY_INDEX` (`shadow.rs:37,65`),
  `parse_linux_mountinfo`, `mounts_of` (`inventory/mount.rs:28,107`),
  `class::{from_rotational,from_seek_penalty,from_solid_state}` (`class.rs:21,35,48`),
  `WINDOWS_PHYSICAL_DRIVE` (`naming.rs:31`).
- `NARROW` the re-export `crates/argos_device/src/lib.rs:24` `pub use inventory::{DeviceInfo,
  MountPoint};` — no consumer outside `argos_device` writes either name; every consumer reaches
  them through `inventory::list()` and field access. `M-BALANCED-MODULES` argues the other way
  (essential items in the root), so this one is low priority and marked as a judgement call.

### 6.4 → `crates/argos_fs/src/lib.rs`

- `MERGE` `bytes.rs` (88) into `lib.rs` (215). **Result's responsibility (303 lines):** the crate's
  shared vocabulary — `FsKind`, `Origin`, `Anchor`, `Volume`, `DeletedFile`, `FsError` — and the
  checked reads all six parsers are built on. **Why:** `bytes.rs` has six intra-crate consumers, so
  it is not a fragment of any one of them; but at 88 lines of `pub(crate)` helpers it is crate-wide
  vocabulary, which is what `lib.rs` already holds. This also clears 7 of the 29
  `redundant_pub_crate` hits.
- `NARROW` `SUPERBLOCK_OFFSET` (`ext4.rs:28`), `DEFAULT_RECORD_SIZE` (`ntfs.rs:56`), `MAX_VOLUMES`,
  `MAX_RECORD_REGIONS` (`residue.rs:43,47`), `has_container_magic`, `fletcher64`
  (`apfs.rs:320,521`), and the four fixture helpers at `fixture.rs:358,946,1113,1234` that are
  called only from `fixture.rs` itself.

### 6.5 → `crates/argos_carve/src/lib.rs`

- `MERGE` `stream.rs` (175) into `lib.rs` (535). **Result's responsibility (710 lines):** the
  carving surface — `Carver`, `Detector`, `Candidate`, `Finding`, `validate`, `CarveError` — and the
  bounded cursor its validators read the medium through. `stream::Bytes` has three consumers
  (`jpeg.rs`, `png.rs`, `mcu.rs`), all of them driven from `lib.rs`.

### 6.6 → `crates/argos_carve/src/reassemble.rs`

- `INLINE` `assemble.rs` (157). Sole consumer is `reassemble.rs`; `Assembled` exists to present a
  hypothesis's extent list to the decoder. **Result (1,623 lines):** the reassembly search —
  hypotheses, gap carving, the PUP walk, and the extent list presented to the oracle as one stream.
- `NARROW` `Assembled` and `total_len` — **do this last**, and only after §6.7, which makes
  `Assembled` the engine's implementation too and therefore requires it to stay `pub`.

### 6.7 → deduplicate the extent reader (`A-ONE-IMPLEMENTATION`)

`crates/argos_engine/src/pipeline.rs:2303` `pub(crate) struct ExtentReader<'a, V>` and
`crates/argos_carve/src/assemble.rs:18` `pub struct Assembled<'a, R>` are the same type: identical
fields (`view`/`src`, `extents`, `index`, `consumed`, `positioned`), identical `Read` loop.
`Assembled` is a strict superset — it also implements `Seek` and exposes `len()`/`is_empty()`.
`argos_engine` already depends on `argos_carve`.

`FLATTEN`: delete `ExtentReader` and have `hash_extents` and the emit path construct
`argos_carve::assemble::Assembled`. This is the one item in Regime A that removes lines (~130). It
qualifies under Regime B as well and the three anchors are recorded in §7.2 for review; it is listed
here because the behaviour does not disappear, it moves.

### 6.8 → `crates/argos_classify/src/lib.rs`

- `MERGE` `phash.rs` (119) into `lib.rs` (85). **Result (204 lines):** what triage does to one
  image — label it by the rules, and hash it so near-duplicates collapse. `phash` is called
  directly by `argos_engine/src/annotate.rs`, so the path shortens from
  `argos_classify::phash::hash` to `argos_classify::hash` (`M-SINGLE-ITEM-PATH`).
- `NARROW` `PHOTOGRAPH_MIN_LONG_SIDE` (`rank.rs:38`) and the five threshold constants in `rules.rs`
  that no file outside `argos_classify/src` names.

### 6.9 → `crates/argos_engine/src/lib.rs`

- `INLINE` `error.rs` (80). **Result (146 lines):** the crate root — its doc, its public surface,
  and `ScanError`. **Why:** `argos_fs`, `argos_carve` and `argos_report` all keep their canonical
  error struct in `lib.rs`; `argos_engine` is the only crate that gives an 80-line error type a
  file. `M-ERRORS-CANONICAL-STRUCTS` governs the shape of the type, not where it lives.

### 6.10 → `crates/argos_engine/src/finding.rs`

- `MERGE` `merge.rs` (209) and `cache_run.rs` (182) into `finding.rs` (299). **Result (690 lines):**
  findings — what the stages produce, how overlapping claims from two stages consolidate into one,
  and what a run of same-sized neighbours means about them. All three already reference each other
  (`finding.rs` uses `CacheRun`; `merge.rs` operates on `Finding`), and all three are consumed by
  the same two files.

### 6.11 `SPLIT` `crates/argos_engine/src/pipeline.rs` — the counterweight

2,598 lines carrying five distinct stages plus the progress plumbing. This is the file where
consolidation would be a mistake and the tree should grow.

| New file | Lines | Responsibility | Moved items |
| --- | ---: | --- | --- |
| `pipeline.rs` | 768 | stage order, the progress counters every stage reports through, and the one sequential pass over the medium | `Bound`, `Counter`, the four `PROGRESS_*` constants, `run`, `resume`, `report_findings`, `same_size_runs`, `standings`, `Span`, `Chunk`, `Swept`, `sweep`, `read_chunks`, `fill_chunk`, `read_exact_at` |
| `pipeline/filesystem.rs` | 690 | stage C: recovering deleted files from every volume the sweep located | `recover_filesystems`, `confirm_ntfs`, `name_from_change_journal`, the index-slack naming pass |
| `pipeline/reassembly.rs` | 570 | stage E: planning the search, spending its budget, and turning a hypothesis into a finding | `Reassembling`, `reassemble_broken`, `Searching`, `in_parallel`, `search_region`, `allocation_grid`, `plan_search`, `worth_reporting`, `partial_prefixes`, `finding_from_reassembly` |
| `pipeline/emit.rs` | 440 | stages F/H: measuring, hashing and writing what the stages found | `Measure`, `Measured`, `clears_floor`, `emit`, `Writing`, `Hashing`, `hash_extents` (`ExtentReader` is deleted by §6.7) |
| `pipeline/region.rs` | 430 | the in-memory region a search reads its hypotheses from | all of the current `search.rs`, `git mv`-ed |

The `#[cfg(test)] mod tests` at `pipeline.rs:2431-2598` splits with the code it exercises. The
resulting `pipeline/` directory groups four siblings on one subject.

### 6.12 `SPLIT` and merge `crates/argos_report/src`

- `SPLIT` `lib.rs` (988) at its existing seam: lines 31–478 are the manifest schema and its
  read/write; lines 479–894 are `Store`; 895–988 are `Summary`, `ReportError` and helpers.
  New `manifest.rs` (~450): **the manifest schema and its read/write** — the thing
  `argos/src/console.rs`, `argos/src/results.rs`, `argos/src/serve/translate.rs` and
  `argos/src/scan.rs` all parse.
- `INLINE` `preview.rs` (209) and `handback.rs` (150) into the remainder.
  **Result `lib.rs` (~897):** one output directory and everything written into it — the `Store`
  sink, the previews it renders, the ownership it hands back, and `ReportError`.
- `NARROW` the inherent `Store::record_only` (`lib.rs:700`) and `Store::save_preview`
  (`lib.rs:798`): both are called only from the `ArtifactSink` impl at `lib.rs:876,889` via
  `Self::record_only(self, …)` / `Self::save_preview(self, …)`. Two of the three pure forwarding
  wrappers in the tree. `M-ESSENTIAL-FN-INHERENT` argues for keeping essential functions inherent —
  but `Store::save` (`lib.rs:868`) is the one an outside caller uses, and these two have no outside
  caller at all.

### 6.13 → `crates/argos/src/main.rs` and `console.rs`

- `MERGE` `progress.rs` (294) into `console.rs` (498), and move the `println!` blocks out of
  `main.rs`'s `run_scan`, `run_graft`, `run_export` and `run_acquire` into it.
  **Result `console.rs` (~875):** the terminal user interface — what a run prints, the live progress
  it redraws, and the keys that pause and stop it. `Console` (the `Notice` impl) and `Renderer` (the
  `ProgressSink` impl) are the two halves of the same interface and are both consumed only by
  `main.rs`.
  **Result `main.rs` (~450):** the commands clap accepts and their dispatch, with no printing.

### 6.14 → `crates/argos/src/scan.rs`

- `INLINE` `scanlog.rs` (190) — sole consumer is `scan.rs`.
- `INLINE` `invoker.rs` (36) — sole consumer is `scan.rs:158`; 36 lines deciding which account the
  output is handed to.
- `MERGE` `graft.rs` (67) — consumers are `main.rs` and `scan.rs`; the graft command is a scan's
  reference sweep without the scan, and `scan::graft_after` already drives it.
- **Result (791 lines):** driving one scan from the CLI — its options, the run, the graft sweep that
  can follow it, and everything written beside the artifacts.

### 6.15 → `crates/argos/src/medium.rs` (new name for `source.rs`)

- `MERGE` `destination.rs` (140) into `source.rs` (251), renaming the result `medium.rs`.
  **Result (381 lines):** where a run reads from and where it may write — opening a source
  read-only, and refusing a destination that would sit on it.
- **What the merge pays for:** the device-node predicate is written three times in this crate today
  and one of them differs.
  - `crates/argos/src/source.rs:223` `is_block_device` — `cfg(unix)`, block **or** char device.
  - `crates/argos/src/acquire.rs:134` `is_device_node` — `cfg(unix)`, **byte-for-byte identical**.
  - `crates/argos/src/destination.rs:113` `is_block_device` — `cfg(target_os = "linux")`, block
    only.
  Collapse the first two into one `pub(crate) fn` in `medium.rs`; keep the third's narrower
  behaviour only if it is deliberate, and see Q3.

### 6.16 → `crates/argos/src/results.rs` (new name for `export.rs`)

- `MERGE` `standing.rs` (71) into `export.rs` (332), renaming the result `results.rs`.
  **Result (403 lines):** reading a finished session back — how its records rank, and how its
  artifacts are exported. Both operate exclusively on `argos_report::Manifest` and `ArtifactRecord`.
- **What the merge pays for — two more `A-ONE-IMPLEMENTATION` findings:**
  - **The artifact ordering is written twice, and the two disagree.**
    `crates/argos/src/console.rs:453-457` sorts by `(standing::rank, standing::long_side, length)`.
    `crates/argos/src/serve/translate.rs:91-98` sorts by the same three keys **plus**
    `.then(left.sha256.cmp(&right.sha256))`. Two artifacts of equal standing, equal long side and
    equal length therefore come out in one order from `argos report` and another from the wire
    gallery, which `A-CLI-FIRST` forbids. `MERGE` both into one `results::ordered(manifest, floor,
    include_unwritten)`; console and translate both call it.
  - **"Long side of a record" is written twice.** `crates/argos/src/standing.rs:64-72`
    `long_side(record)` takes the max of `width`, `height`, `declared_width`, `declared_height`;
    `crates/argos/src/export.rs:99-104` inlines the identical four-way max inside `Filter::admits`.
    One call replaces the copy.

### 6.17 → `crates/argos/src/serve.rs`

- `INLINE` `serve/wire.rs` (57) into `serve/mod.rs` (672) and `git mv serve/mod.rs serve.rs`.
  **Result (729 lines):** the JSON-RPC dispatch loop and the one thing in the process allowed to
  write to stdout. `Wire` is a 40-line `Mutex<BufWriter<Stdout>>` whose only consumer is the
  dispatch loop it serves.
  After this, `crates/argos/src/serve/` holds three siblings — `pace.rs`, `trace.rs`,
  `translate.rs` — and the workspace holds zero `mod.rs`.

### 6.18 Integration binaries

- `MERGE` `crates/argos_carve/tests/reassemble_scale.rs` (170) into
  `tests/reassemble_at_scale.rs` (217). **Result (387):** reassembly measured on a medium bigger
  than its own search — what it recovers, and what the walk costs. Two integration binaries with
  near-identical names, both building hundreds-of-megabytes sparse fixtures, become one with two
  named sections.
- `MERGE` `crates/argos_carve/tests/oracle_equivalence.rs` (265) into `tests/mcu.rs` (185).
  **Result (450):** the entropy decoder as an oracle — what it decodes, and that it agrees with the
  reference decoder.
- `MERGE` `crates/argos_classify/tests/probe.rs` (22) into `tests/thresholds.rs`, as an
  `#[ignore]`d diagnostic. `probe.rs` contains one test function with **no assertion** — it decodes
  two fixtures and `eprintln!`s their feature values. `thresholds.rs` is the declared diagnostic
  harness that `.github/workflows/ci.yml` runs with `--ignored --nocapture`; that is where a
  distribution print belongs, and `A-SUPPORT-DECLARED` then covers it.
- *Optional, lower value:* `crates/argos_engine/tests/merge_scale.rs` (96) into `tests/pipeline.rs`
  (1,523). Not proposed — the destination is already the largest test binary in the workspace.

---

## 7. Removal list (Regime B)

Two items. Both are built for item-by-item review; each carries three anchored evidences.

### 7.1 `pub enum Message` — `crates/argos_ipc/src/wire.rs:309`

An inbound-message discriminator (`Response` | `Notification`) for a client parsing the stream.

1. **No reachable caller.** The only two clients of this wire format are
   `crates/argos/src/serve/mod.rs` (which parses `Request`, imports at `serve/mod.rs:27`) and
   `crates/argos_ui/src/engine.rs` (which parses `Response` at `engine.rs:201`; its import at
   `engine.rs:15` is `{Call, Outcome, Reply, Request, Response}` and does not include `Message`).
   Every reference in the tree is inside `wire.rs` itself.
2. **No test exercising the behaviour.** The only uses are `wire.rs:330` (test import),
   `wire.rs:382` and `wire.rs:386`, in a unit test that deserializes into `Message` and asserts it
   matched the variant it was given — a test of the type, not of a behaviour.
3. **No unique behaviour.** Both real clients discriminate by attempting `Response`
   (`argos_ui/src/engine.rs:201`) or by matching a `Call` (`argos/src/serve/mod.rs`), so the
   "either kind" union is produced by another path today.

**Tension to weigh before executing.** `crates/argos_ipc/src/lib.rs:7-11` states the crate exists
for "a future client nobody has written", which is a licence to keep speculative surface;
`A-NO-SPECULATIVE-SURFACE` says the opposite and names exactly this shape. Removing it changes no
JSON field, so `SCHEMA_VERSION` does not move and capabilities C34/C37/C47 are untouched. Commit
`c2134d2` already ran a sweep of this kind; this is one it missed. ~15 lines.

### 7.2 `pub(crate) struct ExtentReader` — `crates/argos_engine/src/pipeline.rs:2303`

Listed here for review even though §6.7 executes it as a consolidation, because it is the one item
whose evidence must be checked the Regime B way.

1. **No reachable caller after substitution.** Its only callers are `hash_extents`
   (`pipeline.rs:2283`) and the emit path in the same file; both can construct
   `argos_carve::assemble::Assembled::new(view, extents)` instead, and `argos_engine` already
   depends on `argos_carve` (`crates/argos_engine/Cargo.toml`).
2. **No test exercising it uniquely.** It has no direct test; it is covered transitively by
   `crates/argos_engine/tests/pipeline.rs`, which exercises the same hashing path and would cover
   the replacement identically.
3. **No unique behaviour.** `crates/argos_carve/src/assemble.rs:18-68` has the same five fields and
   the same `Read` loop, plus `Seek` and `len()`. The replacement is a strict superset.

**Check before executing:** `Assembled::new` takes `&mut R` where `ExtentReader::new` takes
`&mut V`; both are `Read + Seek`. Confirm the emit path does not rely on `ExtentReader` *not*
implementing `Seek` (nothing suggests it does). ~130 lines.

### Explicitly not removed

Everything else that looked dead is not. In particular: `crates/argos_device/src/class.rs:21,35,48`
(`from_rotational`, `from_seek_penalty`, `from_solid_state`) are each called from exactly one
per-OS HAL under `cfg`, and `from_rotational` only from `device/linux.rs:173`; a `dead_code` warning
on a non-host target is not evidence. `crates/argos_device/src/device.rs:198-201` carries the
tree's only `#[expect(dead_code, reason = …)]`, correctly scoped by `cfg_attr` to the three targets
with a native HAL — `M-LINT-OVERRIDE-EXPECT`-compliant and load-bearing. The whole of
`crates/*/fuzz/` and `crates/*/examples/` is reachable under route 3 and route 4 of
`A-EARNS-ITS-PLACE` and stays.

---

## 8. Findings by module

Each entry is self-contained. "Lost if it vanished" is answered against `docs/CAPABILITIES.md`.

### `crates/argos_core/src`

- **Responsibility.** The domain vocabulary and the four ports every other crate either implements
  or calls through.
- **Layer.** Core / ports. **Lines** 1,477 across 8 files. **Pub items** 88; 4 with no name written
  outside the crate's `src/`. **Consuming files** 62 distinct files name `geometry`, 19 `classify`,
  16 each `artifact` and `progress`, 14 `source`, 5 `fixture`.
- **Reachable from.** Everything.
- **Destination.** 7 files; `recovery.rs` dissolves into `lib.rs`.
- **If it vanished.** Every capability. It is the hexagon.
- **Findings.**
  - `lib.rs` is a 28-line shell whose one `pub use` lifts a private module to the root →
    `DISSOLVE` `recovery.rs` (§6.1).
  - `BlockSource`, `ArtifactSink`, `Classifier`, `ProgressSink`: four traits, each with 2–4
    implementations (a real adapter, a fixture, sometimes a null adapter such as
    `classify::AcceptAll` and `progress::Discard`). All four are ports, core declares them,
    infrastructure implements them, the dependency points inward. **`KEEP`, always.** They look
    superfluous from inside `argos_core` — the visibility probe reports all four as "never used" in
    a crate that never calls them — and that is precisely what a port looks like.
  - `SectorSize::{MIN_BYTES,MAX_BYTES}` have no external consumer → `NARROW`.
  - `fixture.rs` (110) is under 150 lines but `KEEP`: `A-ONE-IMPLEMENTATION` explicitly protects
    per-crate `fixture` modules behind `test-util`, five files name `argos_core::fixture::MemDisk`,
    and folding it into `source.rs` would churn that path for nothing.
- **Confidence.** High.

### `crates/argos_device/src`

- **Responsibility.** Read-only `BlockSource` adapters: the per-OS HAL, raw image files, and
  multi-pass acquisition.
- **Layer.** Infrastructure adapter. **Lines** 3,597 across 17 files. **Pub items** 74; 21 unnamed
  outside the crate's `src/`.
- **Who calls it.** `argos/src/{medium,acquire,console,serve/translate}.rs`,
  `argos/examples/orphan_census.rs`. **What it calls.** `argos_core::source`, `libc`/`windows-sys`.
- **Reachable from.** CLI (`argos devices`, `scan`, `acquire`), GUI through `serve`, and three test
  binaries.
- **Destination.** 13 files (§6.2, §6.3).
- **If it vanished.** C01–C08 — every capability that touches hardware.
- **Findings.**
  - Four single-consumer fragments: `mock.rs` (54), `image.rs` (119), `reader.rs` (141), `shadow.rs`
    (142) → `INLINE`/`MERGE`.
  - Two error structs in one crate (`DeviceError` at `device.rs:171`, `AcquireError` at
    `acquire.rs:167`). Not a finding: `AcquireError` is the *destination*-write failure with its
    own message and backtrace, `DeviceError` is the source-open failure with a private
    `DeviceErrorKind`. `M-ERRORS-CANONICAL-STRUCTS` asks for exactly this shape and does not ask for
    one per crate. `KEEP`.
  - `impl AsRef<Path>` at `device.rs:68` and `image.rs:33` on internal APIs → the "inherited
    defensiveness" pattern; `M-IMPL-ASREF` is the rule that put it there, so it is a rule collision
    and I propose nothing. Noted only.
  - The `class.rs` / `naming.rs` / `inventory/mount.rs` arrangement, compiled on every target with
    only syscalls behind `cfg`, is `KEEP` by mandate and by `M-MOCKABLE-SYSCALLS`.
  - `lib.rs:24` re-exports two types nobody names → low-priority `NARROW`, judgement call.
- **Confidence.** High for the merges; medium for the `lib.rs:24` re-export (see Q4).

### `crates/argos_fs/src`

- **Responsibility.** Partition tables, per-filesystem metadata recovery, and the residue sweep.
- **Layer.** Technique crate (sans-IO adapter over `impl Read + Seek`). **Lines** 5,467 across
  9 files. **Pub items** 124.
- **Who calls it.** `argos_engine/src/pipeline.rs`, `finding.rs`, `config.rs`. **What it calls.**
  `argos_core::geometry`, `crc32fast`.
- **Reachable from.** CLI, GUI, `tests/recovery.rs` (1,099 lines), 6 fuzz targets.
- **Destination.** 8 files; `bytes.rs` merges into `lib.rs`.
- **If it vanished.** C10, C11, C12, C16 — all filesystem-aware recovery.
- **Findings.**
  - `bytes.rs` (88) → `MERGE` into `lib.rs` (§6.4). Seven `redundant_pub_crate` hits clear with it.
  - One file per on-disk format, each 284–1,535 lines, each with a fuzz target: this is what
    `A-FUZZ-EVERY-PARSER` and `M-BALANCED-MODULES` both want. `KEEP` all six.
  - Four `pub fn` in `fixture.rs` (`:358, :946, :1113, :1234`) are called only from `fixture.rs`
    itself → `NARROW`.
  - Constants with no external consumer in `ext4.rs:28`, `ntfs.rs:56`, `residue.rs:43,47` →
    `NARROW`.
  - `fixture.rs` at 1,535 lines is the largest fixture builder in the tree. It is *not* proposed for
    `SPLIT`: it builds five filesystem families and the split would be by format, mirroring the
    parser files exactly — a parallel structure this audit would then flag. Noted as Q6.
- **Confidence.** High.

### `crates/argos_carve/src`

- **Responsibility.** Signature carving, format validation, block classification and fragment
  reassembly, sans-IO.
- **Layer.** Technique crate. **Lines** 6,749 across 12 files. **Pub items** 149; 20 unnamed
  outside the crate's `src/`.
- **Who calls it.** `argos_engine/src/{pipeline,search,graft,lib}.rs`. **What it calls.**
  `argos_core`, `zune-jpeg`, `zune-png`, `miniz_oxide`, `memchr`, `crc32fast`.
- **Reachable from.** CLI, GUI, 9 test binaries, 2 benches, 2 examples, 10 fuzz targets.
- **Destination.** 10 files.
- **If it vanished.** C09, C13, C14, C15, C51 — all carving and reassembly.
- **Findings.**
  - `assemble.rs` (157), single consumer → `INLINE` (§6.6), and it is one half of the
    `A-ONE-IMPLEMENTATION` duplicate in §6.7.
  - `stream.rs` (175), three consumers all driven from `lib.rs` → `MERGE` into `lib.rs` (§6.5).
  - `jpeg.rs` (265) and `png.rs` (405): siblings on one subject, but both above the fragment
    threshold, both with their own fuzz target and their own state machine. `KEEP` separate. They
    look mergeable and are not: a PNG chunk walk and a JPEG marker grammar share no code.
  - Fourteen `pub` constants and three `pub fn` with no external consumer → `NARROW` (list in
    §4.5).
  - `mcu.rs` (1,456) and `reassemble.rs` (1,466) are large but each has one nameable subject. Not
    `SPLIT`.
- **Confidence.** High.

### `crates/argos_classify/src`

- **Responsibility.** Deterministic triage of decoded images, plus perceptual-hash dedup and
  standing.
- **Layer.** Technique crate / `Classifier` adapter. **Lines** 1,193 across 5 files. **Pub items**
  41; 11 unnamed outside the crate's `src/`.
- **Who calls it.** `argos_engine/src/{annotate,finding,pipeline}.rs`;
  `argos/src/{standing,export,serve/mod,serve/translate}.rs`.
- **Reachable from.** CLI, GUI, 6 test binaries, 1 bench, the `triage-thresholds` CI lane.
- **Destination.** 4 files.
- **If it vanished.** C24, C25, C27, C28 — labels, dedup and ordering.
- **Findings.**
  - `phash.rs` (119) → `MERGE` into `lib.rs` (§6.8).
  - `rank.rs` (176) `KEEP`: 7 cross-crate consumers, a distinct subject (evidence ordering, not
    photograph-vs-asset), and a test binary of its own.
  - `Triage` (`lib.rs:47`) is a fieldless struct with `new()` and a `decide` associated function —
    the classic "function wearing a costume". It is **not** a finding: it is the concrete type
    behind the `Classifier` port, `Classifier::score_batch` needs `&mut self`, and the type is what
    carries `ModelIdentity`. `KEEP`.
  - Six threshold constants with no external consumer → `NARROW`; each is required to be named and
    documented by `M-DOCUMENTED-MAGIC`, which says nothing about visibility.
  - `tests/probe.rs` (22) asserts nothing → `MERGE` into `tests/thresholds.rs` (§6.18).
- **Confidence.** High.

### `crates/argos_engine/src`

- **Responsibility.** *Two sentences are needed today, which is itself the finding:* it owns the
  staged scan pipeline and its concurrency; and, inside one 2,598-line file, it also owns the
  filesystem recovery driver, the reassembly search driver and the emit path. After §6.11 one
  sentence per file suffices.
- **Layer.** Core (the hexagon's centre). **Lines** 5,591 across 12 files. **Pub items** 73.
- **Who calls it.** `argos/src/{scan,graft,serve/mod}.rs`. **What it calls.** `argos_carve`,
  `argos_fs`, `argos_classify`, `argos_core`, `crossbeam-channel`, `sha2`.
- **Reachable from.** CLI, GUI, 5 test binaries.
- **Destination.** 12 files, reshaped (§6.9–6.11).
- **If it vanished.** C09–C29, C42–C47 — the whole pipeline.
- **Findings.**
  - `pipeline.rs` 2,598 lines, five responsibilities → `SPLIT` (§6.11). This is the mandatory
    counterweight and the single most valuable structural change in the engine.
  - `search.rs` (430) has exactly one code consumer, `pipeline.rs`; its apparent second consumer
    (`argos_carve/benches/replay.rs:49`) is a doc comment, and that bench does not link
    `argos_engine`. → move to `pipeline/region.rs`.
  - `error.rs` (80) → `INLINE` into `lib.rs` (§6.9).
  - `merge.rs` (209) + `cache_run.rs` (182) → `MERGE` into `finding.rs` (§6.10).
  - `ExtentReader` (`pipeline.rs:2303`) duplicates `argos_carve::assemble::Assembled` →
    `FLATTEN` (§6.7, §7.2). The strongest `A-ONE-IMPLEMENTATION` finding in the audit.
  - `ScanConfigBuilder` (`config.rs:233`): 8 fields, 7 chainable setters, a validating `build()`
    returning `Result<_, ConfigError>`. `A-NO-SPECULATIVE-SURFACE` exempts a builder at four or more
    construction permutations and `M-INIT-BUILDER`/`M-BUILD-RESULT` require this exact shape.
    `KEEP` — it looks like a Java builder and is the one the rules ask for.
  - `graft.rs` (212) `KEEP`: deliberately not a pipeline stage, because what it produces is pixels
    rather than files (C51's honesty constraint). Merging it into the pipeline would blur that.
  - `Box<dyn Error + Send + Sync>` at `error.rs:23` is the sink's error, erased at a port boundary
    where the concrete type genuinely varies. `KEEP`.
- **Confidence.** High for the split boundaries; medium for the exact line split of
  `pipeline/filesystem.rs` vs `pipeline.rs`, where `confirm_ntfs` could reasonably sit either side.

### `crates/argos_report/src`

- **Responsibility.** One output directory and the manifest that describes it.
- **Layer.** Infrastructure adapter (`ArtifactSink`). **Lines** 1,347 across 3 files. **Pub items**
  34.
- **Who calls it.** `argos/src/{scan,export,standing,console,serve/*}.rs`. **What it calls.**
  `argos_core::artifact`, `serde`, `serde_json`, `sha2`, `jpeg-encoder`.
- **Reachable from.** CLI, GUI, `tests/store.rs`.
- **Destination.** 2 files (§6.12).
- **If it vanished.** C30–C36 — the manifest, previews, ownership handback and hash-verified export.
- **Findings.**
  - `preview.rs` (209) and `handback.rs` (150), both single-consumer → `INLINE`.
  - `lib.rs` (988) holds two things at a clean seam → `SPLIT` out `manifest.rs`.
  - `Store::record_only` (`lib.rs:700`) and `Store::save_preview` (`lib.rs:798`) are pure forwarding
    wrappers with no caller but the trait impl → `NARROW`.
  - `impl AsRef<Path>` at `lib.rs:246,263,506` — same rule collision as `argos_device`. Noted only.
  - `argos_engine` does **not** depend on this crate. That is the correct hexagonal arrangement and
    the reason `argos_report` deserves to exist even with one consumer. `KEEP` the crate.
- **Confidence.** High.

### `crates/argos_ipc/src`

- **Responsibility.** The wire format between the engine process and its clients.
- **Layer.** Shared contract; depends on nothing in the workspace by design. **Lines** 1,113 across
  3 files. **Pub items** 41; 2 unnamed outside the crate.
- **Who calls it.** `argos/src/serve/*`, `argos_ui/src/{engine,commands,lib}.rs`.
- **Reachable from.** CLI (`argos serve`), GUI.
- **Destination.** 3 files, unchanged.
- **If it vanished.** C47, and C34–C41's wire surface.
- **Findings.**
  - `pub enum Message` (`wire.rs:309`) has no consumer → `REMOVE` (§7.1).
  - `Failure` (`wire.rs:238`) is constructed at `wire.rs:174` and held by `Outcome::Error`
    (`wire.rs:195`), which `argos_ui/src/engine.rs:15` imports. Reachable. `KEEP`.
  - The three files are 88 / 581 / 444 lines with distinct subjects (the contract's version, the
    data, the framing). No consolidation. `KEEP`.
  - The `bindings` feature has exactly one consumer — the CI lane that regenerates `dto.ts`. That
    is route 3 of `A-EARNS-ITS-PLACE`. `KEEP`.
- **Confidence.** High.

### `crates/argos/src`

- **Responsibility.** The CLI and the engine process: every capability, headless, plus `serve`.
- **Layer.** Driving adapter / composition root. **Lines** 4,806 across 17 files — the most
  fragmented crate in the workspace, and the only one where file count is clearly wrong.
- **Who calls it.** Nothing; it is the entry point. **What it calls.** `argos_engine`,
  `argos_device`, `argos_report`, `argos_classify`, `argos_core`, `argos_ipc`.
- **Reachable from.** CLI directly; GUI through `serve`; 4 test binaries (2,253 lines).
- **Destination.** 10 files (§6.13–6.17).
- **If it vanished.** Every line of the capability inventory. It is the surface.
- **Findings.**
  - Seven single-consumer fragments: `console.rs`→`main.rs`, `progress.rs`→`console.rs`,
    `scanlog.rs`+`invoker.rs`+`graft.rs`→`scan.rs`, `destination.rs`→`source.rs`,
    `standing.rs`→`export.rs`, `serve/wire.rs`→`serve/mod.rs`.
  - **Three duplicated implementations**, all of which the merges dissolve: the device-node
    predicate (§6.15), the artifact ordering (§6.16), the long-side calculation (§6.16). The
    ordering duplicate is not cosmetic — the two copies disagree on the tiebreak.
  - Two distinct traits both named `Notice` (`acquire.rs:149`, `scan.rs:70`). Both are legitimate
    mini-ports: `acquire::Notice` has three implementations (`console.rs:34`, `serve/mod.rs:592`,
    a test collector at `acquire.rs:179`), `scan::Notice` has two (`console.rs:16`,
    `serve/mod.rs:647`). Neither is a speculative abstraction; both `KEEP`. The shared name is a
    readability cost (`M-SHORT-NAMES` cuts both ways here) — after §6.13/§6.14 they live in
    `acquire.rs` and `scan.rs` respectively, which is where the disambiguation reads best.
  - `serve/mod.rs` is the tree's only `mod.rs` → rename to `serve.rs` (§6.17).
  - `main.rs` prints in four `run_*` functions while `console.rs:1-5` claims printing lives only
    there. The doc is currently false; §6.13 makes it true again.
  - `Arc<Mutex<Option<ScanSession>>>` (`serve/mod.rs:80`) and `Arc<Mutex<bool>>`
    (`serve/mod.rs:88`) genuinely cross threads — the dispatch loop and the pipeline's workers.
    `KEEP`.
- **Confidence.** High.

### Support targets

- **`tests/`**: 20 integration binaries, 8,432 lines. Three merges proposed (§6.18), all within one
  crate, none folding a test into a library.
- **`benches/`**: 3 targets, declared `harness = false` in the manifests, built by the `bench` CI
  lane (`cargo bench --workspace --no-run`). `KEEP`.
- **`examples/`**: 4 targets. Each carries an `A-SUPPORT-DECLARED` line naming what runs it and what
  it guarantees. Kept compiling by `cargo clippy --all-targets`. `KEEP`.
- **`fuzz/`**: 16 targets in 2 out-of-workspace crates. `.github/workflows/ci.yml` runs each by name
  for 120 s. Route 3 and route 4 of `A-EARNS-ITS-PLACE`. `KEEP` all.
- **Corpus directories**: 16 under `fuzz/*/corpus/`, seeded by the two `gen_fuzz_corpus` examples —
  authored seeds from a committed generator, exactly what `A-SUPPORT-DECLARED` requires. Not
  audited for content.

---

## 9. Architecture violations

No removal is proposed for anything in this section.

### A9.1 `docs/DEVELOPMENT-PLAN.md` §2.2's DAG no longer matches the code

The diagram declares `argos_engine ──▶ argos_report`. That edge **does not exist**:
`crates/argos_engine/Cargo.toml` depends on `argos_carve`, `argos_classify`, `argos_core`,
`argos_fs`, `crossbeam-channel` and `sha2`, and nothing else. The sink reaches the engine as
`argos_core::artifact::ArtifactSink`, injected by the binary. **The code is better than the
document**: the engine cannot name a report type, which is what keeps `ArtifactSink` a port rather
than a call.

Three edges exist that the diagram does not draw: `argos ──▶ argos_report`,
`argos ──▶ argos_classify`, `argos ──▶ argos_core`. All three are composition-root wiring — the
binary constructs the adapters and injects them — which is the correct hexagonal shape.

§2.2 says "any new edge must be justified against it", so the document is what needs updating, not
the graph. `ARCHITECTURE` — update §2.2 to the actual DAG:

```
argos ──▶ argos_engine ──▶ {argos_fs, argos_carve, argos_classify} ──▶ argos_core
argos ──▶ {argos_device, argos_report, argos_classify, argos_core, argos_ipc}
argos_ui ──▶ argos_ipc
```

The binding constraint — `argos_fs`, `argos_carve` and `argos_classify` never depend on each other
or on `argos_device` — **holds**, verified against every manifest.

### A9.2 `serve/translate.rs` computes, and its own doc says it does not

`crates/argos/src/serve/translate.rs:1-11` states "Nothing is computed here. A field is copied, or a
value is rendered with the `Display` the engine already defines. A calculation in this file would be
a second implementation of something the engine already decided (`A-SHELL-NO-DOMAIN`)."

`gallery` (`translate.rs:69-115`) filters by standing and sorts by four keys. The *placement* is
right — the ordering must live in the engine process rather than in the client, and
`translate.rs:60-63` says so explicitly — but the module doc contradicts it, and the calculation has
in fact become the second implementation the doc warns about (§6.16). `ARCHITECTURE` + `MERGE`: move
the ordering into `results::ordered`, called from both `console.rs` and `translate.rs`, and correct
the module doc.

### A9.3 Domain vocabulary reaches the CLI directly

`crates/argos/src/standing.rs:13,47` and `crates/argos/src/serve/mod.rs:376-378` call
`argos_classify::rank` from the driving adapter, bypassing `argos_engine`. This is not a violation —
the binary is the composition root and `standing.rs:5-7` cites `A-ONE-IMPLEMENTATION` as the reason
the derivation lives in one place — but it is worth recording that the ordering rule for a manifest
is a domain decision reachable from two adapters (`console`, `serve`) and from no engine entry
point. If a third client ever appears, `argos_engine` re-exporting `Standing` would be the better
home. **No change proposed now.**

### A9.4 The manifest schema is a shared contract with no crate of its own

`argos_report::{Manifest, ArtifactRecord, ExtentRecord, …}` are parsed by four modules in `argos`
and, indirectly, drive `argos_ipc::dto`. That is fine — but it means `argos_report` is both an
adapter (the `Store` sink) and a schema (`Manifest`). §6.12 separates them into two files inside the
crate, which is the right amount of separation; splitting them into two crates would be
`M-SMALLER-CRATES` taken past its usefulness, since nothing consumes the schema without also
consuming the crate. Recorded, not proposed.

---

## 10. Execution waves

Leaves to centre. Every wave leaves `cargo fmt --check`, `cargo clippy --workspace --all-targets
-- -D warnings` and `cargo test --workspace` green. Regime A first, in full, then Regime B against
the consolidated tree.

Baseline before wave 1: 140 `.rs` files, 42,599 lines, suite green (verified 2026-08-23).

**Acceptance criteria common to every Regime A wave:**
1. The three checks pass.
2. `git diff --stat` shows a net line change within ±30 of the wave's stated figure; a larger drop
   means code was deleted during a move.
3. `docs/CAPABILITIES.md` is unchanged and every line of it still true.
4. Every move used `git mv` where a file survives under a new name.

Revert per wave: `git revert --no-commit <wave-sha>..HEAD && git commit`, or
`git reset --hard <sha-before-wave>` on a branch not yet pushed.

| # | Wave | Touches | Net lines | Files |
| ---: | --- | --- | ---: | ---: |
| 1 | **`argos_core` root.** §6.1: dissolve `recovery.rs`; narrow two constants. | 1 crate, leaf of the graph | ≈0 | −1 |
| 2 | **`argos_device` media and inventory.** §6.2, §6.3: fold `mock`/`image`/`reader` into `device.rs`, `shadow` into `inventory.rs`; the `NARROW` list. | 1 crate | ≈0 | −4 |
| 3 | **`argos_fs` vocabulary.** §6.4: fold `bytes.rs` into `lib.rs`; the `NARROW` list. Clears 7 `redundant_pub_crate` hits. | 1 crate | ≈0 | −1 |
| 4 | **`argos_carve` cursor and assembler.** §6.5, §6.6: fold `stream.rs` into `lib.rs`, `assemble.rs` into `reassemble.rs`; the `NARROW` list *except* `Assembled`, which waits for wave 8. | 1 crate | ≈0 | −2 |
| 5 | **`argos_classify`.** §6.8: fold `phash.rs` into `lib.rs`; the `NARROW` list. Path change `argos_classify::phash::*` → `argos_classify::*` touches `argos_engine/src/annotate.rs`. | 2 crates | ≈0 | −1 |
| 6 | **`argos_report` seam.** §6.12: split `manifest.rs` out, fold `preview.rs` and `handback.rs` in, narrow the two forwarding wrappers. | 1 crate | ≈0 | −1 |
| 7 | **`argos_ipc`.** No Regime A work. Skipped. | — | 0 | 0 |
| 8 | **`argos_engine` reshaping.** §6.9, §6.10, §6.11, §6.7: inline `error.rs`; merge `merge.rs`+`cache_run.rs` into `finding.rs`; split `pipeline.rs` into four and move `search.rs` to `pipeline/region.rs`; replace `ExtentReader` with `argos_carve::assemble::Assembled`. **The largest wave — split it in two if the diff is unreviewable**: 8a is the three merges, 8b is the pipeline split plus the dedup. | 1 crate | −130 | ±0 |
| 9 | **`argos` binary.** §6.13–6.17, in this order: `medium.rs` first (it unlocks the predicate dedup), then `results.rs` (unlocks the ordering dedup), then `scan.rs`, then `console.rs`+`main.rs`, then `serve.rs`. Each is independently green. | 1 crate | ≈−10 | −7 |
| 10 | **Integration binaries.** §6.18: three test merges. | 3 crates' `tests/` | ≈0 | −3 |
| 11 | **Regime B.** §7.1 only (§7.2 executed in wave 8). Review item-by-item against the three anchors before touching anything. | `argos_ipc` | −15 | 0 |
| 12 | **Fixed-point visibility sweep.** Re-run the §4.5 procedure bottom-up on the consolidated tree, promoting what one pass names and repeating until it names nothing new. Apply the resulting `NARROW` list. No file moves. | all | ≈0 | 0 |

Wave 8b additionally requires: `cargo bench --workspace --no-run` (the bench lane compiles against
`argos_engine`'s doc-referenced paths), and a check that
`crates/argos_carve/benches/replay.rs:49,182` — which name `argos_engine::search::REGION_BYTES` and
`argos_engine::pipeline::in_parallel` **in doc comments** — are updated to the new paths so the
prose does not rot.

Wave 9 additionally requires `cargo test -p argos --test serve` and `--test cli` (2,253 lines of
integration coverage over exactly the code being moved), and a manual check that
`crates/argos/tests/characterization/manifest.snapshot.json` still matches.

---

## 11. Self-critique

**The module I understood worst** is `crates/argos_engine/src/pipeline.rs`. I mapped it by item
signatures and doc comments rather than by reading all 2,598 lines, and my proposed split boundaries
are inferred from that map. `confirm_ntfs` (`pipeline.rs:930`) in particular could belong either in
`pipeline/filesystem.rs` or in the orchestrator, and `Measure`/`Measured` (`pipeline.rs:1992,1999`)
are referenced from both the reassembly path and the emit path, so they may need to stay in
`pipeline.rs` rather than move to `pipeline/emit.rs`. The person executing wave 8b should treat my
line allocations as a starting proposal, not a specification.

**The finding most likely to be a false positive** is the `NARROW` list in §4.5. My name-level test
cannot see a type reached only through inference, and the compiler-verified probe was, by its own
limitation, a single non-fixed-point pass. I have flagged the constants and free functions
separately because those are safe, but I would not execute the type-level narrowing without wave 12
first. A second candidate: `argos_device/src/lib.rs:24` — `M-BALANCED-MODULES` may well be the
reason that re-export exists, in which case narrowing it is a regression in discoverability.

**The merge that risks making a file incoherent** is §6.13, `progress.rs` into `console.rs`. It
produces an 875-line file holding a `Notice` impl, a `ProgressSink` impl, four report-printing
functions and two threads that read the keyboard. I can name it — "the terminal user interface" —
but it is the widest span of any merged file in this plan, and a reviewer who thinks the keyboard
threads are a separate subject would be defensible. The fallback is to keep `progress.rs` and merge
only `console.rs` into `main.rs`, which still removes a file.

**The removal I would revert first** is §7.1, `argos_ipc::wire::Message`. Its evidence is complete
but it sits in the one crate whose stated purpose includes serving a client that does not exist yet,
and it costs 15 lines. If a client is written this quarter, removing it was churn.

**What I did not do.** I did not evaluate performance, allocation or algorithmic cost anywhere; the
one place I touched a performance-shaped file (`crates/argos_carve/tests/reassemble_scale.rs`) I
treated purely as a duplicate-named integration binary. I did not open any `argos_ui` source except
to grep four identifiers, and I read no frontend file. I did not run Miri or the fuzz lane.

---

## 12. Questions

Seven, each matching an `INVESTIGATE` only the author can resolve.

1. **`crates/argos/src/destination.rs:113`** narrows the device-node predicate to
   `cfg(target_os = "linux")` and to block devices only, while the two identical copies at
   `source.rs:223` and `acquire.rs:134` accept character devices on any unix. On macOS, `/dev/rdiskN`
   is a character device — does that mean `refuse_writing_onto_source` performs no device check
   there at all, and is that deliberate or a gap in `A-READ-ONLY`?

2. **`crates/argos/src/serve/translate.rs:97`** adds `.then(left.sha256.cmp(&right.sha256))` to the
   gallery ordering that `crates/argos/src/console.rs:453-457` does not have. Is the total order
   deliberate for the paged wire API (so two `gallery` calls agree), in which case `argos report`
   should gain it too — or did the two copies simply drift?

3. **`crates/argos_ipc::wire::Message` (`wire.rs:309`)** — is a third-party client of the JSON-RPC
   surface a real near-term expectation? If yes, §7.1 should be `KEEP` and the type should get a doc
   line saying who it is for; if no, it is the only Regime B item in the audit.

4. **`crates/argos_device/src/lib.rs:24`** re-exports `DeviceInfo` and `MountPoint`, which no
   consumer names. Is that re-export there for `M-BALANCED-MODULES` (essential items in the root),
   or is it residue? The two rules point opposite ways and only you know which was intended.

5. **`crates/argos_engine/src/pipeline.rs:1992,1999`** — are `Measure` and `Measured` used by both
   the reassembly path and the emit path? If so they stay in the orchestrator rather than moving to
   `pipeline/emit.rs`, and the §6.11 line allocation shifts by ~60 lines.

6. **`crates/argos_fs/src/fixture.rs` (1,535 lines)** builds five filesystem families in one file.
   Splitting it per format would mirror the parser files exactly — a parallel structure this audit
   would then flag as pattern 7. Do you consider the single fixture file the intended shape, or is
   the mirror the lesser evil here?

7. **`docs/DEVELOPMENT-PLAN.md` §2.2** declares an `argos_engine ──▶ argos_report` edge that does
   not exist and omits three that do (§9.1). Should §2.2 be corrected to the actual DAG, or was the
   edge dropped in an earlier change that the plan is meant to reinstate?
