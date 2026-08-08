# Argos

Forensic recovery of permanently deleted images from block devices (HDD/SSD/NVMe), written in Rust.
Correctness and memory safety outrank features: this tool reads evidence, and a wrong answer is
worse than no answer.

## The standard

This project codes to a fixed set of **89 `M-*` rules**, adapted from the Pragmatic Rust Guidelines
(Microsoft, MIT). Their text lives in `.claude/skills/<skill>/reference.md`, one rule to exactly one
file — **those files are the standard**, there is no external document behind them. The map is
[.claude/guidelines-index.md](.claude/guidelines-index.md).

**Invoke the relevant skill before writing the code, not after.** Each skill carries the rule text
plus how it applies to this project. When a rule is cited in a review, a commit or a discussion,
cite it by id (`M-UNSOUND`, `M-HOTPATH`).

## Routing

| When you are about to… | Invoke |
| --- | --- |
| create/split a crate, touch `Cargo.toml`, add a dependency or feature, set lints | `rust-workspace-setup` |
| name a type/fn/module, decide `pub`, add a `pub use`, lay out a crate root | `rust-api-surface` |
| define a struct/enum/trait, a public signature, a constructor or builder | `rust-type-design` |
| write anything returning `Result`, an error type, or any `panic!`/`unwrap`/`expect` | `rust-errors-panics` |
| write `unsafe`, a raw pointer, a syscall/ioctl, FFI, or wrap a native handle | `rust-unsafe-ffi` |
| write a scan loop, buffer handling, a collection, or a hot data structure | `rust-performance` |
| add threads, rayon, channels, `async`, shared state, or a `static` | `rust-concurrency` |
| write a test, a fixture, or a type that performs I/O | `rust-testing` |
| write `///` or `//!` docs, or introduce a constant | `rust-docs` |
| emit a log, metric or progress event | `rust-telemetry` |
| reach for a macro | `rust-macros` |
| open a device/image, parse bytes from a medium, record or report an artifact | `argos-evidence-handling` |
| write a parser of on-disk structures, or its fuzz target/fixtures | `argos-ondisk-parsing` |
| implement or change a recovery technique (carving, FS recovery, reassembly) | `argos-recovery-algorithms` |
| touch `argos_classify`: model, thresholds, inference, dedup | `argos-ml-triage` |
| touch `argos_ui` or the `--serve` IPC surface | `argos-tauri-shell` |

Reviews are on demand, read-only, and cite rule ids: `rust-design-reviewer`, `rust-safety-reviewer`,
`rust-perf-reviewer`, `rust-test-reviewer`, `forensic-boundary-reviewer` (the `A-*` rules).

The development plan — target architecture, algorithm pipeline, phases P0–P9 with per-phase skills,
reviewers and exit criteria — is [docs/DEVELOPMENT-PLAN.md](docs/DEVELOPMENT-PLAN.md).

## Non-negotiables

These hold without consulting a skill:

1. **No unsound code, ever** (`M-UNSOUND`). The only rule in the book with no exceptions: safe-looking
   code must not be able to cause UB from safe callers, not even theoretically. If it cannot be
   encapsulated safely, expose an `unsafe fn` with a documented contract instead.
2. **`unsafe` needs a written reason** (`M-UNSAFE`) — novel abstraction, benchmarked performance, or
   FFI. Every block carries plain-text safety reasoning; every `unsafe fn` carries a `# Safety`
   section. Never as a shortcut.
3. **A panic means stop the program** (`M-PANIC-IS-STOP`, `M-PANIC-ON-BUG`). Bugs panic with a
   message containing the actual values; anything the caller could act on is a `Result`. Bad sectors,
   truncated images and unknown filesystems are `Result` — corruption is the expected condition here.
4. **Never write to the source medium.** Devices are opened read-only at the lowest layer; no
   write path exists to be reviewed.
5. **Disk images are untrusted input.** Never trust a length, offset or count read from the medium:
   no unchecked arithmetic, allocation or indexing derived from it.
6. **No `println!`/`dbg!` outside the CLI** (`M-LOG-NOT-PRINT`). Libraries emit named structured
   events, and never log recovered content, filenames or identifying paths (`M-LOG-STRUCTURED`).
7. **Lint overrides are `#[expect(lint, reason = "…")]`**, never `#[allow]` (`M-LINT-OVERRIDE-EXPECT`).
8. **Document the end state, never the design journey** (`M-NO-META-DESIGN-DOCUMENTATION`). No
   "why we chose X" essays and no "guidelines applied ✅" tables in code, docs or commit messages.

## Layout

```
crates/argos/          CLI binary — stdout is its UI; also the engine process (`--serve`);
                       may use one app-error crate (anyhow/eyre)
crates/argos_core/     domain newtypes (Lba, ByteOffset, SectorSize, DeviceClass, Confidence),
                       port traits (BlockSource, …) and canonical errors shared across crates
crates/argos_device/   BlockSource adapters: per-OS HAL (Linux/Windows/macOS), device
                       enumeration and mount state, image files, acquisition — the only
                       crate allowed to contain unsafe. Path conventions and device-class
                       decisions live in modules compiled on every target, so they are
                       tested everywhere; only syscalls sit behind cfg
crates/argos_fs/       partition tables + filesystem metadata recovery (NTFS/ext4/FAT/APFS)
                       + prior-filesystem residue scan
crates/argos_carve/    signature carving, block classification and fragment reassembly over
                       `impl Read + Seek` (sans-IO, testable in memory)
crates/argos_classify/ ML triage (pure-Rust inference) + perceptual-hash dedup
crates/argos_engine/   scan pipeline, session lifecycle, concurrency, finding merge, confidence
crates/argos_report/   findings, manifests, hashes, chain of custody
crates/argos_ui/       Tauri presentation shell — no recovery logic
```

Library crates use canonical error structs (`M-ERRORS-CANONICAL-STRUCTS`); only the binary may use
an application error crate (`M-APP-ERROR`). The dependency DAG in
[docs/DEVELOPMENT-PLAN.md](docs/DEVELOPMENT-PLAN.md) §2.2 is binding: `argos_fs`, `argos_carve`
and `argos_classify` never depend on each other or on `argos_device`; they meet in `argos_engine`.

## Checks

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo +nightly miri test -p argos_device     # whenever unsafe changes
```

The lint tables to apply live verbatim under `M-STATIC-VERIFICATION` in
`.claude/skills/rust-workspace-setup/reference.md`.
