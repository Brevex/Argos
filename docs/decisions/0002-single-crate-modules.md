# ADR 0002 — Single crate with modules

- **Status:** Accepted
- **Date:** 2026-05-07

## Context

Argos has multiple internal subsystems (raw I/O, carving, validation, reassembly, custody, bridge). A workspace with multiple crates would express boundaries via Cargo manifests. A single crate expresses them via module visibility.

## Decision

Argos is a single Rust crate. Subsystems are modules under `src/`. Visibility is controlled with `pub(crate)`, `pub(super)`, and module-private items. The Tauri integration may live in a sibling `src-tauri/` crate as required by Tauri's build, but the recovery logic itself is one crate.

## Rationale

- **Faster iteration.** No inter-crate compilation overhead during development.
- **Refactor freedom.** Moving items between modules is cheap; moving items between crates is expensive.
- **No premature boundaries.** The architecture is still settling. Hard crate boundaries lock in decisions we may want to revisit.
- **Simpler dependency graph.** One `Cargo.toml`, one set of features.

## Consequences

- We rely on `pub(crate)` and module discipline rather than the compiler refusing to compile cross-crate use.
- The `rust-reviewer` and `forensics-validator` agents must check visibility manually for newly added items.
- If the codebase grows past ~30k LOC and module boundaries blur, this ADR is revisited.
