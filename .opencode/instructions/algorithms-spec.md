# Algorithm Specification Contract

This file is the source of truth for Argos's recovery algorithms. Implementations must match. Detailed derivations live in `docs/algorithms/`.

## Device dispatch

The recovery pipeline branches on detected device class:
- **HDD (rotational):** SmartCarving with PUP + SHT for fragmentation-point detection, plus format-specific fragment validation.
- **SSD / NVMe:** Header/Footer carving via Aho–Corasick over surviving regions. No reassembly attempted on the addressable region (TRIM + GC eliminate viability).

Device class is determined from kernel-reported rotational flag (Linux: `/sys/block/.../queue/rotational`; Windows: `STORAGE_DEVICE_SEEK_PENALTY_DESCRIPTOR`; macOS: `IOMediaIsRotational`). The detection function is centralized; ad hoc heuristics are forbidden.

## HDD pipeline

### Parallel Unique Path (PUP)

- Reference: Pal & Memon, "The Evolution of File Carving", IEEE Signal Processing Magazine, 2009.
- Reconstructs fragmented files by simultaneously extending all candidate files one block at a time, choosing at each step the candidate whose best next-block weight is highest, with the constraint that each block is consumed by at most one file.
- Inputs: a set of header-detected starting blocks and a block-weight function `w(file, block)`.
- Block-weight function `w` is plugged in per format. For JPEG, it is the negative log-likelihood of the next block being a continuation under Huffman validation (see below).
- Invariants:
  - Each block index appears in at most one reconstructed file.
  - Reconstruction terminates when (a) a footer is found, (b) the SHT detector signals a fragmentation point with no valid continuation, or (c) a per-format max-length cap is reached.

### Sequential Hypothesis Testing (SHT / Wald SPRT)

- Reference: Wald, "Sequential Tests of Statistical Hypotheses", 1945; applied to fragmentation by Pal, Sencar, Memon, "Detecting File Fragmentation Point Using Sequential Hypothesis Testing", DFRWS 2008.
- Two hypotheses per block transition:
  - H0: the next block continues the current fragment.
  - H1: the next block is a fragmentation point.
- Per-block test statistic: log-likelihood ratio derived from the format validator (e.g., Huffman match rate for JPEG, CRC residual structure for PNG).
- Default thresholds: α = 0.01 (false fragmentation rate), β = 0.01 (missed fragmentation rate). Bounds are `A = log((1-β)/α)` and `B = log(β/(1-α))`.
- Decision: `S_n ≥ A` accept H1 (fragmentation), `S_n ≤ B` accept H0 (continuation), else continue accumulating.
- Thresholds are configuration; defaults are constants in `carve::hdd::sht`.

### JPEG fragment validation (canonical Huffman, baseline; PUP continuation as heuristic in v0.1)

- Reference: ITU-T Rec. T.81 / ISO 10918-1, Annex C.2 (canonical Huffman) and F.2.2 (decoder procedures).
- **`validate(data)` (v0.1, see ADR 0010):** decodes the entropy stream with a canonical Huffman decoder seeded by the JPEG's own DHT segments. Component-to-table mapping is taken from SOS; per-component sampling factors from SOF0. Score `= mcus_decoded / mcus_expected`, with `mcus_expected` derived from frame dimensions and the maximum sampling factor. Non-baseline frames (SOF1/2/3) return `0.5` when structurally complete (worth recovering for visual inspection), `0.0` otherwise. JPEGs using a DRI restart interval recover up to the first restart marker; resumption past restart markers is deferred to v0.2.
- **`continuation_score(block)` (PUP weighting) in v0.1:** a coarse byte-distribution heuristic — low for zero-dominated blocks or blocks containing `0xFF` EOI/restart markers, high otherwise. This is sufficient for SPRT to discriminate filesystem padding from entropy data. State-resumed canonical decoding inside PUP is the v0.2 target.
- **v0.2 target:** restart-marker resumption in `validate`; PUP's `continuation_score` upgraded to a Huffman-state-seeded decoder that consumes prior-block state and scores by MCU yield in the new block. Will land behind a criterion benchmark gate.

### PNG fragment validation (CRC32)

- Reference: PNG specification, RFC 2083, chunk structure with CRC32.
- Each PNG chunk has a length, type, data, and CRC32 over (type + data).
- A candidate continuation block is valid if it preserves chunk alignment and CRCs verify on completed chunks.
- Partial chunks at block boundaries are tracked; validation defers until completion.

## SSD / NVMe pipeline

### Aho–Corasick header/footer carving

- Reference: Aho & Corasick, "Efficient String Matching: An Aid to Bibliographic Search", CACM 1975.
- Multi-pattern matching automaton built once over the union of known image-format magic numbers (headers and footers).
- Scan operates on streamed sector-aligned blocks. The automaton state persists across block boundaries.
- On header match: open a tentative artifact at the matched offset. On footer match: close and emit the artifact for validation.
- For formats without reliable footers (e.g., raw camera formats), use a max-length policy plus structural validation.
- The pattern set is centralized in `carve::ssd::patterns`; additions require an ADR.

### Why no reassembly on SSDs

TRIM and garbage collection mathematically eliminate the persistence of evicted blocks in the addressable region. Carving over surviving regions (slack, unmapped GC residue accessible via vendor tools) is the theoretical software-recovery ceiling. Argos does not pursue deeper reconstruction without specialized hardware support.

## Validation contracts (cross-pipeline)

Every recovered artifact, regardless of device class, passes through a format-specific structural validator before being emitted:
- JPEG (v0.1): SOI/EOI present, SOF + DHT + DQT + SOS segments present, baseline entropy stream decodes via canonical Huffman. Restart-marker resumption deferred to v0.2 (ADR 0010).
- PNG: IHDR present, CRC32s verify on all chunks, IEND present.
- Future formats: contracts added here before code.

## Property-test obligations

The following must have property tests:
- Header/footer pattern matching (no false negatives for synthesized inputs containing magics; bounded false-positive rate).
- SPRT decision boundaries (decisions match closed-form expectation).
- PUP block uniqueness (no block consumed by two files).
- CRC32 over reconstructed chunks (matches reference implementation).

## Divergence from spec

Implementation may not silently diverge from this document. Any deviation is recorded as an ADR under `docs/decisions/`, and this file is updated in the same PR.
