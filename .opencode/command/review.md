---
description: Full multi-agent review of the current diff
agent: build
---

Run a full review of the current branch's diff against `main`.

Steps:

1. Snapshot the diff: `git diff main...HEAD --stat` and `git diff main...HEAD`.
2. Dispatch in parallel, each scoped to relevant paths:
   - `@rust-reviewer` over all changed `*.rs` files.
   - `@perf-auditor` over `src/carve/`, `src/validate/`, `src/reassemble/`, `src/io/`.
   - `@tauri-bridge-reviewer` over `src/bridge/`, `src-tauri/`, and the frontend tree.
   - `@forensics-validator` over the entire diff.
   - `@algorithm-verifier` over `src/carve/`, `src/validate/`, `src/reassemble/`.
   - `@security-reviewer` over `Cargo.toml`, `Cargo.lock`, `package.json`, `pnpm-lock.yaml`, `src-tauri/tauri.conf.json`, capability files, and any new `unsafe` blocks.
3. Consolidate findings into one report grouped by severity (BLOCKER, MAJOR, MINOR).
4. List any findings where two or more agents agreed — those are highest priority.
5. Final verdict: PASS, NEEDS-CHANGES, or BLOCKED. If BLOCKED, name the user guarantee or law that would be violated.

Do not edit code in this command. Output the report to the user.
