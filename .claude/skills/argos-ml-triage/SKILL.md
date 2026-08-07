---
name: argos-ml-triage
description: Use BEFORE writing or reviewing anything in `argos_classify` — model loading, inference, classification thresholds, perceptual-hash dedup — or touching the model artifact and its eval harness. Covers A-MODEL-PINNED, A-TRIAGE-NOT-VERDICT, A-INFERENCE-PURE-RUST, A-EVAL-GATED.
---

# ML triage

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.
The classifier orders and labels recovered images (photograph vs synthetic asset); it exists to
save the examiner's time, never to decide what the examiner sees.

## The boundary that must never move

`A-TRIAGE-NOT-VERDICT` — classifier output is a score attached to an artifact. It may sort, group
and label. It may **never** remove an artifact from the manifest, skip its recovery, or trigger
deletion. There is no code path from a classifier score to artifact disposal — structurally, the
`Classifier` port is consumed only after artifacts are persisted and hashed.

## Operational rules

- `A-MODEL-PINNED` — the model is a local file, versioned in the manifest, verified against a
  SHA-256 recorded in the source tree before first use. No runtime downloads, no network access
  in `argos_classify` at all. A hash mismatch disables triage (scan proceeds, `AcceptAll`
  behaviour) and is reported.
- `A-INFERENCE-PURE-RUST` — inference uses a pure-Rust runtime (`tract` or `candle`; decide by
  benchmark at P6). No C/C++ runtime dependencies: `unsafe` stays confined to `argos_device`.
- `A-EVAL-GATED` — the eval harness (fixed labeled set: photos vs icons/sprites/UI assets,
  including high-resolution assets) gates every model or threshold change; precision/recall are
  recorded in the harness output, and thresholds are named constants with documented derivation
  (`M-DOCUMENTED-MAGIC`).

## Also invoke

`rust-performance` for the batch worker; `argos-evidence-handling` — decoded pixel data counts as
recovered content (`A-NO-CONTENT-IN-LOGS`).

## Full rule text

[reference.md](reference.md) — `A-MODEL-PINNED`, `A-TRIAGE-NOT-VERDICT`,
`A-INFERENCE-PURE-RUST`, `A-EVAL-GATED`.
