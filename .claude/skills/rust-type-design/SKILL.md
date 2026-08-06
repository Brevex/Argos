---
name: rust-type-design
description: Use BEFORE defining or changing any struct, enum, trait, newtype, public function signature, constructor or builder — including deciding parameter types, generics vs `dyn Trait`, which traits to derive, and how a type is constructed. Covers M-STRONG-TYPES, M-STRONG-TYPES-GUARD, M-PUBLIC-DEBUG, M-PUBLIC-DISPLAY, M-COLLECTION-TRAITS, M-DI-HIERARCHY, M-AVOID-WRAPPERS, M-SIMPLE-ABSTRACTIONS, M-ESSENTIAL-FN-INHERENT, M-INIT-BUILDER, M-INIT-CASCADED, M-BUILD-RESULT, M-PARAMETER-CONSISTENCY, M-IMPL-ASREF, M-IMPL-IO, M-IMPL-RANGEBOUNDS.
---

# Types, traits, signatures and construction

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## Newtypes must guard their invariant

`M-STRONG-TYPES-GUARD` — a newtype that enforces nothing is worse than no newtype, because every
caller re-checks:

```rust,ignore
pub struct SectorSize(u32);

impl SectorSize {
    /// Fails unless `bytes` is a power of two in 512..=4096.
    pub fn from_u32(bytes: u32) -> Result<Self, GeometryError> { … }   // required, fallible
    pub const fn new(bytes: u32) -> Self { … }                          // optional, panicking, const
}
```

- [ ] At least one fallible constructor (`from_*` → `Result`)
- [ ] Extra panicking constructor is fine and should be `const` (so `const { SectorSize::new(511) }`
      fails at compile time)
- [ ] Weak → strong conversions are `TryFrom`/`FromStr`; **never** `impl From<u32>`
- [ ] Use the strongest `std` type available, as early as possible (`M-STRONG-TYPES`): `PathBuf`/
      `Path` for anything OS-facing, never `String`. Plain numbers stay plain at API boundaries.

## Traits derived on public types

- [ ] `Debug` on every public type (`M-PUBLIC-DEBUG`). Sensitive payloads get a **manual** `Debug`
      that redacts, plus a unit test proving the secret never renders.
- [ ] `Display` when humans read it (`M-PUBLIC-DISPLAY`) — mandatory for error types.
- [ ] Eagerly derive the rest where they make sense: `Copy`, `Clone`, `Eq`, `PartialEq`, `Ord`,
      `PartialOrd`, `Hash`, `Default`.
- [ ] Custom collections implement the full iterator family (`M-COLLECTION-TRAITS`): `Iter`,
      `IterMut`, `IntoIter`, `iter()`, `iter_mut()`, `IntoIterator` for value/`&`/`&mut`,
      `FromIterator`, `Extend`, and a truthful `size_hint()`.

## Signatures

- [ ] Accept `impl AsRef<Path>` / `AsRef<str>` / `AsRef<[u8]>` when you don't need ownership
      (`M-IMPL-ASREF`) — but never put those bounds on *types*, only on functions.
- [ ] Accept `impl Read`/`impl Read + Seek`/`impl Write` instead of `File` (`M-IMPL-IO`, sans-IO).
      This is what makes carvers testable over `Cursor<Vec<u8>>`.
- [ ] Accept `impl RangeBounds<T>` instead of `(low, high)` pairs (`M-IMPL-RANGEBOUNDS`).
- [ ] Same conceptual parameters in the same order everywhere (`M-PARAMETER-CONSISTENCY`):
      call-specific first, ubiquitous last, closures always last, at most one closure.
- [ ] No `Arc`, `Rc`, `Box`, `RefCell`, `Mutex` in public signatures (`M-AVOID-WRAPPERS`) — hide
      them behind `&T`, `&mut T`, `T`.
- [ ] Essential functionality is **inherent**; trait impls forward to it (`M-ESSENTIAL-FN-INHERENT`).

## Abstraction ladder for dependencies

`M-DI-HIERARCHY` — escalate only when forced:

1. **Concrete type** — the default.
2. **Enum** dispatching to native vs mocked, when the only other impl exists for testing
   (this is the `M-MOCKABLE-SYSCALLS` pattern; see `rust-testing`).
3. **Narrow traits** as generic params (`fn scan(src: impl ReadSectors)`), when users bring their own.
4. **`dyn Trait`** only when generics start nesting — and then behind a named wrapper struct, not
   raw `Arc<dyn T>` in the API.

Never accept a `FooBuilder` as a parameter; ask for `impl Fn() -> Foo` if you need repeated
construction. Keep visible type parameters shallow (`M-SIMPLE-ABSTRACTIONS`): `Scanner<Device>` is
acceptable, `Scanner<Device<Cache<File>>>` is not.

## Construction

- ≤ 2 optional parameters → inherent `new` / `with_x` / `with_x_y` (`M-INIT-BUILDER`)
- ≥ 4 permutations → `Foo::builder()` returning `FooBuilder`, chainable, setters named `x()` not
  `set_x()`, terminating in `.build()`; no public `FooBuilder::new()`
- Setters never fail; **all validation happens in `.build() -> Result<..>`** (`M-BUILD-RESULT`)
- Required params go into the builder constructor, grouped in a `FooDeps` struct taken as
  `impl Into<FooDeps>`
- ≥ 4 raw parameters → cascade through helper types instead (`M-INIT-CASCADED`)

## Argos application

- Model the disk domain with guarded newtypes, not `u64` everywhere: `Lba`, `ByteOffset`,
  `SectorSize`, `ClusterCount`, `Signature`. Mixing an LBA with a byte offset is the classic
  data-recovery bug, and the type system can make it unrepresentable.
- Carvers and filesystem parsers take `impl Read + Seek`, so unit tests run on synthetic images in
  memory and the production path passes a real device handle. No carver should know what a `File` is.
- `ScanConfig` (device, sector size, offset range, signature set, output dir, recursion depth,
  parallelism) is well past 4 permutations → builder with validation in `.build()`.
- Range-based APIs (`scan(0..)`, `scan(offset..end)`) use `impl RangeBounds<ByteOffset>`.
- Recovered artifacts may contain personal data: any type holding file content gets a manual
  redacting `Debug` (see also `rust-telemetry`).

## Full rule text

[reference.md](reference.md) — 16 rules, listed in the file header.
