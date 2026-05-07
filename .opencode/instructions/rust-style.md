# Rust Style and Discipline

Rules in this file are binding for all Rust code in Argos.

## No comments

Source files contain zero comments. No `//`, `/* */`, or `///`. If you reach for a comment, refactor: extract a function with a descriptive name, introduce a newtype, or split a complex expression into named bindings. Architectural intent lives in `docs/`.

## No duplication, no dead code

Every line in the crate is load-bearing and integrated.

- **Search before adding.** Before writing a new function, grep the crate for an existing one that does the same thing or could with a small generalization. If two call sites would benefit from a shared helper, extract on the second occurrence, not the third.
- **No "almost the same" forks.** A function that is a copy of another with one parameter changed is duplication. Generalize via a parameter or a generic.
- **Delete unused symbols.** If a function, type, constant, or module is not reached from `main`, the bridge, a test, or a benchmark, delete it. The same applies to unused dependencies and feature flags.
- **No `#[allow(dead_code)]` in non-test code.** If you cannot avoid it, the code does not belong here.
- **Speculative abstractions are dead code.** Traits with one impl, generic parameters with one instantiation, and configuration toggles never flipped are removed.
- **Crate-level lints (in `lib.rs`):**
  ```
  #![deny(dead_code)]
  #![deny(unused_imports)]
  #![deny(unused_variables)]
  #![deny(unused_mut)]
  #![deny(unused_must_use)]
  #![deny(unreachable_code)]
  #![deny(unreachable_patterns)]
  ```
- **CI gates:** `cargo machete` (unused dependencies), `cargo udeps` (unused crate features), `cargo clippy -- -D warnings -W clippy::redundant_clone -W clippy::needless_pass_by_value`.
- **Public surface:** `lib.rs` re-exports only what tests, benchmarks, or the bridge consume. Anything `pub` that nothing outside the module reads is downgraded to `pub(crate)` or `pub(super)`.

## Allocation

- Every `Vec`, `String`, or `HashMap` instantiated near a loop is preceded by `::with_capacity(n)` where `n` is computable, or its allocation is hoisted out of the loop and reused via `clear()`.
- For byte buffers, prefer `bytes::BytesMut` or a pooled allocator. Per-fragment allocation is forbidden in carving and validation paths.
- For temporary aggregations during a single recovery pass, use a bump allocator (`bumpalo`) scoped to the pass.
- Prefer iterators that do not collect. When `collect` is unavoidable, type-annotate the destination so capacity hinting is visible.

## Borrowing and ownership

- Function parameters are `&str`, `&Path`, `&[u8]`, or `&T` unless ownership transfer is required.
- `Cow<'_, T>` is the default when a function might or might not allocate.
- `clone()` requires structural justification (e.g., the value crosses a thread boundary and is not `Arc`-shareable).

## Generics over dyn

In hot paths, dispatch is monomorphized. `Box<dyn Trait>` and `&dyn Trait` are confined to:
- Plugin-style boundaries (none currently exist).
- The bridge module, where heterogeneous responses must be uniformly serialized.

## I/O

- Raw-device reads are sector-aligned, with buffer sizes that are multiples of the logical sector size (typically 4 KiB), and use direct I/O (`O_DIRECT` / `FILE_FLAG_NO_BUFFERING`).
- Block transfer size for sequential scans defaults to 1 MiB. Smaller sizes are a code smell.
- File I/O on the output side uses `BufWriter` with a buffer matching the output filesystem's block size.

## Concurrency

- CPU-bound work (carving, validation, matching, reassembly) runs on a `rayon` pool. The pool is constructed once at startup with explicit thread count.
- I/O concurrency at the bridge boundary uses `tokio` with explicit task budgets.
- The two pools never share work-stealing queues.
- Synchronization preference order: lock-free / atomics → `parking_lot::RwLock` → `parking_lot::Mutex` → channel. `std::sync::Mutex` is forbidden.
- Sharing across threads: `Arc` is acceptable; `Arc<Mutex<T>>` requires justification.

## Unsafe

- Allowed only when a safe alternative would cost a measured performance regression on a benchmarked path.
- Each `unsafe` block is wrapped in a function whose name and type signature make the invariant structural. Example: instead of an `unsafe { ptr.add(i) }` with a `// SAFETY:` comment, define `read_at(buf: &AlignedSlice, i: SectorIndex) -> u8` whose constructor of `AlignedSlice` already enforces alignment.
- `unsafe` blocks are never wider than the minimum needed.
- `cargo geiger` runs in CI with a budget of zero new unsafe lines per crate without an ADR.

## Errors

- Crate-wide error enum `ArgosError` in `src/error.rs`, derived with `thiserror`.
- All public functions return `Result<T, ArgosError>`.
- `?` is the only error propagation idiom.
- `unwrap()`, `expect()`, and `panic!` are forbidden in non-test code, with one exception: invariants that, if violated, indicate memory corruption (e.g., a length read from a typed buffer that the type system cannot express). These cases are extracted into a single helper and are reviewed.

## Tests

- Unit tests live alongside their modules in `#[cfg(test)] mod tests` blocks.
- Integration tests live under `tests/`.
- Parsers (headers, footers, format-specific validators) are exercised with `proptest`. Property tests are not optional for byte-level parsers.
- Pattern matching, Huffman validation, and CRC routines are fuzzed with `cargo-fuzz`.
- Hot-path changes require a `criterion` benchmark in the same PR.

## Lints

- `cargo clippy --all-targets -- -D warnings` is a hard gate.
- `clippy::pedantic` warnings are evaluated case-by-case; suppressions are scoped (`#[allow(...)]` on the smallest item) and require justification in the PR description.
- `#[deny(unsafe_op_in_unsafe_fn)]`, `#[deny(missing_debug_implementations)]`, `#[forbid(trivial_casts)]` are crate-level.

## Public API

- The crate exposes a thin library API in `lib.rs` for testing and benchmarking. The Tauri bridge consumes this API.
- All public types are `Debug`. All public types not containing references are `Send + Sync` unless their semantics require otherwise.
- Public functions have stable, named parameters; avoid `bool` parameters in favor of named enums.
