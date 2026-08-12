---
name: argos-tauri-shell
description: Use BEFORE writing or reviewing anything in `argos_ui` or the `--serve` mode of the CLI — Tauri commands, IPC DTOs, events, frontend code, capability/allowlist config, or the elevated engine-process bridge. Covers A-SHELL-NO-DOMAIN, A-CLI-FIRST, A-DTO-VERSIONED, A-EVENTS-NOT-POLLING.
---

# Tauri shell

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.
Tauri is a presentation shell over the engine and nothing more; the architecture makes this
structural — the UI is a client of the same `--serve` JSON-RPC interface any other client uses.

## Decision layer

- `A-SHELL-NO-DOMAIN` — `argos_ui` (Rust and frontend alike) contains zero recovery logic: no
  parsing, no scoring, no filesystem knowledge, no thresholds. A Tauri command is DTO-in →
  engine call → DTO-out with error mapping, nothing else. A value computed in TypeScript is
  display formatting only.
- `A-CLI-FIRST` — every capability ships in the headless CLI before it appears in the UI. If a
  feature only works through the UI, the boundary is broken; reject the change.
- `A-DTO-VERSIONED` — all IPC types live in one DTO module, serde-serialized, carrying a schema
  version field. DTOs never leak engine types (`M-DONT-LEAK-TYPES`) and never carry raw sector
  data; previews travel as files via the scoped asset protocol, not base64 over IPC.
- `A-EVENTS-NOT-POLLING` — progress flows engine → UI through events fed by the `ProgressSink`
  port; commands never poll. The application elevates itself before drawing anything, on all three
  platforms, and the engine inherits that; there is no unprivileged mode and no flag to get wrong.
  Because the scan then writes as the administrator, the output is handed back to the account that
  asked, and a failure to hand it back is said out loud.
- Tauri capabilities stay minimal: the command set and one asset-protocol scope restricted to the
  session output directory — no shell, no fs, no http exposure to the webview.

## Full rule text

[reference.md](reference.md) — `A-SHELL-NO-DOMAIN`, `A-CLI-FIRST`, `A-DTO-VERSIONED`,
`A-EVENTS-NOT-POLLING`.
