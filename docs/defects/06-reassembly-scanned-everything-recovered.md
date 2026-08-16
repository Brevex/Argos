# 06 — The search consulted every recovered artifact for every candidate block

A scan of the 1 TB disk ran for 12 h 40 m. The reading stages finished normally; reassembly entered
at 13,546 s and then **stopped reporting progress for over six hours** on a single step. The stage
was not stalled on the medium and was not deadlocked: it was computing.

The suspicion it raised — that reassembly had never actually run in the field, and that the
two-hour ceiling had only been hiding it — was correct.

## What was measured

From the run's own log:

| Stage | Window | Result |
| --- | --- | --- |
| `carve` | 0 → 7,238 s | 2,026,184 candidates |
| `filesystem` | 7,239 → 8,744 s | 2,618 findings |
| `validation` | 8,744 → 13,546 s | **1,639,834 findings** |
| `reassembly` | 13,546 s → killed | 46,345 steps planned |

The stage's counter has a stride of 232. It reported twice and then went quiet:

```
[19869.226s] reassembly  232/46345     1 h 45 m for the first step
[22997.019s] reassembly  464/46345     +52 min
                                        nothing further; elapsed 45,106 s
```

From the live process: 8 h 56 m of CPU, **31,787 s of user time against 63 s of system**. Almost no
I/O. The stage was spending its time in one loop.

Measured against the loop directly, the cost is the product of two numbers and nothing else:

| candidate blocks | claimed extents | walk |
| --- | --- | --- |
| 512 | 400,000 | 0.357 s |
| 2,048 | 400,000 | 1.468 s |
| 8,192 | 100,000 | 1.455 s |
| 8,192 | 400,000 | 5.759 s |
| 8,192 | **0** | **0.057 s** |

Sixteen times the blocks is sixteen times the time; four times the claimed set is four times the
time; with nothing claimed the cost disappears.

## Cause

`grow_path` asks, once per candidate block it considers, whether those bytes are already spoken for
— by filesystem metadata, by a contiguous carve, or by a reassembly that already completed. The
question is right. The answer was found by walking the whole list:

```rust
fn covers(extents: &[ByteRange], at: u64) -> bool {
    extents.iter().any(|extent| /* … */)
}
```

That list is one extent per artifact the run has recovered. On a fixture it holds a handful; on this
disk it held **1,639,834**. A held region is 256 MiB, which is 65,536 candidate blocks, so a single
node of the walk asked the question 65,536 times and each answer scanned 1.6 million ranges —
about 10¹¹ comparisons over 26 MB, far past last-level cache. A candidate visits up to 320 nodes.

Nothing about the *answer* needed the walk: the ranges are positions on a medium, and one that lies
outside the region being searched can never match a block inside it.

Two details made it worse. The hypothesis ceiling was tested *after* the two `covers` calls, so a
block that was already claimed cost a full scan and did not count as an attempt — the budget that
exists to bound the stage could never be reached. And `Shared` was built with `spoken_for.to_vec()`,
copying 26 MB per candidate.

This is why the previous run reported 254 attempts and 3 recoveries: those 254 are the gap searches
of the first region, which do not consult the claimed set. The graph walk of that same region
consumed the remaining two hours.

The lab suite passes because a fixture recovers dozens of artifacts. `recovery_rate.rs` measures
87 % on two and three fragments with the claimed set practically empty. The algorithm was never
wrong; the scale of one of its inputs was never exercised.

## Change

- `Claimed` replaces the raw `Vec<ByteRange>` in the walk: sorted and merged once, then answered by
  binary search. Merging is what lets the search stop at a single candidate range.
- The engine narrows the claimed set **to the region** before the walk sees it, once per region
  rather than once per candidate. A hypothesis cannot reach past what was held, so a claim outside
  it cannot matter — on a 256 MiB region of this disk that is a few hundred ranges instead of 1.6
  million.
- The hypothesis ceiling is tested before the claim tests, so a spent budget ends the loop instead
  of going on paying for questions it can no longer act on.
- `Shared` no longer copies the whole claimed set per candidate.
- `reassemble_broken`'s per-region body moves into `search_region`, which is what made the
  narrowing a single place rather than three.

## Proof

`crates/argos_carve/tests/reassemble_scale.rs` asserts the shape rather than a duration, so it means
the same in debug and release: extents claimed elsewhere must not change what is recovered, a walk
against 400,000 of them must not cost more than one against none, and four times the claimed set
must not cost four times the time. All three fail before the change — the first by 1.481 s against
0.015 s — and pass after.

Re-measured on the same instrument as the table above: at 8,192 blocks and 400,000 claimed extents,
**5.759 s → 0.066 s**, and what remains does not grow with the claimed set.

`recovery_rate.rs` is unchanged and still reports 87 % / 87 % / 25 % with **0 fabricated** on every
pattern. Nothing here touches the oracle that decides whether an assembly is accepted — only what
the search spends to reach it.
