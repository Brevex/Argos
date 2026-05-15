# ADR 0010 — Canonical Huffman JPEG validation; PUP continuation as heuristic in v0.1

- **Status:** Accepted
- **Date:** 2026-05-13 (revised)
- **Affects:** `.opencode/instructions/algorithms-spec.md` § "Validation contracts" and § "JPEG fragment validation".

## Context

The first iteration of `src/validate/jpeg/mod.rs` shipped a Huffman entropy decoder that was incorrect:

1. `decode_huffman_value` indexed `table.values` by the integer value of the code itself, instead of computing the canonical offset `valptr[L] + (code - mincode[L])`. The per-length `mincode`, `maxcode`, `valptr` tables (Annex C.2 / F.2.2.3 of ITU-T T.81) were never built. Only fixtures with a single-code-of-length-1 table happened to pass.
2. DC and AC table selectors were hardcoded to slot 0. JPEGs of 3+ components (Y/Cb/Cr) assign distinct tables per component in SOS; the decoder ignored SOS entirely.
3. No byte stuffing (`0xFF 0x00`), no DC differential decoding, no run-length AC categorization.

The earliest revision of this ADR proposed replacing the entropy decoder with a structural-only validator (presence of SOI/EOI/SOF/DHT/DQT/SOS). That made the user-visible behaviour better than "0 recoveries" but downgraded the project to dressing — `DecoderState`, `parse_huffman_table`, `HuffmanTable`, `jpeg::DHT`, `jpeg::SOF0` were left exposed and populated in `pup::initialize_state` while nothing consumed their contents, in direct violation of ADR 0006 (no dead code).

This revision corrects that.

## Decision

For v0.1:

1. **`validate(data)` uses a canonical Huffman entropy decoder.** Internal pipeline:
   - `parse_jpeg(data)` returns `(segments, entropy_start, entropy_end)`. The SOS segment retains its body so we can read scan component selectors.
   - `HuffmanLut::from_segment_data` builds canonical Huffman tables (`mincode`, `maxcode`, `valptr`, `values`) per DHT.
   - `parse_frame` parses SOF0 (precision, dimensions, per-component sampling and quant table indices).
   - `parse_scan_components` parses SOS (per-component DC/AC table selectors), cross-referencing the frame.
   - DQT presence is verified for every component's referenced quantization table.
   - `BitReader` implements byte-stuffing (`0xFF 0x00` collapses to `0xFF`) and detects markers (any `0xFF NN` where `NN ∉ {0x00}`).
   - `decode_block` reads the DC magnitude category, consumes the category bits, then walks the AC stream: `(R, S)` pairs run-length-skip zeros and consume `S` magnitude bits, terminating on EOB or after coefficient 63. Categories outside the baseline range (DC > 11, AC > 10) abort the block.
   - `decode_mcu` dispatches blocks across components by their `H × V` sampling factors.
   - `validate` decodes MCUs until either the expected count is reached or the decoder fails (truncated, marker mid-stream, invalid Huffman path). Score `= decoded / expected`.
   - Non-baseline frames (SOF1/2/3 — extended sequential, progressive, lossless) are recognised by `parse_jpeg`. Without a baseline decoder we cannot validate their entropy; if the surrounding structure is otherwise complete, score is `0.5` (worth recovering for visual inspection); otherwise `0.0`.

2. **PUP `continuation_score` remains a coarse byte-distribution heuristic.** Per-block Huffman-state-resumed scoring requires carrying decoder state across path extensions and is the right v0.2 work — it pushes complexity into the SPRT loop that ADR 0006's no-speculation rule does not justify before the first release is field-tested. The heuristic (low score for zero-dominated blocks or blocks containing `0xFF` EOI/restart markers; otherwise neutral-high) is sufficient for SPRT to discriminate filesystem padding from entropy data.

3. **Dead JPEG state in PUP is removed.** `Path::jpeg_state`, the `DecoderState`-population branch of `initialize_state`, the public re-exports of `parse_huffman_table`, `HuffmanTable`, `DecoderState`, `DHT`, and `SOF0` are deleted. `parse_segments` is also removed (its only caller was a test, which now calls the internal `parse_jpeg` directly). The Huffman machinery is consumed by `validate` and lives entirely within `src/validate/jpeg/mod.rs`.

4. **Dead `ValidationKind` variants are removed.** `InvalidMarker`, `BadHuffmanCode`, `BadEntropyStream`, `BadDctCoefficient`, and `BadCrc32` had no producer. The surviving set is `MissingSoi`, `MissingEoi`, `TruncatedSegment`, `BadHuffmanTable`, `MissingIhdr`, `MissingIend`, `TruncatedChunk`.

## Rationale

- **Recovery in v0.1 must work on real disks.** A correctly-implemented Huffman decoder is what the project promised; restoring it (rather than papering over with a structural check) is the right answer to "but does the software use Huffman?".
- **Restart markers are deferred.** Mid-stream restart-marker resynchronisation (`0xFF 0xD0..0xD7`) is structurally separate from canonical decoding and adds buffer-state complexity (drain padded bits, detect marker after byte boundary, reset DC predictors). v0.1 detects markers via `BitReader` but does not resume past them — JPEGs with `DRI` set score by the number of MCUs decoded up to the first restart, then halt. This is correct partial-recovery behaviour and a future ADR will add resumption.
- **No half-built abstractions.** Anything not consumed by the bridge, tests, or benchmarks is gone. The remaining JPEG types are the minimum required to decode baseline entropy streams.

## Consequences

- `src/validate/jpeg/mod.rs` becomes the single home of all JPEG-decoding code: `BitReader`, `HuffmanLut`, frame/scan/quant parsers, `decode_block`, `decode_mcu`, `validate`, `continuation_score`. Nothing in this module is `pub` beyond `validate` and `continuation_score`.
- `src/carve/hdd/pup.rs` no longer references `jpeg::DecoderState`, `jpeg::DHT`, `jpeg::SOF0`, or `jpeg::parse_huffman_table`. The `Path` struct loses the `jpeg_state` field.
- `algorithms-spec.md` JPEG validation section is updated to reflect baseline decoding with restart-marker resumption deferred to v0.2, and to clarify that PUP continuation scoring uses a heuristic in v0.1.
- Forensic operators get correct partial recovery on restart-marker JPEGs and full recovery on baseline JPEGs without restart intervals (the common case for camera photos before re-encoding).
- The `jpeg_validate` criterion benchmark continues to exercise `validate`; the change in score semantics (now reflects actual decode success) will be visible in benchmark output.

## Alternatives considered

- **Use a third-party JPEG decoder (`jpeg-decoder`, `image`).** Each introduces an adversarial-input parsing surface that needs its own threat-model review per ADR 0006 and `security-baseline.md`. Tracked for v0.2 only as a comparison baseline.
- **Implement restart-marker resumption now.** Requires either (a) draining the BitReader's buffered bytes before each align/check, or (b) refactoring BitReader to expose its byte-aligned position so we can sync deterministically. Both are tractable but each adds invariants the v0.1 review window cannot absorb.
- **Keep the structural-only validator from the first revision.** Rejected because it left meaningful complexity (the Huffman types) on the books as dead code, and because honest forensic carving should validate the entropy stream when it can.
