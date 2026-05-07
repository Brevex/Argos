---
description: Safety, FFI, and IPC review
agent: build
---

Run a safety check across the FFI and IPC surface:

1. Dispatch in parallel:
   - `@security-reviewer` over dependencies, `unsafe` usage, and threat-model anchors.
   - `@tauri-bridge-reviewer` over `src/bridge/`, `src-tauri/`, and frontend.
   - `@forensics-validator` over read-only access, custody, and logging.
2. Cross-check: any new `unsafe` block, any new dependency, any new Tauri command must appear in the report with explicit verdicts.
3. Output grouped by surface (deps, unsafe, FFI/IPC, forensic-integrity).

Verdict: PASS, NEEDS-CHANGES, or BLOCKED.
