# Argos — Agent Operating Manual

Argos is a desktop forensics tool that recovers permanently-deleted image files from storage devices. This document is the entry point for any AI agent working on this codebase. Read it in full before touching code.

## Fundamental Law

> Computer science and software engineering must always be respected. Bad practices — especially in low-level code — are abolished without exception.

If you cannot uphold this law for a given change, stop and ask. Do not approximate.

## Stack

- **Backend:** Rust, single crate with modules. MSRV pinned in `rust-toolchain.toml`.
- **Frontend:** Tauri 2.x with TypeScript (strict mode).
- **No workspace.** Modules under `src/` express the architecture; do not split into sub-crates without an ADR.

## Module Layout (target)

```
src/
├── main.rs              — binary entry, wires bridge to runtime
├── lib.rs               — re-exports the public surface for tests/bench
├── io/                  — raw device access, read-only, sector-aligned
├── carve/               — carving engines (hdd/, ssd/), dispatch by device class
├── validate/            — Huffman validation, CRC32, format-specific checks
├── reassemble/          — PUP + SHT for HDD; trivial linear for SSD
├── custody/             — hashing, audit log, chain-of-custody artifacts
├── bridge/              — Tauri command handlers; sole FFI/IPC boundary
└── error.rs             — crate-wide error enum (thiserror)
```

## Inviolable Rules

### Code documentation

**No comments. Anywhere. Ever.** No `//`, no `/* */`, no `///` doc-comments. The code is the documentation.
- If a block needs explanation, extract a named function or introduce a newtype.
- Architectural rationale belongs in `docs/`, not in source.
- This rule applies to Rust, TypeScript, and shell scripts. Configuration files (TOML, JSON) are exempt because comments there are inert.

### No duplication, no dead code

Every line of code in Argos must be load-bearing and integrated. Two principles:

1. **No duplication.** If the same logic appears in two places, it is a bug. Refactor on the second occurrence, not the third. Search before writing: a similar function may already exist. "Almost the same" with a different parameter is still duplication — generalize.
2. **No dead code.** If a symbol is not reached from `main`, the bridge, or a test, it does not belong in the codebase. Delete it.
   - `#[allow(dead_code)]` is forbidden in non-test code.
   - `dead_code`, `unused_imports`, `unused_variables`, `unreachable_code` are denied at the crate level.
   - `cargo-machete` (unused dependencies) and `cargo-udeps` are CI gates.
   - "We'll need this later" hooks are dead code by another name. Add them when you need them.
   - Public API in `lib.rs` re-exports only what tests, benchmarks, or the bridge actually consume.
   - TypeScript: `noUnusedLocals`, `noUnusedParameters`, `noFallthroughCasesInSwitch` are strict; ESLint `no-unused-vars` is denied.

Duplicate detection is run in CI (e.g., `tokei` plus a similarity check) and any flagged hotspot blocks merge until refactored or justified.

### Forensic integrity

- Source devices are opened **read-only** with direct I/O (`O_RDONLY | O_DIRECT` on Linux, `FILE_FLAG_NO_BUFFERING` on Windows). Output goes to a separate filesystem.
- Every dump and every recovered artifact is hashed (SHA-256). Hashes and operations are logged to an append-only audit log.
- Bad sectors (`EIO`) are recorded and skipped; never abort a recovery on them.
- Operate at block level. Do not trust filesystem metadata for recovery decisions.

### Performance discipline

Forbidden in hot paths (carving, validation, matching, reassembly):
- `Vec` reallocation in loops without `with_capacity` or buffer reuse.
- `clone()` where `&` or `Cow<'_, T>` would suffice.
- `String`/`PathBuf` where `&str`/`&Path`/`&[u8]` work.
- `Box<dyn Trait>` in tight loops where generics monomorphize.
- Unbuffered or unaligned I/O against raw devices.
- `async` for CPU-bound work (carving runs on `rayon`, not `tokio`).
- `Mutex` where `RwLock`, `parking_lot`, atomics, or sharding fit better.
- Per-fragment heap allocation (use `bytes::BytesMut`, arena, or pooled buffers).

### Safety discipline

- `unsafe` is allowed only when no safe alternative exists at the same performance level. Wrap each `unsafe` block in a function whose **name and type signature encode the invariant** (e.g., `read_into_aligned<const N: usize>(buf: &mut AlignedBuf<N>)`). No `// SAFETY:` comments — refactor until the invariant is structural.
- No `unwrap()`, `expect()`, or `panic!` outside tests and proven-by-construction invariants.
- Errors via `thiserror` (`ArgosError`). The binary may use `anyhow` only at the top level.
- Inputs from the bridge are validated against a typed scope; raw `String` paths from the frontend are rejected.

### Tauri / frontend rules

- IPC payloads >1 MiB go through `tauri::ipc::Response` with raw bytes or stream events; never JSON-serialize large buffers.
- Progress is pushed via events. The frontend never polls the backend.
- Capabilities are explicit allow-lists. No `fs:default`. CSP is strict; no `dangerousDisableAssetCspModification`.
- Lists of fragments/results are virtualized in the UI.
- Recovered images are served via `convertFileSrc` / `asset://`, not loaded into JS memory.
- EXIF and metadata are sanitized before render. No `dangerouslySetInnerHTML`.

### Logging and privacy

- `tracing` with structured fields. **Never log:** recovered content, user-supplied absolute paths, content hashes of recovered artifacts, or anything derivable from user data.
- Logs are local, rotated, and redacted. No telemetry.

### Errors and panics

- Public functions return `Result<T, ArgosError>`.
- `panic!` is reserved for invariants that, if violated, mean memory corruption or programmer error — never for user-input failures.

## Workflow

- `cargo clippy --all-targets -- -D warnings` must pass.
- `cargo test` and `cargo test --release` for hot-path code.
- `cargo bench` for any change touching `carve/`, `reassemble/`, or `validate/`.
- `cargo fmt --check`, `cargo deny check`, `cargo audit`, `cargo machete`, `cargo udeps` are CI gates.
- `pnpm tauri dev` for frontend; `pnpm typecheck` and `pnpm lint` are gates.

## Modular Instructions

For domain-specific rules, agents read these (loaded via `opencode.json`):

- `.opencode/instructions/rust-style.md` — allocation, unsafe, parallelism, testing, dead-code policy.
- `.opencode/instructions/tauri-patterns.md` — IPC, capabilities, frontend.
- `.opencode/instructions/forensics-rules.md` — read-only access, custody, bad sectors.
- `.opencode/instructions/algorithms-spec.md` — PUP, SHT, Huffman validation, Aho–Corasick contracts.
- `.opencode/instructions/security-baseline.md` — dependencies, threat-model anchors.

## Decisions

Architectural decisions are recorded as ADRs under `docs/decisions/`. If a change contradicts an ADR, write a new ADR superseding it — do not silently diverge.
