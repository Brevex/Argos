# 05 — Reassembly was measured in the lab and never ran in the field

The scan of the 1 TB disk recorded in [01](01-reassembly-unbounded.md) reported
**0 images recovered by reassembly** out of 345,862 artifacts. The stage exists,
is correct, and is held to measured floors — 85% on two fragments, with zero
fabrication. It did not fail because it is weak. It never reached the candidates.

## What was measured

Three faults, each reproduced as a test before it was changed.

**The budget was spent on the accounting.** `REASSEMBLY_BUDGET` allowed 250,000
decode attempts for a whole medium, and a candidate that found nothing was
charged the per-candidate ceiling of 4,096 rather than what it spent — including
a candidate rejected outright, which spends none. On a fixture holding 200
fragmentation points the stage reached **63** of them and stopped.

**The graph walk looked in the wrong place.** `grow_path` iterated the candidate
block list in medium order for every header, so the blocks it tried first were
always the lowest-numbered on the disk. For a header at 800 GB the continuation
was never among them. It survived review because the suite's media are a few
hundred kilobytes, where every block is a neighbour of every header.

**The gap search reached a quarter of what it advertised.** `MAX_GAP_BYTES` is
64 MiB, but the hypothesis ceiling stopped the sweep at 4,096 × 4 KiB = 16 MiB,
and it returned from inside the first splice's loop, so seven of the eight
`MAX_PREFIX_CANDIDATES` were never tried. The search was also forward-only.

Behind all three sat a cost the constant's own documentation had wrong. It
recorded 5 us for a rejected hypothesis. Measured against planted photographs,
every hypothesis re-decoded the first fragment from `SOI`:

| first fragment | per hypothesis |
| --- | --- |
| 5.8 KB | 166 us |
| 21 KB | 701 us |
| 81 KB | 2.6 ms |
| 315 KB | 10.6 ms |

Linear in the fragment, so a 1 MB first fragment costs ~33 ms and one candidate's
full sweep costs nine minutes. No budget could have been both large enough to
find anything and small enough to finish.

## Change

- **The decode is resumed rather than repeated.** `mcu::resume_at` decodes a
  first fragment once to the last MCU boundary at or below the splice and keeps
  the position — predictors, bit accumulator, restart phase, MCU index —
  and `mcu::scan_resumed` carries it on over each proposed continuation. With
  the cursor restored, a hypothesis costs its own bytes: **~7 us whatever the
  fragment's size**, from 166 us and 10.6 ms at the extremes.
- `Bytes` refills from 1 KiB, doubling to 64 KiB. A rejected hypothesis reads
  about thirty bytes, and reading 64 KiB to look at them was the rest of the
  cost.
- `bifragment` and `parallel_unique_path` report the attempts they spent,
  found or not, and the engine charges that. The stage's ceiling is now wall
  clock — `--reassembly-budget`, two hours by default — because a decode's cost
  is not a constant a count could stand for.
- Each splice gets a share of the per-candidate ceiling with the remainder
  rolling forward, and the sweep runs ahead of the first fragment and then
  behind the header. An assembly whose trimmed extents overlap is refused.
- `grow_path` takes candidates nearest-first from the end of the path, within
  `SEARCH_RADIUS_BYTES`, and starts from every splice rather than the nearest.
- Extents already recovered by filesystem metadata seed `spoken_for`, so the
  walk does not spend the budget reclaiming bytes a run list already accounts
  for.

## Proof

`crates/argos_carve/tests/reassemble_at_scale.rs` plants fragments on media of
64–256 MiB, generated per byte read rather than held (`fixture::SparseDisk`), so
the distances are a disk's. Four tests, each failing before the change: a gap of
24 MiB; a remainder stored behind its header; a continuation the candidate list
ranks 20,000th; and a header whose remainder is **not** on the medium, which must
still reassemble to nothing.

`every_fragmentation_point_is_offered_to_reassembly` in
`crates/argos_engine/tests/pipeline.rs` plants 200 broken photographs: 63 before,
200 after.

`recovery_rate.rs` is unchanged and still reports **0 fabricated** on every
pattern, including the competing photograph from the same encoder. Widening a
search is only safe because the oracle did not move.
