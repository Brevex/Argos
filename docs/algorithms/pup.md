# Parallel Unique Path (PUP)

## Reference

Pal, A., & Memon, N. (2009). *The Evolution of File Carving*. IEEE Signal Processing Magazine, 26(2), 59–71.

## Problem

Given a disk image and a set of header-detected start blocks for candidate files, reconstruct each file by selecting the correct sequence of data blocks, under the constraint that each block belongs to at most one file.

## Definitions

- `B = {b_0, b_1, ..., b_{n-1}}` — the ordered set of data blocks of fixed size on the source device.
- `H = {h_0, h_1, ..., h_{k-1}}` — header blocks, each `h_i ∈ B`. Each header is the first block of a candidate file `f_i`.
- `w(f, b)` — block-weight function: a real-valued score representing the likelihood that block `b` is a valid continuation of file `f` given its current reconstructed prefix. Higher is better. Format-specific.
- `s(f, b)` — best-continuation score: `s(f, b) = max_{b' ∈ B \ consumed} w(f, b')`. PUP picks the file whose best continuation has the highest weight.

## Algorithm (informal)

```
consumed ← {h_0, ..., h_{k-1}}
for each f_i: prefix(f_i) ← [h_i]
active ← {f_0, ..., f_{k-1}}

while active is non-empty:
    pick f* ∈ active maximizing s(f*, ·) over candidate next blocks
    let b* = argmax_{b ∉ consumed} w(f*, b)
    if w(f*, b*) below acceptance threshold OR SHT decides fragmentation:
        finalize(f*); active ← active \ {f*}
        continue
    append b* to prefix(f*); consumed ← consumed ∪ {b*}
    if footer(f*) detected OR max-length(f*) reached:
        finalize(f*); active ← active \ {f*}
```

## Invariants (must hold in implementation)

1. **Block uniqueness.** `consumed` is a set; no block index appears in the prefix of two files.
2. **Termination.** Each iteration either consumes a block (reducing the unconsumed set) or removes a file from `active`. The outer loop terminates in at most `|B|` iterations.
3. **Determinism.** Given the same `B`, `H`, and `w`, the algorithm produces the same output. Tie-breaking is by lowest block index.
4. **No fabrication.** Blocks corresponding to bad sectors are excluded from the candidate pool; PUP never traverses fabricated bytes.

## Implementation notes

- The candidate-block pool for each file is bounded by a search window around the current end of `prefix(f)` to keep `argmax` tractable.
- `s(f, ·)` is recomputed incrementally when blocks are consumed by other files.
- For JPEG, `w(f, b)` is the negative log-likelihood derived from the Huffman validator's score; for PNG, it is derived from CRC32 chunk-completion likelihood.
- The acceptance threshold and the fragmentation decision are delegated to `validate/` and `sht`.

## Property tests (required)

- Block uniqueness across all reconstructed files for synthesized inputs.
- Termination within `|B|` iterations.
- Determinism: shuffling input order does not change output.
- For inputs with no fragmentation: PUP reconstructs files identically to linear carving.
