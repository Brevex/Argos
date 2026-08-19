# 09 — The counter counted steps of unequal cost, and the run was stopped

A scan of the 1 TB disk ran 10 h 41 m, gave reassembly 7 h 11 m of it, and was
**cancelled by the operator** with the bar reading **811 of 46,345 steps —
1.75%**. Read as a fraction of the time, that says twenty-four more days. It was
not a fraction of the time. Those 811 steps had covered 9 of the 27 regions
holding the target camera's material, and the candidates in them carried 76.8%
of the whole queue's decoded-MCU weight.

The stage was working. The instrument said it had barely started, and the
instrument was believed.

## What was measured

From `~/Imagens/Camera/scan.log` and the manifest that run wrote, with the queue
`plan_search` builds reconstructed from the manifest's 50,355 fragmentation
records — 22,521 taken, 1,303 regions, `22,521×2 + 1,303 = 46,345` steps, which
matches the log exactly.

Eight of the nine regions the run reached are individually timed by the log:

| region | headers | wall | s/header |
| ---: | ---: | ---: | ---: |
| 0 | 36 | 3,592 s | 99.78 |
| 1 | 107 | 14,087 s | **131.65** |
| 2 | 24 | 102 s | 4.25 |
| 3 | 65 | 239 s | **3.68** |
| 4 | 22 | 101 s | 4.59 |
| 5 | 50 | 266 s | 5.33 |
| 6 | 59 | 2,838 s | 48.10 |
| 7 | 16 | 151 s | 9.44 |

**A step costs between 3.68 s and 131.65 s — 36× — and three regions of 1,303
took 79.4% of the stage.** A count of them is not a measure of time.

It is worse than unequal: it is ordered against the reader.
`plan_search` sorts by `Reverse(decoded)` so the most promising material is
searched first, which is right — under a budget it is what puts photographs in
the output — and it puts the dearest steps at the front. The first 1.75% of the
counter is therefore the most expensive 1.75% there is.

The stage was also silent. The counter reports every five seconds by contract,
but only from inside `step`, and nothing calls that until an item finishes. The
log goes `reassembly 268/46345` and then, **4,112 s later**, `269/46345`, with
nothing between. `PROGRESS_INTERVAL_MS` says a stage that goes quiet exactly
when it is slowest cannot be told from a stalled one; this was that stage.

## What this refutes

The obvious repair is to weight the bar by a proxy for cost. Four were tested
against the eight timed regions, and all four fail:

| proxy | result |
| --- | --- |
| decoded-MCU mass | **refuted** — 173× spread in s/MCU; region 2 decoded 2,395,289 MCUs in 102 s, region 1 decoded 1,893,966 in 14,087 s |
| recovered-photograph density in the region | **r = −0.162** — the wrong sign; the two densest regions are among the cheapest |
| summed candidate span | r = +0.159 |
| header count | r = +0.817, but s/header still varies 36× within it |

Hypotheses spent is refuted too, and was already: `defects/07` measured the
ceiling of 262,144 spent in full over both noise and photographs, at 2.5 µs and
580 µs each.

**No quantity available before the search predicts what the search will cost.**
A weighted bar would replace a pessimistic lie with an optimistic one.

## What was changed

The stage already has a quantity that is exactly proportional to the time it has
left, and has had one since `DEFAULT_REASSEMBLY_BUDGET` was written wall-clock
rather than as a decode count — for this very reason, that "a decode's cost is
not a constant… and the stage cannot tell which it is in until it is there". The
counter never got that argument applied to it. Now it does.

- **A run with a budget reports against the budget.** `Counter::until` measures
  the stage in `Unit::Seconds`, elapsed of budget. It is exact, it is monotone,
  and it reaches its end when the stage does. Capped at the budget, so a stage
  that overran waiting for an item to finish does not report past full.
- **A run without one reports steps, and no percentage.**
  `Unit::supports_percentage` answers `false` for `Unit::Steps`, and both the
  terminal and the window honour it: the count is shown, the percentage is not,
  and the ring shows a dash. Nothing is invented for a run that cannot be
  predicted.
- **One thread watches the clock.** `Counter::tick` reports without advancing,
  driven by a scoped thread beside the search, so a stage grinding one item for
  an hour says so every five seconds.

The queue's own numbers are unchanged and stay where they were: the manifest's
`reassembly_attempted`, `reassembled` and `reassembly_skipped_small` are the
record of what was covered, and `ceilings.reassembly_decodes` still says when
the budget ran out (`A-CONFIDENCE-HONEST`).

## What this does not do

**It makes nothing faster.** Measured over the same queue, the stage still wants
about 25 hours, in an interval of 9.6 h to 49 h whose width is 18 regions nobody
has timed. What changed is that a run which is working no longer looks like one
that is stuck, and a run with a budget can be read at a glance.

The ordering that causes the effect is kept deliberately. Searching the most
promising material first is what makes a bounded run worth bounding; the defect
was reporting position in that order as though it were progress through time.
