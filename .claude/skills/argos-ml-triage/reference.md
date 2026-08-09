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

**Implemented: there is no model.** Triage is deterministic image statistics — transparency,
palette, flat runs, and the high-frequency floor a sensor and a JPEG quantizer leave behind. The
rule above has no artifact to govern, and what it protects is satisfied differently: reproducibility
comes from the procedure being in the source tree the binary was built from, so every manifest
records `RULES_VERSION` where it used to record a model hash. Bump that constant when a rule or a
threshold changes; a label is only reproducible against the procedure that produced it. Nothing is
loaded at startup, so the load-failure path is gone with the thing that could fail.

## Triage orders, it never decides (A-TRIAGE-NOT-VERDICT) { #A-TRIAGE-NOT-VERDICT }

<why>a false negative that deletes or hides a recovered photo is an evidence-destroying bug; no
classifier is reliable enough to be given that power.</why>

- The `Classifier` port runs only after an artifact is persisted, hashed and manifested; its output
  is written *onto* the finding (score, label, model id).
- No API exists to filter, skip or delete based on score. UI and CLI may sort and group by score;
  the full manifest remains complete and exportable regardless of any threshold.
- The rule-based pre-filter (dimensions, alpha usage, palette statistics) obeys the same law: it
  short-circuits *inference*, never recovery or reporting.

**One exception exists, and it is written down here because a rule quietly broken is worse than a
rule with a stated exception.** `--exclude-assets` leaves artifacts labelled `synthetic-asset`
unwritten, which is a label deciding what reaches the output directory. Its conditions are what keep
it defensible, and all four hold together or the exception is void:

1. **Opt-in.** Off by default on the command line. A caller asks for it by name.
2. **Nothing leaves the account.** Every omitted artifact is examined, hashed and recorded with its
   extents, format, confidence, label and the property that decided the label, plus `written: false`
   and why. The manifest stays a complete statement of what the medium held; only the directory is
   smaller, and `argos export` can fetch the bytes from the source without a second scan.
3. **Only a settled label acts.** `Ambiguous` is written, and so is anything that did not decode or
   could not be judged. An unclear label must never cost evidence.
4. **The label is decided before the write, not after.** That is why screening happens inside the
   report stage rather than in the annotation pass — a decision about whether to write cannot run
   after the write. The annotation pass is unchanged and still only annotates.

## Inference is pure Rust (A-INFERENCE-PURE-RUST) { #A-INFERENCE-PURE-RUST }

**Implemented: there is no inference.** The rule is satisfied vacuously and stays as written,
because it governs what may be introduced rather than what is there. Bringing a native runtime into
the result path is still an architecture decision, not a dependency bump — and the bar it has to
clear is now higher, since deterministic statistics reach 1.000 precision and recall on the eval
corpus at a fraction of the cost.


<why>the workspace's safety story is "unsafe lives in argos_device"; a C++ inference runtime in the
result path would be its largest unauditable exception.</why>

Inference is pure Rust. Introducing a native runtime (onnxruntime, libtorch) requires revisiting
the workspace unsafe policy explicitly — it is an architecture decision, not a dependency bump.

**Implemented: nothing to cross-check.** The crate that ships holds no network, no weights file and
no second description of the decision to drift from the first. What used to guard that — a
`crosscheck` binary comparing a hand-written forward pass against `candle` over pinned weights — was
deleted with the thing it guarded.

Measured cost per artifact: 1.00 ms for the statistics pass, 0.18 ms for the perceptual hash. The
inference it replaced measured 2.61 ms, and a recovery from a system disk pays that per artifact —
twenty-three thousand of them on the run that prompted the change.

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
to the corpus that gates it leaves nothing gating anything. `crates/argos_classify/tests/thresholds.rs`
prints each feature's distribution over a range of the generator disjoint from the eval seeds, and
the constants come from its extremes — the two ends that have to be separated, never the middle of
either distribution. It is `#[ignore]`d because it measures rather than asserts.

**A threshold is placed on the expensive error's side.** The two mistakes are not equally costly:
with `--exclude-assets` a photograph labelled an asset is a photograph missing from the output,
while an asset labelled a photograph is a file on disk. Every threshold in `rules.rs` documents both
ends of the gap it sits in and which side it leans to.

**Corpus slices are a claim about what has been tested.** Each slice is a shape the classifier is
asserted to handle; a shape with no slice has no evidence behind it. Greyscale photographs earned
their own slice after a colour-only palette rule was found to discard every one of them.

**The corpus is synthetic.** Every number this harness reports is against generated images, which
is what makes it reproducible and what limits what it proves. It gates regressions in the rules; it
is not a measurement of real media.
