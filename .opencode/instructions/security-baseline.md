# Security Baseline

These rules cover dependencies, auditing, and the threat surface. They complement `forensics-rules.md` (which covers data integrity) and `tauri-patterns.md` (which covers IPC and frontend).

## Dependency hygiene

- All Rust dependencies are pinned to exact versions in `Cargo.lock`. The lockfile is committed.
- `cargo-deny` policy: deny advisories, deny unmaintained crates, deny duplicate semver-incompatible versions.
- `cargo-audit` runs in CI on every push.
- `cargo-vet` is the long-term goal; supply-chain review is documented per critical dependency.
- Preference: pure-Rust over `*-sys` crates (e.g., `rustls` over `openssl-sys`). C dependencies require a justification in the PR.
- No transitive dependency on `eval`-style code, no dynamic loading.

## Frontend dependencies

- Pinned via `frontend/package-lock.json`. Lockfile is committed.
- `npm audit` runs in CI; high-severity advisories block merge.
- No remote-loaded scripts. No CDN imports.
- Preference for small, audited libraries with active maintenance.

## Unsafe budget

- `cargo-geiger` reports per-module unsafe usage.
- New `unsafe` lines require an ADR. The budget is zero increment by default.
- Existing `unsafe` is documented structurally (see `rust-style.md` § Unsafe), not via comments.

## Threat model

The threat model is at `docs/threat-model.md`. Anchor assumptions:
- The attacker can supply an arbitrary disk image as input.
- The attacker can craft files designed to exploit parsers (malformed JPEG/PNG, integer overflows, decompression bombs).
- The frontend, once compromised, must not be able to escalate to filesystem write outside the configured output scope.
- Logs and crash reports never leave the machine without explicit user action.

## Crash safety

- The runtime sets `panic = "abort"` in release. Argos does not attempt to continue after a panic.
- Before abort, a redacted crash record is written to the session output directory: kind, module, offset within current operation, no payload data.

## Secrets

- The codebase contains no secrets, API keys, or credentials.
- If a future feature requires credentials (e.g., signing the audit log), they live in OS keystore (Linux Secret Service, Windows Credential Manager, macOS Keychain), accessed at runtime, never in source or config.

## Network

- Argos performs no network I/O during forensic operations. The bridge module rejects any command that would initiate outbound traffic.
- Update checks, if added, are explicit user actions, opt-in, and isolated from forensic state.

## Build provenance

- Release builds are reproducible to the extent the toolchain allows.
- Build artifacts are signed. Signing keys live in CI secret storage; never on developer machines.
- `tauri build` output is verified against a manifest of expected files.
