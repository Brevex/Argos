---
name: rust-performance
description: Use BEFORE writing or optimising any hot loop, buffer handling, allocation, collection construction, hash map, or data layout of a frequently instantiated type — and when adding benchmarks or profiling. Covers M-HOTPATH, M-AVOID-INDIRECTION, M-BOX-DST, M-FAST-HASHER, M-INITIAL-CAPACITY, M-MEM-REUSE, M-SHRINK-TO-FIT.
---

# Hot paths, memory and allocations

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## Know your hot path first

`M-HOTPATH` — decide early whether the crate is performance relevant; for Argos it is. Then:

- [ ] Identify the hot paths and write benchmarks around them (`criterion` or `divan`)
- [ ] Run a profiler regularly for CPU **and** allocation insight
- [ ] Document where the performance-sensitive areas are, so contributors don't wreck them
- [ ] `[profile.bench] debug = 1` so profiles have symbols

Measure before optimising, and never reach for `unsafe` for speed without a benchmark
(see `rust-unsafe-ffi`).

## The allocation rules

| Rule | Do |
| --- | --- |
| `M-INITIAL-CAPACITY` | `Vec::with_capacity(n)` when the size is known; prefer `collect()` over `push` loops — it inherits `size_hint` |
| `M-MEM-REUSE` | Let callers own and reuse buffers; the canonical reuse method is `.clear()`. Design APIs like `read_into(&mut buf)`, not `-> Vec<u8>` per item |
| `M-SHRINK-TO-FIT` | Long-lived collections built without a reservation get `shrink_to_fit()` — growth by doubling can waste ~2x |
| `M-BOX-DST` | Frequently instantiated, immutable, internal sequences become `Box<[T]>` / `Arc<str>`: drops the capacity word, shrinks the handle by 1/3 |
| `M-FAST-HASHER` | For trusted internal keys use `foldhash`/`FxHash`; the std hasher's DoS resistance is only needed for adversarial keys |
| `M-AVOID-INDIRECTION` | Don't reflexively `Arc` nested types. Lift hot, cacheable fields up so a hot check is one memory access, not three DRAM round-trips |

Common wins the document calls out: repeated re-allocations (especially `format!`-assembled strings),
short-lived allocations that could be bump-allocated, cloning collections, re-hashing equal data.
Anecdotally ~15% on hot paths from string fixes alone.

## Argos application

The hot path is the **signature scanner**: bytes/second across a whole device. Everything else is
noise by comparison.

- **Buffer discipline:** allocate scan buffers once per worker and `.clear()`/overwrite per block.
  A `Vec<u8>` per read is the single easiest way to make Argos slow.
- **Read size matters more than clever code:** large sequential reads (≥ 1 MiB), aligned when using
  `O_DIRECT`. Sector-by-sector reads dominate every micro-optimisation you could make.
- **Signature matching:** don't loop naïvely over N signatures × M bytes. Match on the first bytes
  with a multi-pattern approach (e.g. Aho–Corasick or a 256-entry first-byte table) and only then
  confirm the full magic.
- **Handle straddling:** signatures crossing block boundaries need an overlap of `max_signature_len
  - 1` carried between blocks — get this right before optimising anything.
- **Offset maps:** `foldhash`/`FxHash` for internal offset → candidate maps; keys come from our own
  scan, not from an attacker.
- **`Finding`s are numerous and immutable** once produced → `Box<[u8]>`/`Box<str>` for their fields,
  and `shrink_to_fit()` the result set before reporting.
- Keep telemetry out of the inner loop (see `rust-telemetry`, `M-LOG-OVERHEAD`): log per batch or
  per gigabyte, never per sector.

## Full rule text

[reference.md](reference.md) — `M-HOTPATH`, `M-AVOID-INDIRECTION`, `M-BOX-DST`, `M-FAST-HASHER`,
`M-INITIAL-CAPACITY`, `M-MEM-REUSE`, `M-SHRINK-TO-FIT`.
