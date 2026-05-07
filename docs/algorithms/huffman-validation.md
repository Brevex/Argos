# Huffman Validation for JPEG Fragments

## Reference

van der Meer, J., & van den Bos, A. — work on Huffman-based JPEG fragment validation. (Argos's implementation follows the published approach: feeding candidate continuations into a JPEG entropy decoder seeded with prior state, scoring by Huffman-table consistency and DCT coefficient ranges.)

## Problem

Given a partially reconstructed JPEG (decoder state at end of current prefix) and a candidate continuation block, score how likely the candidate is to be a valid continuation.

## Decoder state

A JPEG entropy decoder operating on baseline DCT files maintains:
- The active Huffman tables for DC and AC coefficients per component.
- The current MCU position within the image grid.
- The DC predictor per component (since DC coefficients are differentially coded).
- The bit reservoir (residual bits from the previous block boundary).

This state is the input to the validator alongside the candidate block.

## Validation procedure

1. Initialize the decoder with the prior state.
2. Feed bytes from the candidate continuation, byte-stuffing rules applied.
3. Decode MCUs until either:
   a. The candidate is exhausted (return partial-success score).
   b. A Huffman-table violation is encountered (invalid prefix, no matching symbol).
   c. A DCT coefficient is decoded outside the legal range for the component.
   d. The expected number of MCUs for the image is reached and an end-of-image marker (`0xFFD9`) is observed (full success).
4. Score = (MCUs successfully decoded) / (MCUs attempted in this candidate), bounded to `[0, 1]`.

## Score interpretation

- Score near 1.0: strong evidence the candidate is a valid continuation.
- Score near 0.0: strong evidence it is not.
- Intermediate scores: the SPRT accumulator (`sht`) decides over multiple observations.

## Invariants

1. The validator is a pure function of (decoder state, candidate bytes). No global state.
2. The validator never modifies the input bytes.
3. The validator does not allocate per-call (decoder state is reused via a buffer pool).
4. The score is monotonic in the number of successfully decoded MCUs, holding length constant.

## Edge cases

- Candidate ends mid-MCU: partial credit proportional to bits consumed; remaining MCUs contribute zero.
- Candidate contains a restart marker (`0xFFD0..0xFFD7`): the decoder resets the DC predictor and resyncs. Validator continues.
- Byte stuffing: `0xFF 0x00` is treated as literal `0xFF`.
- Marker segments inside compressed data: anything other than restart markers is a violation.

## Property tests (required)

- For a known-good JPEG split into N contiguous chunks, the validator scores all chunks ≥ 0.99.
- For random data appended to a valid prefix, the validator scores ≤ 0.05 with high probability.
- Validator is deterministic: same inputs produce the same score.
- Validator does not panic on adversarial inputs (fuzz target required).
