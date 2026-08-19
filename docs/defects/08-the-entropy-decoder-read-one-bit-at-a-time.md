# 08 — The oracle every hypothesis answers to read one bit at a time

Reassembly is 83% of a scan's wall clock and the whole of what a budget is spent
on. Its cost is one entropy decode per hypothesis, and that decode had no
optimisation of any kind: no lookup table, no multi-byte refill, no SIMD, no
`#[inline]` — a canonical T.81 walk pulling one bit per call through two layers
of `Result<Option<u8>>`, over a region already resident in memory.

## What was measured

`cargo bench -p argos_carve`, release, on a Ryzen 5 5600XT. The decode is
coefficients-only — no dequantisation, no IDCT, no pixels.

| | Before |
| --- | --- |
| 640x480 frame, 47,183 bytes, 4,800 MCUs | 1.465 ms |
| throughput | 30.7 MiB/s |
| per MCU | 305.2 ns |
| **per bit** | **3.88 ns — 15.5 cycles at 4 GHz** |

Fifteen cycles to extract one bit is the whole finding. A mature JPEG entropy
decoder amortises the refill and resolves a symbol in one table lookup.

The stage's own distribution says where that lands. Of the field run's 866
progress samples, **160 steps took over 60 s and account for 77% of the stage**;
the largest advanced one step in 3,946 s. The per-hypothesis cost `defects/07`
measured — 580 µs — was over a small fixture; a frame of 23,232 MCUs costs
proportionally more, so the slowest items are the large frames in regions dense
with photographs, which is exactly what the search exists to find.

## What was changed

Four changes, each measured alone, in the order they were tried:

| | Change | Effect |
| --- | --- | --- |
| 1 | Take the source position lazily — only the error path reads it — and `#[inline]` the two innermost readers | **−2.1%** |
| 2 | Refill seven clean bytes at once instead of one byte per eight bits | **−1.7%**, within noise |
| 3 | A 10-bit lookup table resolving a Huffman symbol in one indexed read | **−49%** |
| 4 | Take a coefficient's extra bits from the accumulator in one piece | **−30%** |

The first two are near-nothing on their own and are worth stating as such: the
cost was never call overhead — LLVM was already inlining — and never the refill,
because `Bytes` was already buffered, so `next` was an index. Both were
hypotheses that measurement refuted.

The multi-byte refill is kept regardless of its own −1.7%, because it is what
holds 56 bits in the accumulator, and **without it the lookup table almost never
fires**: the table is only consulted with its full width already in hand. It
enabled the gain rather than producing it.

| | Before | After | |
| --- | --- | --- | --- |
| entropy decode | 1.465 ms | 487 µs | **3.01×** |
| throughput | 30.7 MiB/s | 92.3 MiB/s | |
| per bit | 15.5 cycles | 5.2 cycles | |
| gap search, exhausted | 1.337 ms | 451 µs | **2.96×** |
| gap search, found | 1.139 ms | 517 µs | **2.20×** |

*Exhausted* is the field case: `defects/07` measured 262,144 hypotheses and 0
assemblies. Against the queue that run left unfinished, ~61 h becomes ~21 h.

## Why this is not a different decoder

The oracle decides what counts as a recovered photograph, so a change that makes
it faster and also makes it answer differently has replaced it. A recovery rate
that did not move does not rule that out — it is a sample over six fragmentation
patterns, and a decoder can shift on inputs no pattern poses.

Two things rule it out.

**Both fast paths are entered only with the bits already in the accumulator.**
Neither reads the source, so neither can move where the data ends or what halted
it. Anything they do not resolve falls through to the canonical walk, unchanged.
The one place lookahead could have been observed is the source position, which
feeds `end`, `settled`, the offset a splice was crossed at, and `replay_from`:
`BitReader::settle` hands the lookahead back at every MCU boundary, restoring
the one-byte invariant the rest of the decoder is written against. Reading ahead
therefore happens only *within* an MCU — 4,800 settles per frame against 377,464
bit reads.

**The lookup table is verified exhaustively, not sampled.**
`mcu::tests::the_lookup_agrees_with_the_canonical_walk_on_every_prefix` compares
the table against a transcription of the T.81 walk for **every one of the 1,024
prefixes**, across tables covering every shape a Huffman table can take. The
domain is finite and it is checked entire. A prefix the walk refuses must become
a miss and fall through, not be answered for.

`tests/oracle_equivalence.rs` records the whole `ScanOutcome` — every field — over
a corpus of whole photographs, frames cut at four depths, single flipped bytes,
one photograph spliced onto another, a real header over noise, and pure noise,
through `scan_watching` and through `resume_at`/`scan_resumed`. It was recorded
from the decoder as it stood, verified against the untouched sources with the
changes stashed, and passed unchanged after each of the four steps.

## What was measured and not done

**A per-item cost cap.** The tail is 77% of the stage, so bounding it looks
obvious. Against the *original* timings a 300 s cap saved 32%. Against the
optimised ones it saves 18% — 4.9 h becomes 4.0 h — and cuts 2.5% of items,
which are the large frames, which are the photographs. The optimisation absorbed
the tail's damage, and what the cap would buy is no longer worth what it would
drop.

**Building the resume cursor only when asked for.** ~48 bytes written per MCU,
2–3% of the stage. Measured as not worth the branch.

**Memoising the whole-frame median in the seam test.** It is recomputed once per
seam, but only on the acceptance path: 135 acceptances in the field run, a few
tens of milliseconds each, seconds out of hours.
