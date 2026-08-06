---
name: rust-workspace-setup
description: Use BEFORE creating, splitting or renaming a crate, editing any `Cargo.toml` or `.cargo/config.toml`, adding a dependency or cargo feature, setting edition/MSRV/lints/allocator/target-cpu, or wiring CI checks (clippy, rustfmt, cargo-audit, cargo-hack, cargo-udeps, miri). Covers M-CARGO-WORKSPACE, M-CRATES-FLAT-FOLDER, M-CRATES-IN-WORKSPACE, M-LATEST-EDITION, M-MSRV, M-SMALLER-CRATES, M-STATIC-VERIFICATION, M-LINT-OVERRIDE-EXPECT, M-UPSTREAM-GUIDELINES, M-FEATURES-ADDITIVE, M-OOBE, M-SYS-CRATES, M-MIMALLOC-APPS, M-TARGET-CPU.
---

# Workspace, crates, features and toolchain

Read [reference.md](reference.md) for the full rule text — in particular the **complete
`[lints.rust]` and `[lints.clippy]` tables under `M-STATIC-VERIFICATION`**, which are copied
verbatim there and must be applied as-is. This file is the decision layer.

## Layout

```
Cargo.toml            # workspace root: members, [workspace.package], [workspace.dependencies], [workspace.lints]
.cargo/config.toml    # target-cpu per platform
crates/
  argos/              # the CLI binary
  argos_core/         # domain types shared by everything
  …
```

- [ ] One workspace `Cargo.toml`; all crates are **flat siblings** under `crates/`
      (`M-CRATES-FLAT-FOLDER`). A crate inside another crate's directory — and above all inside its
      `src/` — is never acceptable. Relationships are expressed by name prefix (`argos`,
      `argos_core`, `argos_device`).
- [ ] Every crate is a workspace member **and** declared in `[workspace.dependencies]` with a
      version, so siblings depend via `foo.workspace = true`, never `path = "../foo"`
      (`M-CRATES-IN-WORKSPACE`).
- [ ] Shared metadata, dependency versions and lints are inherited from the root
      (`M-CARGO-WORKSPACE`). Crate-specific dependencies are still declared in the workspace, with
      `default-features = false`.
- [ ] `edition = "2024"` (`M-LATEST-EDITION`); `resolver` is not needed.
- [ ] Set an MSRV at creation and bump it conservatively, a few releases behind stable (`M-MSRV`).

## Split early, split often

`M-SMALLER-CRATES` — err toward too many crates: dramatically faster incremental builds and no
cyclic dependencies. If a submodule could stand on its own, it should be a crate. Losing
`pub(crate)` access during the split is a feature, not a problem — it forces a real abstraction.

Crates vs features: a **crate** is something usable on its own; a **feature** unlocks functionality
that cannot live alone.

## Features are additive

`M-FEATURES-ADDITIVE` — every combination must compile:

- [ ] No `no-std` feature — use a `std` feature instead
- [ ] Enabling a feature never removes or changes a public item (adding variants is fine only on
      `#[non_exhaustive]` enums)
- [ ] No feature depends on the user manually enabling another
- [ ] Verify with `cargo hack --feature-powerset check`

## It must just work

`M-OOBE` — `cargo build` with nothing but a Rust toolchain, on all Tier 1 platforms. No required
external tools, no env vars, no vendored build scripts. Platform-specific dependencies go behind
`cfg` or opt-in features, with a HAL module and a dummy fallback for not-yet-supported targets.
`-sys` crates follow `M-SYS-CRATES`: build via the `cc` crate from `build.rs`, embed and verify the
upstream sources, never invoke Makefiles.

## Static verification (check-in gate)

Apply the lint tables from `reference.md` verbatim in `[workspace.lints]`, then run:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo audit          # vulnerable dependencies
cargo hack --feature-powerset check
cargo udeps          # unused dependencies
cargo +nightly miri test -p <crate with unsafe>
```

Lint overrides use `#[expect(lint, reason = "…")]`, never `#[allow]` (`M-LINT-OVERRIDE-EXPECT`) —
an `expect` that stops being needed warns, so the lint set never rots. `#[allow]` stays legal only
inside generated code and macro expansions.

`M-UPSTREAM-GUIDELINES` also applies: the Rust API Guidelines are in force on top of these rules —
especially `C-CONV` (`as_`/`to_`/`into_`), `C-GETTER`, `C-COMMON-TRAITS`, `C-CTOR` (have `new()`
even when `default()` exists) and `C-FEATURE`.

## Application-only settings

These apply to the `argos` binary, never to the libraries:

- `mimalloc` as `#[global_allocator]` (`M-MIMALLOC-APPS`) — up to ~25% on allocation-heavy paths,
  which a carving pipeline is.
- Highest guaranteed `target-cpu` in `.cargo/config.toml` (`M-TARGET-CPU`); relevant here because
  signature scanning benefits from wide SIMD. Weigh it against the range of machines an examiner
  might run Argos on — `x86-64-v3` excludes older hardware.

## Argos application

Proposed split (revisit as features land):

| Crate | Role |
| --- | --- |
| `argos` | CLI binary: arg parsing, output, `mimalloc`, may use `anyhow` |
| `argos_core` | Domain newtypes (`Lba`, `ByteOffset`, `SectorSize`), shared errors |
| `argos_device` | Raw device access + HAL per OS; the only crate with `unsafe` |
| `argos_carve` | Signature-based carving over `impl Read + Seek`, sans-IO |
| `argos_fs` | Filesystem metadata parsing (NTFS/ext4/APFS), one crate per FS if it grows |
| `argos_report` | Findings, manifests, hashes, chain-of-custody output |

`argos_device` is the only place that may depend on `libc`/`windows`; everything above it stays
platform-neutral and testable.

## Full rule text

[reference.md](reference.md) — 14 rules, listed in the file header.
