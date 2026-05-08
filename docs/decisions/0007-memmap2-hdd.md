# ADR 0007 — memmap2 for random-access block I/O in HDD recovery

- **Status:** Accepted
- **Date:** 2026-05-08
- **Supersedes:** ADR 0003 (adds implementation detail)

## Context

ADR 0003 established PUP + SHT as the HDD recovery algorithm. During implementation, a concrete I/O access pattern question arose: PUP requires reading arbitrary non-contiguous blocks for multiple candidate files simultaneously, while the existing `BlockReader` API is strictly sequential (streaming `&[u8]` slices).

## Decision

The HDD pipeline uses `memmap2::Mmap` to map the entire source device into the process address space. The PUP stage then accesses individual blocks via direct byte-slice indexing into the mmap (`&mmap[start..end]`).

The SSD pipeline continues to use the sequential `BlockReader` with `AlignedBuf` and `O_DIRECT`.

## Rationale

- **Random access.** PUP's core loop pops the best candidate, evaluates multiple adjacent blocks, and pushes extended candidates back. Each evaluation requires reading a block at an arbitrary offset. Sequential streaming cannot satisfy this pattern without complex caching or seeking.
- **Zero-copy.** `mmap` returns `&[u8]` slices directly into the kernel page cache without an intermediate user-space buffer. This avoids per-block `Vec` allocations.
- **No `unsafe` proliferation.** `memmap2` is a pure-Rust wrapper around `mmap(2)`. The single `unsafe` block (the `mmap` call itself) is isolated in `memmap2`; our code never dereferences raw pointers directly.
- **Single open.** Combined with ADR 0007-runner (one `SourceDevice` per session), the HDD path opens the device exactly once: for `mmap`. The SSD path also opens once, but uses `pread` via `BlockReader`.

## Alternatives considered

- **Cache LRU + `pread`:** maintain a bounded `BTreeMap<u64, Vec<u8>>` and fill it on demand via `pread`. Rejected because it duplicates kernel page cache functionality and adds per-block allocation overhead.
- **`seek` + `read` on a single `File`:** rejected because `seek`/`read` on a raw device with `O_DIRECT` requires aligned buffers and serializes access; multiple candidates would contend on a single cursor.
- **Thread-per-candidate with independent `File` handles:** rejected because it would require N concurrent device opens and N kernel buffers, violating the "single open" invariant.

## Consequences

- The HDD path requires the source device to be representable as a file (which all block devices are on Linux/Windows/macOS).
- `mmap` on extremely large devices (multi-TB) can exhaust virtual address space on 32-bit targets. Argos targets 64-bit only; this is acceptable.
- `mmap` bypasses `O_DIRECT` alignment guarantees. The SSD path retains `O_DIRECT` for its sequential reads; the HDD path relies on `mmap` page alignment.
- Device class detection (`io::detect_device_class`) is load-bearing: misclassifying an SSD as HDD triggers `mmap` instead of streaming. This wastes time but does not corrupt data.
- The `unsafe` budget increases by the `mmap` call itself, justified by the performance gain and isolation in `memmap2`.
