# Tauri and Frontend Patterns

These rules apply to everything in `src-tauri/`, the bridge module, and the TypeScript frontend.

## No comments

The no-comments rule from `AGENTS.md` applies in full to TypeScript, JSX/TSX, configuration code, and inline scripts. Names and types are the documentation.

## IPC boundary

- Every Tauri command is a thin shell over a typed function in `crate::bridge`. The command itself does no business logic.
- Commands accept and return strongly-typed structs derived with `serde`, never raw `String` or `serde_json::Value`.
- Path inputs from the frontend are rejected unless they fit a typed `ScopedPath` constructed against the configured allow-list. There is no path traversal escape.
- Payloads larger than 1 MiB never cross IPC as JSON. They flow through `tauri::ipc::Response` with raw bytes, or as a sequence of events.
- Progress is pushed from backend to frontend via events on a typed channel. The frontend never polls.

## Capabilities and CSP

- Capabilities are declared per window with the smallest possible surface. No `fs:default`, `shell:default`, or wildcard scopes.
- The CSP forbids inline scripts and remote sources outside `tauri://localhost` and `asset://`.
- `dangerousDisableAssetCspModification`, `withGlobalTauri: true`, and `devUrl` in production builds are forbidden.
- `__TAURI__` is not exposed to the global scope; use `@tauri-apps/api` imports.

## Filesystem and assets

- Recovered images are served via `convertFileSrc` and rendered with native `<img>` tags. They are never base64-encoded into IPC payloads.
- The frontend cannot read or write paths it did not receive from the backend; it does not construct paths.
- Output directories are chosen via the OS file dialog through the `dialog` plugin with explicit user consent per session.

## Frontend performance

- Lists with more than 100 items are virtualized (e.g., `@tanstack/react-virtual` or `solid-virtual`).
- Image previews are lazy-loaded and decoded off the main thread (`decoding="async"`, `loading="lazy"`).
- Heavy computation in the frontend (none expected, but if introduced) runs in a Web Worker.
- State updates from streamed events are coalesced; UI reactivity uses fine-grained primitives, not full re-renders.

## Frontend security

- All metadata derived from recovered files (EXIF, IPTC, XMP) is sanitized before render. `innerHTML` and `dangerouslySetInnerHTML` are forbidden.
- File names from recovered content are escaped before being inserted into the DOM.
- The frontend trusts no input from recovered files. Treat every byte as adversarial.
- Dependencies are pinned and audited. No transitive `eval`, no remote-loaded code.

## Errors at the boundary

- Backend errors are mapped to a discriminated union (`{ kind: "io" | "validation" | "denied" | ..., detail: ... }`) before crossing IPC.
- The frontend renders error kinds explicitly; it does not display backend strings verbatim if they could contain paths or hashes.

## Build

- Production builds enable `tauri.conf.json > build > beforeBuildCommand` with linting and type-checking as gates.
- `tauri build` runs in CI on a clean checkout. Local artifacts are not shipped.
