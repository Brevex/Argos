# Argos Threat Model

This document captures the assumptions Argos relies on. Code that violates these assumptions is a defect.

## Assets

1. **Forensic integrity of the source device.** The source device is read-only from Argos's perspective. Modification of source contents would break the entire premise of the tool.
2. **Recovered artifacts.** Bytes recovered from the source must be faithfully written to the output, with cryptographic hashes proving they were not altered post-recovery.
3. **Audit log.** The session audit log is append-only and hash-chained. Tampering must be detectable.
4. **User privacy.** Recovered content, paths, and metadata are sensitive. They must not leave the user's machine and must not appear in logs.

## Trust boundaries

- **Source device → io::SourceDevice:** untrusted bytes, trusted to remain unmodified by Argos.
- **io → carve → validate → reassemble:** in-process, trusted.
- **bridge ↔ frontend:** typed IPC. The frontend is treated as semi-trusted — it can issue commands, but only against the configured scope.
- **frontend ↔ user:** the user authorizes paths and operations. The frontend cannot escalate privileges.
- **External world:** none. Argos performs no network I/O during forensic operations.

## Adversary capabilities

We assume an adversary may:

1. Supply a crafted disk image designed to exploit parsers (malformed JPEG/PNG headers, integer-overflow chunks, decompression bombs, deeply nested structures).
2. Have prior write access to the source device and have placed adversarial bytes intended to mislead carving.
3. Compromise the frontend (e.g., via a malicious npm dependency in development, or a renderer-process exploit) and attempt to escalate to filesystem write outside the configured output scope.
4. Read process memory or core dumps if they obtain local access.

We do not assume:

- Kernel-level adversary (out of scope for a userspace tool).
- Physical hardware tampering between Argos and the source (e.g., a malicious disk controller).

## Mitigations

| Risk | Mitigation |
|------|------------|
| Accidental write to source | `SourceDevice` handle does not implement `Write`. OS flags enforce read-only. |
| Source/output on same filesystem | Runtime check refuses to start. |
| Parser exploit | `proptest` for parsers; `cargo-fuzz` for pattern matching, Huffman, CRC. `panic = "abort"`. |
| Frontend escalation | `ScopedPath` on every path argument. Capabilities are explicit allow-lists. CSP forbids inline scripts. |
| Path traversal | `ScopedPath` rejects `..`, symlinks outside scope, absolute paths not under the scope root. |
| Logged sensitive data | `tracing` redaction layer at startup. Audit log is structured, not free-text. |
| Tampered audit log | Hash-chained entries. Final entry signed at session close. |
| Supply-chain | `cargo-audit`, `cargo-deny`, `cargo-vet` (long-term). Pinned lockfiles. No remote scripts in frontend. |
| Memory disclosure on crash | `panic = "abort"`. Crash records redacted. |

## Out of scope

- Anti-anti-forensics (detecting that an adversary has scrubbed a disk before Argos runs). Argos reports what is recoverable; it does not certify completeness.
- Cryptographic recovery (key recovery, encrypted-volume defeat). Out of scope; would require a separate threat model.
- Cloud / network forensics.

## Review cadence

This document is reviewed:
- Whenever a new dependency, FFI surface, `unsafe` block, or Tauri command is added.
- At each release.
- Whenever the algorithm spec changes.

Changes are tracked as ADRs.
