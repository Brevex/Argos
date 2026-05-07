# Forensic Integrity Rules

Argos handles user data of high sensitivity. The rules below are binding and override convenience.

## Read-only source

- Source devices are opened with the OS-specific read-only direct-I/O flags:
  - Linux: `O_RDONLY | O_DIRECT | O_NOATIME`
  - Windows: `GENERIC_READ` with `FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN`
  - macOS: `O_RDONLY | F_NOCACHE` set via `fcntl`
- The handle type used for source access does not expose any write method. Any function that writes to a source is a bug.
- Output goes to a different physical filesystem than the source. The runtime refuses to start otherwise.

## Chain of custody

- A SHA-256 hash is computed for the full source dump (when imaged) and for every recovered artifact.
- Operations are appended to a per-session audit log. The log is append-only, hash-chained (each entry includes the SHA-256 of the previous), and signed at session close.
- Audit log entries are structured: `{ timestamp, operation, source_descriptor, output_descriptor, byte_range, hash, status }`. No free-text fields.
- Output filenames embed the artifact's hash prefix to make accidental overwrites detectable.

## Bad sectors and partial reads

- Read errors at the device level (`EIO`, `ERROR_CRC`, `ERROR_SEEK`) are recorded with offset and length, then skipped. The recovery continues.
- A bad-sector map is part of the session output.
- Carving algorithms treat skipped regions as opaque: no fabricated bytes, no zero-fill that could be mistaken for valid content.

## Block-level operation

- Recovery operates on byte ranges and sector boundaries. Filesystem metadata (inodes, MFT entries, directory listings) is treated as a hint at most, never as truth.
- The carving stage is filesystem-agnostic. Filesystem-aware logic, if added later, lives in a separate module that produces hints consumed by the carver.

## Logging and redaction

- `tracing` logs may include: byte offsets, sector indices, operation timings, error kinds, dimensions of recovered images.
- `tracing` logs may **not** include: recovered file content, paths chosen by the user (only their session-scoped IDs), content hashes of recovered artifacts, EXIF/metadata extracted from recovered files.
- Log redaction is enforced by a `tracing` layer applied at startup. Bypassing the layer is a bug.

## No telemetry

- Argos performs no network I/O during forensic operations. The bridge module rejects any command that would initiate outbound traffic.
- Crash reports, if added, must be opt-in per session, scrubbed of session data, and submitted only to a user-specified endpoint.

## Output integrity

- Recovered artifacts are written to disk, hashed, and the hash is recorded in the audit log before any reference to the artifact is returned to the frontend.
- Once written, recovered files are not modified. Re-runs produce new files with new hashes.

## Threat-model anchors

The threat model lives at `docs/threat-model.md`. When adding a feature, confirm it does not violate the assumptions there. If it must, update the threat model in the same PR.
