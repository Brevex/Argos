# ADR 0009 — Privilege elevation strategy

- **Status:** Accepted
- **Date:** 2026-05-12

## Context

Argos performs raw block-device I/O on `/dev/sd*`, `\\.\PhysicalDriveN`, and similar paths. On every supported platform these objects are protected by both discretionary access control (DAC) and capability/privilege checks. Running Argos as an unprivileged user yields `EACCES`/`ERROR_ACCESS_DENIED` at the very first `open()` of the source device, regardless of how thoroughly the application is otherwise configured.

The previous attempt — a postinstall script that ran `setcap cap_sys_rawio+ep` on the Linux binary — was insufficient. `/dev/sd*` is `root:disk 0660`; `CAP_SYS_RAWIO` does not bypass DAC. Users observed Argos starting normally but failing silently when selecting a device. The Windows and macOS paths had no elevation mechanism at all, while release notes claimed otherwise.

This ADR fixes the gap and replaces the half-measure with a single, platform-idiomatic strategy.

## Decision

Argos is an administrator-only application. It refuses to run without root/administrator privileges on every supported platform. Elevation is requested via each platform's native, declarative mechanism. A single runtime guard in `main()` covers command-line invocations that bypass the declarative mechanism.

| Platform | Primary mechanism | Runtime guard behavior |
|---|---|---|
| Linux | `.desktop` entry runs `pkexec /usr/bin/argos`; Polkit action `com.argos.run` (defaults `auth_admin_keep`) | If `geteuid() != 0`, re-exec self through `pkexec` and exit with the child's status |
| Windows | Application manifest embedded at build time with `requestedExecutionLevel level="requireAdministrator"` — the OS prompts for UAC before the binary's entry point runs | If `IsUserAnAdmin()` reports false (manifest stripped or bypassed), the process exits with a clear error |
| macOS | Deferred — not supported in this release | Compile error: macOS targets are explicitly unsupported until a future ADR |

## Rationale

- **Native mechanisms over reinvention.** UAC, Polkit, and Authorization Services are the canonical ways to ask for elevation on each OS. Wrapping them in custom Rust logic adds bugs without adding value.
- **DAC and capabilities are both required for raw device access.** `pkexec` re-execs the binary as root, satisfying DAC, capabilities, and any future LSM (SELinux, AppArmor) checks that gate the relevant capabilities. `setcap` alone does not.
- **`auth_admin_keep` matches the operator workflow.** Forensic operators run multiple sessions; prompting once per shell session is the established UX (GParted, Disks). Per-launch prompts are friction without security gain in this context.
- **Windows manifest is the only path that triggers UAC before code runs.** Programmatic re-exec via `ShellExecute` with the `runas` verb is a fallback used when no manifest is embeddable (we do not have that constraint). Embedded manifest is strictly cleaner.
- **macOS deferral.** A correct macOS implementation requires either a SMJobBless privileged helper or an `osascript`-based trampoline. Both have non-trivial interactions with Gatekeeper, notarization, and the `.app` bundle structure. Shipping a half-working macOS path would repeat the mistake this ADR is correcting. macOS is tracked for a future ADR.

## Consequences

- A new module `src/elevation/` owns the platform dispatch. `main()` calls `elevation::ensure()` before constructing the Tauri builder.
- `build.rs` embeds `resources/windows/Argos.manifest` on Windows targets via the `embed-manifest` build dependency.
- `resources/linux/com.argos.policy` is shipped as a packaged resource and installed by `scripts/postinst.sh` to `/usr/share/polkit-1/actions/`.
- `resources/argos.desktop` uses `Exec=pkexec /usr/bin/argos` so launching from the application menu triggers the Polkit prompt automatically.
- The previous `setcap` postinstall step is removed. Mixing `setcap` and `pkexec` would leave a confusing residue; the binary now runs as root and needs no additional capabilities.
- macOS targets fail compilation with an explicit error referring to this ADR. The `release.yml` workflow continues to build only Linux and Windows artifacts.
- `docs/architecture.md` gains a Privileges section. `docs/threat-model.md` updates the "frontend escalation" row: the frontend now runs inside an already-privileged process, so the `ScopedPath` allow-list and capability surface remain the only barrier between a compromised renderer and arbitrary writes.

## Alternatives considered

- **`setcap cap_sys_rawio,cap_dac_read_search+ep`**: would let the binary read raw devices as an unprivileged user. Rejected because (a) it leaves `/dev/sd*` DAC bypass as an attribute of every Argos binary, harming defense in depth; (b) capability-aware tooling still requires the user to be in the `disk` group on many distributions; (c) it does not generalize to Windows or macOS.
- **Setuid root binary**: classic POSIX elevation. Rejected because setuid binaries inherit a user environment that has been exploited countless times (LD_PRELOAD, etc.). Polkit/pkexec sanitize the environment for us.
- **`gksu`/`kdesu` wrappers**: deprecated; not present on modern distributions. Polkit is the standard.
- **Programmatic UAC re-exec on Windows**: works but produces a visible "child process" UI flash and complicates STDOUT inheritance. Embedded manifest avoids both.
