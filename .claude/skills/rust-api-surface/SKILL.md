---
name: rust-api-surface
description: Use BEFORE naming a type, trait, function or module, deciding `pub` vs `pub(crate)`, adding a `pub use`/re-export, laying out `mod`s or a crate root, or porting a design from another language (C#/Java/C++/Python). Covers M-SHORT-NAMES, M-WEASEL-WORDS, M-REGULAR-FN, M-SINGLE-ITEM-PATH, M-NO-GLOB-REEXPORTS, M-NO-PRELUDE, M-BALANCED-MODULES, M-FOREIGN-REEXPORTS, M-DONT-LEAK-TYPES, M-DESIGN-FOR-AI, M-RUST-SHAPED.
---

# Naming, modules and public surface

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## Naming

- [ ] At most two short words (`ScanConfig`, not `GlobalScanConfiguration`) — `M-SHORT-NAMES`
- [ ] No module/crate name baked into the item: `carve::Jpeg`, not `carve::JpegCarver`; callers
      disambiguate locally (`fn convert(fs::Id) -> carve::Id`)
- [ ] Abbreviations preferred: `CallbackFn` over `CallbackFunction`
- [ ] No weasel words — `Service`, `Manager`, `Factory`, `Helper`, `Util` (`M-WEASEL-WORDS`).
      A type handling many findings is `Findings`; if it does something specific, say what:
      `FindingWriter`. Lifecycle is `Drop`'s job, not a manager's. The Rust name for a factory is
      `Builder`.
- [ ] Functions that don't belong to a receiver are free functions, not associated functions
      (`M-REGULAR-FN`). `impl` blocks are for constructors and `&self` methods.

## Modules

`M-BALANCED-MODULES` — treat it as menu design: the few essential items in the crate root, the rest
grouped by **use case**. A `argos_carve` crate has `Carver` in the root; modules are named `jpeg`,
`png`, `signature` — never `traits`, `types` or `errors`. Neither extreme is acceptable: a flat root
with dozens of items, or a root with nothing but `mod` declarations.

Modules are also the right place for subsystem documentation (see `rust-docs`).

## Visibility and re-exports

- [ ] One public path per item (`M-SINGLE-ITEM-PATH`): if `carve::Jpeg` is public, there is no
      `crate::Jpeg` alias. This rule is broken most often by agents "simplifying" a refactor by
      keeping the old path alive — delete the old path instead.
- [ ] No glob re-exports (`M-NO-GLOB-REEXPORTS`); list items explicitly. The only accepted use is
      forwarding a per-target HAL module (`#[cfg(target_os = "linux")] pub use linux::*;`).
- [ ] No `prelude` module and nothing meant to be imported with `*` (`M-NO-PRELUDE`). Wanting a
      prelude is a symptom of bad module layout.
- [ ] Don't re-export other crates' items (`M-FOREIGN-REEXPORTS`); users depend on the original
      crate. Exceptions: umbrella crates, crates split for technical reasons, macro path helpers.
- [ ] Prefer `std` types in public APIs (`M-DONT-LEAK-TYPES`). A third-party type in a signature
      becomes part of our contract forever. Behind a feature flag it is acceptable (e.g. `serde`);
      without one, only for a substantial ecosystem benefit.

## Don't port foreign shapes

`M-RUST-SHAPED` — domain logic (how you locate a JPEG footer) translates between languages; technical
constructs do not. Error handling, task/thread management, ownership, interfaces-vs-traits and the
absence of OO all need Rust-native answers. A striking structural resemblance to a C#/Java/Python
implementation signals an architectural problem: `throw_if_null()` never makes sense here.

## Designing for agents

`M-DESIGN-FOR-AI` — the same things that make an API good for people make it usable by agents:
idiomatic patterns, thorough module and item docs, runnable examples, strong types instead of
primitive obsession, testable APIs, and real test coverage over observable behaviour.

## Argos application

- Root of `argos_carve`: `Carver`, `Finding`. Modules: `jpeg`, `png`, `heic`, `signature`.
- Root of `argos_device`: `Device`, `DeviceInfo`. Modules: `geometry`, `hal`.
- Names to avoid on sight: `RecoveryManager`, `ScannerService`, `CarverFactory`, `FileUtils`,
  `DiskHelper`.
- Prefer `Lba` in `argos_core` over `LbaAddress`, and let callers write `core::Lba` if ambiguous.
- Third-party parsers (`nom`, image crates) stay internal — their types must not appear in
  `argos_carve`'s public signatures.

## Full rule text

[reference.md](reference.md) — 11 rules, listed in the file header.
