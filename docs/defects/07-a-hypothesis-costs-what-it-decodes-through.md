# 07 — A hypothesis costs what the decoder walks through before rejecting it

The stall of `defects/06` was fixed and the fix holds: a scan of the same 1 TB disk ran reassembly
for 5,996 s without a single step taking more than seconds. The stage still cannot finish. It moves
at **6.21 s per counter step** over a queue of **46,345**, which is 80 hours.

`defects/06` proved a delta — 5.759 s → 0.066 s on a fixture — and never asked the absolute: what a
real item costs, and how many there are. This is that measurement.

## What was measured

Release build, in memory, on the machine that ran the scan. Both halves of the search, at the
shipped ceiling of 262,144 hypotheses, over a 256 MiB region — and the same measurement twice, over
two kinds of surrounding content:

| | region of noise | region of photographs |
| --- | --- | --- |
| gap search, JPEG, span 66 KB | **1.49 s** | **150.28 s** |
| graph walk, JPEG, one header | **0.65 s** | **151.99 s** |
| per hypothesis | 2.5 – 5.7 µs | **573 – 580 µs** |

**Two orders of magnitude, from the content around the candidate.** Same code path, same budget,
same region size, same header.

The cause is what a rejected hypothesis does before it is rejected. A continuation is judged by
running the entropy decoder over the path followed by the trial bytes. Over pseudorandom noise the
decoder meets an invalid Huffman code almost at once and stops. Over another photograph's entropy
data it does not: the codes are valid, the MCUs decode into nonsense, and the decoder walks on for
hundreds of microseconds before the stream fails or the frame ends.

The budget is spent in full either way — 262,144 hypotheses, 0 assembled, in both columns.

## What this refutes

An earlier model attributed 80% of the stage's time to PNG, on the reasoning that the PNG oracle has
no resume and re-validates its whole path per hypothesis. The first half is true and the conclusion
does not follow:

| | modelled | measured |
| --- | --- | --- |
| PNG, per hypothesis | prefix ÷ 150 MB/s = 6.67 µs/KB | **0.186 µs/KB** (≈ 5.4 GB/s) |
| PNG item, at the span the real queue carries | 24.6 s | **0.29 s** |
| JPEG, per hypothesis | flat 7 µs | flat **5.3 – 5.6 µs** |

PNG's path walk is a chunk-header and CRC32 pass, not an inflate — 36× cheaper per byte than the
model assumed. And the PNG items the run actually reached carry a **median span of 144 bytes**,
which makes them the cheapest work in the queue rather than the most expensive. The model reproduced
the stage's total within −10.2% by cancelling one error against another.

JPEG's per-hypothesis cost does not grow with the prefix either. What grows is `Oracle::at`, which
decodes the prefix once per prefix candidate — 20 ms at a 64 KB span, 67 ms at 256 KB, at most eight
times per item. Against 262,144 hypotheses it is not where the time is.

## What follows for this medium

The field's 6.21 s/step sits between the two columns, near the low end: most of a 931 GiB disk is
not dense image data. But the photographs this disk is being searched for were written in batches —
113 frames from one camera inside 0.3 GiB — so **the regions that hold the target are the regions
where a hypothesis costs 580 µs instead of 2.5 µs.** The stage is slowest exactly where the payoff
is, and an average taken over the whole surface understates what the interesting regions cost.

## Not established

- **Where the time splits within one hypothesis.** The measurement is per hypothesis, end to end.
  How much is entropy decoding versus buffer assembly is not separated.
- **The real disk's mix.** The two columns bracket the field number; the proportion of regions of
  each kind on `/dev/sdc` was not measured.
- **Whether bounding the decode would preserve what is accepted.** A branch and bound against the
  best progress so far is the direct attack on this cost and it is untried; nothing here shows it
  keeps `recovery_rate.rs` at 87 % / 87 % / 25 % with 0 fabricated.

## Change

**The search now runs on every core the run was given.** H3 and H4 of the earlier audit are refuted
— the stage is neither I/O-bound nor contended, it is one core computing — so the cores were simply
idle. A region is memory once loaded, so the parallel unit is the header: each worker takes its own
view of the held bytes and its own buffers.

The gap search consults nothing the headers share and splits exactly. The walk consults the claimed
set, so every worker is given it as it stood when the phase began, and the region's outcome is
settled afterwards in header order — a walk a neighbour's recovery invalidated is run again against
everything claimed by then. The region's result is therefore the sequential one for any thread
count, which `the_search_finds_the_same_images_however_many_threads_ran_it` asserts.

Measured: **28.78 s → 5.02 s, 5.74×** on six physical cores, identical artifacts and identical
recovery counts on 1 and 12 workers. Against the field's 80 hours for 46,345 steps, that is **under
15 hours** — an overnight run rather than a week. What stays serial is reading the regions, which is
the medium's speed and not a core count.

**Nothing was done about the 580 µs itself.** The cost is a hypothesis decoding a frame's worth of
MCUs before it can be rejected, and the test that would reject it earlier — the seam — is a ratio
against the *median row difference of the whole frame*, which does not exist until the frame is
decoded. Cutting the decode short therefore means a different, absolute test emitted from inside the
entropy decoder, on a threshold calibrated to reject nothing the final test would accept. That is a
change to the most delicate code in the project and it is not made on an estimate.

Two further changes were made around this, both independent of the cost:

- Cancelling during the search no longer discards what the search found
  (`crates/argos_engine/src/session.rs`, `pipeline.rs`). A run cancelled in reassembly wrote
  `0 artifacts` with 12.4 GB of findings in hand; stopping the writing now takes a second request.
- The progress counter reports on a 5 s clock as well as on its stride. At 200 steps for a queue of
  46,345, a stage whose items cost seconds went quiet for 24 minutes at a time, which is what made
  this undiagnosable from a log.
