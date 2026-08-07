---
name: argos-evidence-handling
description: Use BEFORE writing any code that opens a device or image, interprets bytes read from a medium, records a recovered artifact, or reports recovery results — and before running any shell command that names a block device. Covers A-READ-ONLY, A-UNTRUSTED-ONDISK, A-PROVENANCE, A-CONFIDENCE-HONEST, A-NO-CONTENT-IN-LOGS.
---

# Evidence handling

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.
Argos reads evidence: a wrong answer is worse than no answer, and the source medium is
irreplaceable. These rules exist so neither is ever put at risk.

## The two absolutes

- `A-READ-ONLY` — no code path may open a source medium writable, and no API for writing to a
  `BlockSource` may exist. Output always goes to a destination the user chose. This is structural:
  the port trait has no write method, so a violation cannot compile.
- `A-UNTRUSTED-ONDISK` — every length, offset, count, or flag read from the medium is attacker-
  controlled until proven otherwise. Concretely:
  - arithmetic on medium-derived values uses `checked_*`/`saturating_*`, never bare `+`/`*`;
  - indexing/slicing uses `get()`/`get_mut()`, never `[]`, when the index derives from the medium;
  - allocations are capped by a named constant independent of on-disk values — `Vec::with_capacity`
    sized by an on-disk count is a finding;
  - a value that fails validation produces a `Result` (corruption is the expected condition), never
    a panic and never silent clamping that fabricates data.

## Reporting honestly

- `A-PROVENANCE` — every artifact records how it was obtained: recovery stage, exact source extents
  (LBA ranges), source filesystem object if any, and content hash computed at recovery time.
  A result that cannot be traced to sectors is not a result.
- `A-CONFIDENCE-HONEST` — the reported confidence tier is the evidence tier
  (`FsMetadata > JournalResidue > ContiguousCarve > Reassembled > PartialOrThumbnail`), never
  higher. Unreadable sectors, TRIM-erased ranges and overwritten regions are reported as
  unrecoverable — Argos never interpolates or invents content.
- `A-NO-CONTENT-IN-LOGS` — logs, telemetry, error messages and panic messages never contain
  recovered content, filenames recovered from the medium, or identifying paths. Offsets, sizes,
  hashes and counts are fine.

## Guardrails for AI agents working on this repo

- Never run a command that writes to `/dev/*` or targets a block device with `dd`, `mkfs`,
  `shred`, `wipefs`, `blkdiscard` or similar. Hooks in `.claude/settings.json` enforce this;
  do not attempt to bypass them.
- Tests run exclusively against generated fixtures and image files. Real-device runs are manual,
  human-initiated, and live behind `#[ignore]`.
- Fixtures never contain real personal photos; generate synthetic images.

## Full rule text

[reference.md](reference.md) — `A-READ-ONLY`, `A-UNTRUSTED-ONDISK`, `A-PROVENANCE`,
`A-CONFIDENCE-HONEST`, `A-NO-CONTENT-IN-LOGS`.
