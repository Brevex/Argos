---
description: Performance audit of hot paths
agent: build
---

Run a performance audit:

1. Dispatch `@perf-auditor` over `src/carve/`, `src/validate/`, `src/reassemble/`, `src/io/`.
2. Run `cargo bench -- --save-baseline current` if benchmarks exist.
3. Inspect `cargo asm` output for any function flagged as a hot path that is not inlined or is calling a virtual dispatch.
4. Report:
   - Antipatterns found, with file:line and the rule from `rust-style.md`.
   - Benchmark numbers vs the `main` baseline (if available via `git stash` + checkout pattern).
   - Concrete fixes prioritized by expected impact.

Do not edit code.
