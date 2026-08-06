---
name: rust-testing
description: Use BEFORE writing or reviewing tests, `#[test]`, `mod tests`, files under `tests/`, fixtures, or any mocking/faking of I/O, syscalls, clocks or randomness — and when designing a type that performs I/O so it stays testable. Covers M-INTEGRATION-TESTS, M-TAUTOLOGICAL-TESTS, M-MOCKABLE-SYSCALLS, M-TEST-UTIL.
---

# Tests and mockable I/O

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## Where a test goes

`M-INTEGRATION-TESTS` — if it only touches the public API, it is an integration test and belongs in
`tests/`, not in `mod tests {}`. When a goal can be reached either way, prefer `tests/`. Keeps `src/`
readable in IDEs and PRs.

Unit tests in `src/` are for private behaviour that genuinely cannot be reached from outside.

## Tests must be able to fail

`M-TAUTOLOGICAL-TESTS` — a test that restates a constant, or mirrors the branches of the code under
test, passes by construction and only raises the noise floor:

```rust,ignore
// Worthless: asserts the definition against itself.
#[test] fn jpeg_signature_is_correct() { assert_eq!(JPEG_SOI, [0xFF, 0xD8, 0xFF]); }

// Meaningful: asserts a property the data must satisfy.
#[test] fn signatures_are_unique_and_nonempty() { … }
#[test] fn carver_finds_jpeg_at_unaligned_offset() { … }
```

Test observable behaviour on real inputs. If a test exists only to satisfy a mutation-coverage
target, skip the mutation instead.

## I/O must be mockable

`M-MOCKABLE-SYSCALLS` — anything non-deterministic, environment-dependent or hard to reproduce
(device reads, file access, clocks, entropy) has to be mockable. Concretely, a library must not:

- perform ad-hoc I/O (`read("/dev/sda")` buried in logic),
- build its own non-mockable syscall core,
- offer a `Default` constructor for an I/O type.

Two accepted shapes:

```rust,ignore
let dev = Device::open(path)?;              // real
let (dev, ctrl) = Device::new_mocked();     // mocked, controller returned as a tuple
```

The core is a private enum dispatching to the OS or to a mock controller:

```rust,ignore
enum DeviceCore {
    Native(NativeDevice),
    #[cfg(feature = "test-util")]
    Mocked(mock::MockCtrl),   // Arc<Inner> internally, so lib and test share one instance
}
```

Return the controller from the constructor — never accept one as a parameter, which lets two
instances silently share state.

## Test utilities are feature gated

`M-TEST-UTIL` — mocks, fake data generators, sensitive-data inspectors and safety-check overrides
live behind a single `test-util` feature, so a production build cannot reach them.

## Argos application

- **CI has no disks.** `M-MOCKABLE-SYSCALLS` is therefore the enabling rule of the whole project:
  device access must be an enum core behind `test-util`, or nothing below the CLI is testable.
- Prefer real-shaped fixtures over mocks where possible: small synthetic disk images (a few hundred
  KB) built by a generator in `tests/fixtures/`, covering fragmented files, partially overwritten
  headers, missing footers, unaligned offsets and truncated tails. Commit the generator, not
  gigabytes of binary.
- Carvers take `impl Read + Seek` (see `rust-type-design`), so most tests need no mocking at all —
  a `Cursor<Vec<u8>>` is the test device.
- Recovery correctness is a property: *every byte written to output must exist at the claimed offset
  in the source image*. Assert that, not the internal branch structure.
- Never test against a real `/dev/*` node in CI, and never let a test open a device writable.
- Fixtures must not contain real personal photos. Generate synthetic images.

## Full rule text

[reference.md](reference.md) — `M-INTEGRATION-TESTS`, `M-TAUTOLOGICAL-TESTS`,
`M-MOCKABLE-SYSCALLS`, `M-TEST-UTIL`.
