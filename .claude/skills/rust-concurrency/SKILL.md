---
name: rust-concurrency
description: Use BEFORE adding threads, a thread pool, rayon, channels, `async`/`await`, a runtime, shared mutable state, a `static`/`thread_local`, or a heavyweight long-lived "service" type — and when deciding how to partition work across workers. Covers M-TYPES-SEND, M-ASYNC-FN, M-AVOID-STATICS, M-YIELD-POINTS, M-THROUGHPUT, M-ASYNC-STACK-SIZE, M-SERVICES-CLONE.
---

# Concurrency, async and shared state

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## Optimise for throughput

`M-THROUGHPUT` — the key metric is **items per CPU cycle**, not latency of a single item:

- [ ] Partition work into reasonable chunks ahead of time
- [ ] Let each worker own its slice independently
- [ ] Sleep or yield when there is no work — never hot-spin for one more item
- [ ] Design and consume **batched** APIs
- [ ] Exploit cache locality
- [ ] Share state only when sharing costs less than recomputing

Explicitly not: work stealing to balance individual items, or per-item processing where batching is
possible.

## Statics are duplicated state

`M-AVOID-STATICS` — a `static` may exist in several linked copies (different crate versions), so any
code whose correctness depends on a consistent view must not use one. Statics also break unit tests
and contend in thread-per-core designs. Statics purely for performance are fine.

Pass state explicitly instead; the "single instance per run" pattern is a value you construct in
`main` and hand down, not a global.

## Services are cheap to clone

`M-SERVICES-CLONE` — long-lived service-like types implement shared-ownership `Clone` via the
`Arc<Inner>` pattern, so dependents each hold a handle:

```rust,ignore
struct DeviceInner { … }

#[derive(Clone)]
pub struct Device { inner: Arc<DeviceInner> }
```

`Clone` must not deep-copy. Note this is the one sanctioned place for `Arc` — it stays *inside* the
type, never in the public signature (`rust-type-design`, `M-AVOID-WRAPPERS`).

## Send

`M-TYPES-SEND` — public types should be `Send`, and any future we produce must be. Assert it:

```rust,ignore
const fn assert_send<T: Send>() {}
const _: () = assert_send::<Device>();
```

A `!Send` type held across an `.await` infects the whole future. `!Send` is acceptable only for
types used instantaneously.

## If (and only if) we go async

- `async fn` over `-> impl Future` when both work (`M-ASYNC-FN`)
- Track future sizes on hot paths with `size_of_val`; large arguments get embedded in the future
  type (`M-ASYNC-STACK-SIZE`) — return `impl Future` and do setup outside the `async` block to keep
  it small
- Long CPU-bound work needs `yield_now().await` roughly every 10–100 µs (`M-YIELD-POINTS`), or it
  starves every other task on the runtime

## Argos application

**A device scan is CPU/IO-bound batch work, not a concurrency puzzle.** Default to threads +
`rayon`-style partitioning over `async`; async buys little when the workload is sequential bulk reads
plus CPU matching. Revisit only if we add network or many-device orchestration. Until that decision
is made, the async rules above are conditional; the rest apply now.

- Partition the device into byte ranges per worker (`M-THROUGHPUT`), each with its own reusable
  buffer (`rust-performance`) and its own file descriptor — with the overlap window carried so
  signatures straddling a partition boundary are not lost.
- Aggregate findings per batch through a channel; never lock a shared `Vec` per hit.
- No global scan counters or config in `static`s (`M-AVOID-STATICS`) — pass a `Device`/`ScanConfig`
  handle down. Statics also make the two-scan-at-once case silently wrong.
- `Device`, and anything an examiner constructs once and shares, follows the `Arc<Inner>` clone
  pattern and asserts `Send`.
- Progress reporting is a batched event, never a per-sector message.

## Full rule text

[reference.md](reference.md) — `M-TYPES-SEND`, `M-ASYNC-FN`, `M-AVOID-STATICS`, `M-YIELD-POINTS`,
`M-THROUGHPUT`, `M-ASYNC-STACK-SIZE`, `M-SERVICES-CLONE`.
