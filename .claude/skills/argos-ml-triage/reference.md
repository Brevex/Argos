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

Inference is pure Rust. Introducing a native runtime (onnxruntime, libtorch) requires revisiting
the workspace unsafe policy explicitly — it is an architecture decision, not a dependency bump.

**Implemented: no inference runtime at all in the shipped crate.** `argos_classify::net` writes
the forward pass out directly — three convolutions, max-pooling, global average pooling, one
linear head — over weights read from a `safetensors` file by `argos_classify::weights`. Both
pure-Rust runtimes were weighed and rejected for the *evidence path*: `candle` makes `tokenizers`
a mandatory dependency, which brings roughly two hundred crates and several duplicated versions
(`cargo clippy`'s `multiple_crate_versions` fails on it) into the process that reads a disk, and
`tract` requires an ONNX export step that puts a second graph description between training and
inference. Four layers of arithmetic do not justify either. Training does use `candle`, in
`tools/train_triage`, which is `exclude`d from the workspace.

**The condition on that choice is `crosscheck`.** Two descriptions of one network can drift, and
drift would show up as an eval number nobody can explain — or not show up at all.
`tools/train_triage/src/bin/crosscheck.rs` runs the hand-written forward pass and the `candle` one
over the same pinned weights and the same inputs, and fails if any probability differs by more
than `1e-4`. It must pass before a new model's hash is pinned; it is the reason the split is safe,
and a change to either description without running it is a change that has left the standard.

Measured cost per artifact on the eval corpus: 2.61 ms inference, 1.00 ms pre-filter, 0.18 ms
perceptual hash, 0.03 ms input reduction. Batching does not amortize anything here — a plain loop
has no per-call setup — so the engine's batch worker exists for pipelining decode against scoring,
not for throughput.

## The pipeline (implemented)

Triage is two mechanisms with different inputs, and the split is deliberate:

- **Rule pre-filter** (`argos_classify::prefilter`) reads alpha, quantized palette size,
  luminance-level count and horizontal flat-run fraction. It may only ever return *synthetic
  asset*; a photograph verdict rests on absence of signal, and absence is not evidence.
- **Model** (`argos_classify::net`) reads texture from a 64x64 point-sampled, per-channel
  standardized RGB input. It never sees alpha — `model_input` composites it over white — so the
  two mechanisms are genuinely complementary rather than redundant.

Two consequences the eval harness has to respect. Point sampling is required, not incidental:
area-averaging the input erases the sensor noise the classes separate on, and training on
averaged inputs plateaus at chance-plus-epsilon. And any statistic quantized per channel is blind
to greyscale — a monochrome photograph has at most 32 distinct 5-bit colours — so the palette rule
requires a low **luminance**-level count alongside the low colour count, or every black-and-white
photograph is short-circuited away from the model.

Perceptual-hash dedup (blockhash, 8x8 block means against the median) runs before inference so
near-duplicates share one score. Grouping is an annotation: both artifacts stay in the manifest,
and the near-duplicate records which artifact it matched.

## Model and threshold changes are eval-gated (A-EVAL-GATED) { #A-EVAL-GATED }

<why>a threshold tweaked by eye silently reshapes what examiners look at first; only a fixed eval
set makes the trade-off visible.</why>

- The eval harness holds a fixed, versioned, labeled dataset (synthetic and licensed images only:
  photographs vs icons/sprites/UI chrome/web assets, including high-resolution assets).
- Every model replacement or threshold change runs the harness; precision/recall (overall and on
  the high-resolution-asset slice) are recorded in the harness output committed with the change.
- Thresholds are named constants documenting how they were derived (`M-DOCUMENTED-MAGIC`). The
  training pipeline lives outside the workspace; only the pinned model and the eval harness enter
  the repo.

**Thresholds are derived on the validation range, never on the eval corpus.** Fitting a threshold
to the corpus that gates it leaves nothing gating anything. `tools/train_triage`'s `thresholds`
binary reports the score distribution over the trainer's validation seeds — disjoint from both
training and eval — and the constants come from its percentiles.

**The harness measures the model alone as well as the shipped pipeline.** The pre-filter settles
most synthetic assets before inference, so the shipped numbers say almost nothing about the model
on that class: a model that stopped working entirely would still score well, because the rules
never claim a photograph and every photograph would merely come back `Ambiguous`. The model-only
floors are what make a model regression fail the gate.

**Corpus slices are a claim about what has been tested.** Each slice is a shape the classifier is
asserted to handle; a shape with no slice has no evidence behind it. Greyscale photographs earned
their own slice after a colour-only palette rule was found to discard every one of them.
