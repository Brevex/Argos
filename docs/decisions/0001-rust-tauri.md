# ADR 0001 — Rust backend with Tauri frontend

- **Status:** Accepted
- **Date:** 2026-05-07

## Context

Argos performs raw block-device I/O, parsing of adversarial inputs, and CPU-intensive pattern matching. It also needs a desktop GUI.

## Decision

- Backend: Rust.
- Frontend: Tauri 2.x with TypeScript.

## Rationale

- **Rust backend:** memory safety without GC, predictable performance, strong type system to encode forensic invariants (read-only handles, scoped paths, sector-aligned buffers). Mature ecosystem for FFI, raw-device access, and pattern matching (`aho-corasick`, `bytes`, `rayon`).
- **Tauri over Electron:** Tauri uses the OS WebView, eliminating the bundled Chromium that Electron ships. Smaller binaries, lower memory footprint, smaller attack surface, capability-based IPC. The cost (less uniform WebView behavior across platforms) is acceptable for a desktop forensic tool.
- **TypeScript frontend:** typed boundary against the bridge; standard tooling.

## Consequences

- We must test on each target WebView (WebKit on macOS/Linux, WebView2 on Windows).
- We commit to maintaining typed schemas for all IPC.
- We avoid Node-style libraries that assume a full browser environment in the renderer.
