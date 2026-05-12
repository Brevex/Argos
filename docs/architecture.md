# Argos Architecture

Argos is a single Rust crate with a Tauri 2 frontend. The architecture is module-driven, not crate-driven: boundaries are enforced by visibility (`pub(crate)`, `pub(super)`), naming, and ownership of types — not by separate compilation units.

## Pipeline

```
+------------+   +-----------+   +-------------+   +-------------+   +-----------+
|  io        |-->|  carve    |-->|  validate   |-->| reassemble  |-->| custody   |
|  (RO src)  |   |  (find)   |   |  (verify)   |   | (PUP+SHT)   |   | (hash+log)|
+------------+   +-----------+   +-------------+   +-------------+   +-----------+
                                                                            |
                                                                            v
                                                                      +-----------+
                                                                      |  bridge   |
                                                                      |  (Tauri)  |
                                                                      +-----------+
                                                                            |
                                                                            v
                                                                      +-----------+
                                                                      | frontend  |
                                                                      |  (TS/UI)  |
                                                                      +-----------+
```

Data flows in one direction: from device through the pipeline, out via the bridge to the frontend. The frontend never reaches back into recovery state directly; it consumes events.

## Modules

### `io/`

- Owns raw device access. The only place that opens source devices.
- Exposes `SourceDevice` (read-only, sector-aligned, direct I/O) and `OutputSink` (writable, ideally distinct filesystem).
- Provides typed handle constructors that pin the OS-specific flags. `SourceDevice` does not implement `Write`.
- Block-iterator API streams `&[u8]` of sector-aligned size.

### `carve/`

- Two submodules: `hdd/` and `ssd/`. A dispatcher selects based on detected device class.
- `ssd/`: Aho–Corasick over header/footer patterns. Streams sector-aligned input; produces `Candidate` artifacts identified by start offset and tentative end.
- `hdd/`: SmartCarving. Header detection produces seeds; PUP extends them; SHT decides fragmentation; format validators score continuations.
- Carving never allocates per fragment. Buffers are reused across the entire scan.

### `validate/`

- Format-specific structural validators. One submodule per supported format (`jpeg/`, `png/`).
- Each validator exposes a continuous score in [0, 1] and a boolean accept/reject for completed artifacts.
- Validators are pure functions over `&[u8]` slices (plus prior decoder state where relevant).

### `reassemble/`

- HDD: PUP driver that consumes seeds from `carve::hdd` and validators from `validate`. Maintains the invariant that each block is consumed by at most one file.
- SSD: linear assembly from header to footer; no reassembly across non-contiguous regions.

### `custody/`

- SHA-256 hashing of dumps and recovered artifacts.
- Append-only audit log with hash-chained entries.
- Bad-sector map.
- Output filename policy (hash-prefixed).

### `bridge/`

- The sole IPC/FFI surface.
- Each Tauri command is a thin shell over a typed function in this module.
- `ScopedPath` rejects unauthorized paths.
- Errors crossing IPC are mapped to a discriminated union before serialization.
- Progress is pushed via events on a typed channel.

### `error.rs`

- Crate-wide `ArgosError` enum derived with `thiserror`.
- One conversion per source error type. No catch-all `From<Box<dyn Error>>`.

## Threading model

- One `rayon` pool for CPU-bound work (carving, validation, reassembly). Constructed at startup with explicit thread count.
- One `tokio` runtime for the bridge layer (IPC, event channels). Tasks here are I/O-bound and short-lived.
- The two pools do not share work. Cross-pool communication is via `tokio::sync::mpsc` channels with bounded capacity.

## Lifetimes of long-running work

A recovery session is a typed `Session` value owned by the bridge. It holds:
- The `SourceDevice` handle.
- The `OutputSink` handle.
- The `AuditLog`.
- A cancellation token observed by the carving pipeline.

Cancelling a session aborts in-flight work and finalizes the audit log. No state escapes a cancelled session.

## Privileges

Argos refuses to run without administrator/root privileges. The pipeline opens raw block devices, which is unconditionally privileged on every supported OS. The strategy is one of native, declarative mechanisms with a single runtime guard:

- **Windows:** an embedded application manifest (`resources/windows/Argos.manifest`) declares `requestedExecutionLevel level="requireAdministrator"`. UAC prompts before the binary's entry point runs. `build.rs` embeds the manifest via `embed-manifest`.
- **Linux:** the `.desktop` entry runs `pkexec /usr/bin/argos`. The Polkit action `com.argos.run` is packaged at `/usr/share/polkit-1/actions/com.argos.policy` with defaults `auth_admin_keep`, so the operator authenticates once per session.
- **macOS:** unsupported in this release. The `elevation` module emits a `compile_error!` on macOS targets. See ADR 0009.

The runtime guard lives in `src/elevation/` and is the first thing `main()` does. If the process is not elevated, the Linux backend re-execs through `pkexec`; the Windows backend exits with an explicit error because the manifest should have triggered UAC. Either way, no Argos code that touches a device runs unelevated.

## What does not exist (and won't, without an ADR)

- A plugin system.
- Filesystem-aware recovery (NTFS MFT walking, ext4 journal scanning) as a primary path. May be added later as optional hint sources, not as the trust root.
- Network access of any kind.
- A configuration file format with comments. Configuration is structured TOML, parsed into typed structs.
- macOS support. Deferred by ADR 0009; will be reintroduced when a SMJobBless or `osascript` trampoline is designed against Gatekeeper/notarization constraints.
