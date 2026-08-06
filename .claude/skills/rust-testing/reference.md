# Tests and mockable I/O — guideline reference

<!-- Rule text adapted from the Pragmatic Rust Guidelines, Copyright (c) Microsoft Corporation, MIT license. -->

**This file is the authoritative text of these rules for the Argos project.** It is
the source, not a copy: treat the wording here as normative and edit it directly when
the project's standard changes. Cite rules by id (e.g. `M-INTEGRATION-TESTS`).

Rules in this file (4): `M-INTEGRATION-TESTS`, `M-MOCKABLE-SYSCALLS`, `M-TAUTOLOGICAL-TESTS`, `M-TEST-UTIL`

Decision layer and Argos-specific application: [SKILL.md](SKILL.md). Full rule map: [guidelines index](../../guidelines-index.md).

---

## Tests do not assert ground truth (M-TAUTOLOGICAL-TESTS) { #M-TAUTOLOGICAL-TESTS }

<why>tests that add value, not noise.</why>

Unit tests should verify meaningful behavior instead of repeating foundational definitions.

Agents frequently produce tests that re-state the expected value from the same logic the code under test uses, or that simply mirror the implementation's branches. Such tests pass by construction, provide virtually no value, but increase the noise floor of subsequent changes:

```rust
const CHECKPOINTS: [u32; 4] = [0, 90, 180, 270];

#[test]
fn checkpoints_are_correct() {
    assert_eq!(CHECKPOINTS, [0, 90, 180, 270]);
}
```

Where these are used to satisfy mutation tests, the mutation test should be skipped instead.

Instead, a meaningful test would check a property the constants are supposed to satisfy, for example that they are evenly spaced, monotonically increasing, or impose some direction in related logic.

## Integration tests live under `tests/` (M-INTEGRATION-TESTS) { #M-INTEGRATION-TESTS }

<why>clean code files.</why>

Tests that only touch public API surface are _integration tests_ and belong under `tests/`, not `mod tests {}`.

In projects with coverage targets, it is not uncommon for `src/` files to contain more testing code than actual business logic. This can make browsing and understanding the code harder both in IDEs and PRs. Likewise, if a testing goal can be achieved through either an integration test or a unit test, the former is always preferred.

## I/O and system calls are mockable (M-MOCKABLE-SYSCALLS) { #M-MOCKABLE-SYSCALLS }

<why>testable edge cases that are otherwise hard to evoke.</why>

Any user-facing type doing I/O, or sys calls with side effects, should be mockable to these effects. This includes file and
network access, clocks, entropy sources and seeds, and similar. More generally, any operation that is

- non-deterministic,
- reliant on external state,
- depending on the hardware or the environment,
- is otherwise fragile or not universally reproducible

should be mockable.

> ### <tip></tip> Mocking Allocations?
>
> Unless you write kernel code or similar, you can consider allocations to be deterministic, hardware independent and practically
> infallible, thus not covered by this guideline.
>
> However, this does _not_ mean you should expect there to be unlimited memory available. While it is ok to
> accept caller provided input as-is if your library has a _reasonable_ memory complexity, memory-hungry libraries
> and code handling external input should provide bounded and / or chunking operations.

This guideline has several implications for libraries, they

- should not perform ad-hoc I/O, i.e., call `read("foo.txt")`
- should not rely on non-mockable I/O and sys calls
- should not create their own I/O or sys call _core_ themselves
- should not offer `MyIoLibrary::default()` constructors

Instead, libraries performing I/O and sys calls should either accept some I/O _core_ that is mockable already, or provide mocking functionality themselves:

```rust, ignore
let lib = Library::new_runtime(runtime_io); // mockable I/O functionality passed in
let (lib, mock) = Library::new_mocked(); // supports inherent mocking
```

Libraries supporting inherent mocking should implement it as follows:

```rust, ignore
pub struct Library {
    some_core: LibraryCore // Encapsulates syscalls, I/O, ... compare below.
}

impl Library {
    pub fn new() -> Self { ... }
    pub fn new_mocked() -> (Self, MockCtrl) { ... }
}
```

Behind the scenes, `LibraryCore` is a non-public enum, like the one used to abstract a runtime, that either dispatches
calls to the respective sys call, or to an mocking controller.

```rust, ignore
// Dispatches calls either to the operating system, or to a
// mocking controller.
enum LibraryCore {
    Native,

    #[cfg(feature = "test-util")]
    Mocked(mock::MockCtrl)
}

impl LibraryCore {
    // Some function you'd forward to the operating system.
    fn random_u32(&self) {
        match self {
            Self::Native => unsafe { os_random_u32() }
            Self::Mocked(m) => m.random_u32()
        }
    }
}


#[cfg(feature = "test-util")]
mod mock {
    // This follows the M-SERVICES-CLONE pattern, so both `LibraryCore` and
    // the user can hold on to the same `MockCtrl` instance.
    pub struct MockCtrl {
        inner: Arc<MockCtrlInner>
    }

    // Implement required logic accordingly, usually forwarding to
    // `MockCtrlInner` below.
    impl MockCtrl {
        pub fn set_next_u32(&self, x: u32) { ... }
        pub fn random_u32(&self) { ... }
    }

    // Contains actual logic, e.g., the next random number we should return.
    struct MockCtrlInner {
        next_call: u32
    }
}
```

Runtime-aware libraries that already dispatch through a runtime enum should extend that enum instead:

```rust, ignore
enum Runtime {
    #[cfg(feature="tokio")]
    Tokio(tokio::Tokio),

    #[cfg(feature="smol")]
    Smol(smol::Smol)

    #[cfg(feature="test-util")]
    Mock(mock::MockCtrl)
}
```

As indicated above, most libraries supporting mocking should not accept mock controllers, but return them via parameter tuples,
with the first parameter being the library instance, the second the mock controller. This is to prevent state ambiguity if multiple
instances shared a single controller:

```rust, ignore
impl Library {
    pub fn new_mocked() -> (Self, MockCtrl) { ... } // good
    pub fn new_mocked_bad(&mut MockCtrl) -> Self { ... } // prone to misuse
}
```


## Test utilities are feature gated (M-TEST-UTIL) { #M-TEST-UTIL }

<why>production builds that cannot bypass safety checks.</why>

Testing functionality must be guarded behind a feature flag. This includes

- mocking functionality ([M-MOCKABLE-SYSCALLS]),
- the ability to inspect sensitive data,
- safety check overrides,
- fake data generation.

We recommend you use a single flag only, named `test-util`. In any case, the feature(s) must clearly communicate they are for testing purposes.

```rust, ignore
impl HttpClient {
    pub fn get() { ... }

    #[cfg(feature = "test-util")]
    pub fn bypass_certificate_checks() { ... }
}
```

[M-MOCKABLE-SYSCALLS]: #M-MOCKABLE-SYSCALLS

