# ADR 0006 — No duplication, no dead code

- **Status:** Accepted
- **Date:** 2026-05-07

## Context

A codebase is a mental model. Duplication forks that model: two definitions of "the same thing" inevitably drift, and every reader must reconcile the divergence. Dead code is worse: it claims to be load-bearing but is not, occupying mental space without justifying it.

## Decision

In Argos, every line of code is integrated and reachable.

- Duplicated logic is refactored on the second occurrence, not the third.
- Functions, types, constants, and modules not reached from `main`, the bridge, a test, or a benchmark are deleted.
- Speculative abstractions (traits with one impl, generic parameters with one instantiation, configuration toggles never flipped) are dead code by another name and follow the same rule.
- `#[allow(dead_code)]` is forbidden in non-test code.
- `dead_code`, `unused_imports`, `unused_variables`, `unused_mut`, `unused_must_use`, `unreachable_code`, `unreachable_patterns` are denied at the crate level.
- TypeScript: `noUnusedLocals`, `noUnusedParameters`, ESLint `no-unused-vars` are denied.

## Rationale

- **Drift prevention.** Two copies of "the same" function will not stay synchronized under maintenance pressure. The second occurrence is the warning sign.
- **Cognitive load.** Code that exists must justify its existence by being reached. Otherwise it is noise that masks the real architecture.
- **Forensic context.** Argos handles sensitive data. Reading the codebase end-to-end must yield a coherent mental model; speculative or unreachable code obscures real behavior.

## Enforcement

CI gates:
- `cargo clippy --all-targets -- -D warnings` (denies the unused-* lints).
- `cargo machete` (unused dependencies).
- `cargo udeps` (unused crate features).
- A duplication check (e.g., `tokei`-based or a similarity scanner). PRs over a configured threshold are flagged.

Reviewer checklist (`@rust-reviewer`, `@tauri-bridge-reviewer`):
- New symbols must have a reachable use site.
- New functions are checked against existing ones for overlap.
- Speculative parameters and traits are challenged; "we'll need this later" is rejected.

## Consequences

- Adding a feature requires deleting the scaffolding once the feature is built; we do not leave stubs.
- Refactors will frequently delete code. This is expected and welcome.
- Public API in `lib.rs` re-exports only what tests, benchmarks, or the bridge consume; everything else is `pub(crate)` or narrower.
