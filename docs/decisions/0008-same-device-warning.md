# ADR 0008 — Warning instead of prohibition for same-device output

- **Status:** Accepted
- **Date:** 2026-05-12

## Context

Previous architecture documentation and threat-model assumptions stated that Argos should refuse to start when the output directory resides on the same filesystem as the source device. This was intended to prevent users from accidentally overwriting recoverable data.

## Decision

Argos will **not** refuse to start when source and output share a filesystem. Instead, it will detect the condition at session start and surface a non-blocking warning to the user. The backend returns the warning in the `StartResponse` payload; the frontend renders it as a dismissible banner and allows the session to proceed normally.

## Rationale

- User autonomy: a forensic operator may have legitimate reasons to write output to the same physical device (e.g., a separate partition or an emergency acquisition scenario).
- Non-blocking flow: the tool must remain usable in constrained environments where an external output disk is not available.
- The warning is sufficient to inform the user of the risk without enforcing a hard policy that could block valid workflows.

## Consequences

- The `StartResponse` bridge type now carries an optional `warning` field.
- `io::OutputSink` no longer performs (and never performed) a same-filesystem runtime check; the check lives in `bridge::commands` as a heuristic.
- Frontend components must render the warning banner and allow dismissal.
- Documentation (`docs/architecture.md` and `docs/threat-model.md`) is updated to reflect the advisory nature of the check.
