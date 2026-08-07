# ML triage — full rule text

Argos-specific rules (`A-*` namespace). Index: [../../guidelines-index.md](../../guidelines-index.md).

## The model is a pinned local artifact (A-MODEL-PINNED) { #A-MODEL-PINNED }

<why>a forensic tool must be reproducible and offline; a model fetched or swapped at runtime makes
results unreproducible and opens a supply-chain hole into an evidence pipeline.</why>

- The model ships as a local file; its SHA-256 and version are constants in the source tree, and
  the manifest of every scan records which model (hash + version) scored it.
- `argos_classify` performs no network I/O of any kind.
- On hash mismatch or load failure: triage is disabled for the session, the scan proceeds with
  `AcceptAll` behaviour, and the condition is reported in the manifest — never a silent fallback,
  never a hard failure of the scan.

## Triage orders, it never decides (A-TRIAGE-NOT-VERDICT) { #A-TRIAGE-NOT-VERDICT }

<why>a false negative that deletes or hides a recovered photo is an evidence-destroying bug; no
classifier is reliable enough to be given that power.</why>

- The `Classifier` port runs only after an artifact is persisted, hashed and manifested; its output
  is written *onto* the finding (score, label, model id).
- No API exists to filter, skip or delete based on score. UI and CLI may sort and group by score;
  the full manifest remains complete and exportable regardless of any threshold.
- The rule-based pre-filter (dimensions, alpha usage, palette statistics) obeys the same law: it
  short-circuits *inference*, never recovery or reporting.

## Inference is pure Rust (A-INFERENCE-PURE-RUST) { #A-INFERENCE-PURE-RUST }

<why>the workspace's safety story is "unsafe lives in argos_device"; a C++ inference runtime in the
result path would be its largest unauditable exception.</why>

Inference uses `tract` or `candle` (chosen by benchmark; both pure Rust). Introducing a native
runtime (onnxruntime, libtorch) requires revisiting the workspace unsafe policy explicitly — it is
an architecture decision, not a dependency bump.

## Model and threshold changes are eval-gated (A-EVAL-GATED) { #A-EVAL-GATED }

<why>a threshold tweaked by eye silently reshapes what examiners look at first; only a fixed eval
set makes the trade-off visible.</why>

- The eval harness holds a fixed, versioned, labeled dataset (synthetic and licensed images only:
  photographs vs icons/sprites/UI chrome/web assets, including high-resolution assets).
- Every model replacement or threshold change runs the harness; precision/recall (overall and on
  the high-resolution-asset slice) are recorded in the harness output committed with the change.
- Thresholds are named constants documenting how they were derived from the eval run
  (`M-DOCUMENTED-MAGIC`). The training pipeline lives outside the workspace; only the pinned model
  and the eval harness enter the repo.
