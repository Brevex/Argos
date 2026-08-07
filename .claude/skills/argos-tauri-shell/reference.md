# Tauri shell — full rule text

Argos-specific rules (`A-*` namespace). Index: [../../guidelines-index.md](../../guidelines-index.md).

## The shell contains no domain logic (A-SHELL-NO-DOMAIN) { #A-SHELL-NO-DOMAIN }

<why>duplicated recovery logic in a UI drifts from the engine and turns the presentation layer
into a second, unreviewed implementation of forensics.</why>

`argos_ui` — its Rust side, its frontend, and any glue — contains no parsing, no carving, no
scoring, no filesystem knowledge, no confidence rules, no thresholds:

- A Tauri command translates a DTO to an engine/`--serve` call and maps the result back. Target
  shape: no branching beyond error mapping. A command that grows an `if` on domain data is moving
  logic across the boundary.
- Frontend TypeScript computes display formatting only (units, sorting the already-scored list,
  layout). If a number influences what is recovered, reported or ranked, it is engine code.
- The UI holds no state beyond a mirror of engine events and user view preferences.

## Every capability is CLI-first (A-CLI-FIRST) { #A-CLI-FIRST }

<why>the CLI is the proof that the shell is a shell; a UI-only feature is domain logic hiding in
the presentation layer.</why>

Every user-reachable capability (scan, pause, cancel, list, export, report) works headless via the
`argos` CLI before any UI exposes it. UI changes that require new engine behaviour land as: engine +
CLI first, shell second — in the same or consecutive changes, never shell-only.

## IPC DTOs are versioned and opaque-free (A-DTO-VERSIONED) { #A-DTO-VERSIONED }

<why>the UI and an elevated engine process evolve separately; an unversioned or leaky wire format
breaks silently at the worst boundary to debug.</why>

- One module defines every IPC type; serde-serialized; a schema version field travels with every
  session handshake, and mismatches fail loudly at connect time.
- DTOs never re-export engine types (`M-DONT-LEAK-TYPES`) and never contain raw sector data or
  bulk image bytes. Previews and exports are files in the session output directory, reached
  through the Tauri asset-protocol scope limited to exactly that directory.
- The same DTO module serves the in-process adapter and the `--serve` JSON-RPC mode — one wire
  format, two transports.

## Progress is events, never polling (A-EVENTS-NOT-POLLING) { #A-EVENTS-NOT-POLLING }

<why>polling couples the UI to engine internals and turns long scans into busy loops; the engine
already owns a progress port.</why>

- The engine's `ProgressSink` port feeds Tauri events (or `--serve` notifications); the UI
  subscribes. Commands are verbs (start, pause, cancel, export) — none of them fetch progress.
- Elevation: the shell spawns `argos --serve` elevated (UAC manifest on Windows, `pkexec` on
  Linux, `osascript` on macOS) and bridges stdio to events. The GUI process never runs elevated;
  scanning a plain image file may run in-process without elevation.
