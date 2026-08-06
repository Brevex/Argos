---
name: rust-docs
description: Use BEFORE writing or reviewing any `///` item doc, `//!` module doc, crate README, doc example, or hardcoded constant/magic value. Also when tempted to document how a change was designed. Covers M-CANONICAL-DOCS, M-MODULE-DOCS, M-FIRST-DOC-SENTENCE, M-DOC-INLINE, M-DOCUMENTED-MAGIC, M-NO-META-DESIGN-DOCUMENTATION.
---

# Documentation

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## Item docs have canonical sections

`M-CANONICAL-DOCS` — the summary sentence is always required; the rest when applicable:

```rust,ignore
/// Summary sentence under 15 words.
///
/// Extended free-form documentation.
///
/// # Examples
/// # Errors     — when the fn returns `Result`, list the known error conditions
/// # Panics     — when the fn may panic, list when
/// # Safety     — when the fn is `unsafe`, list every condition the caller must uphold
/// # Abort      — when the fn may abort the process
```

Never write a `# Parameters` table — Rust explains parameters in prose:
`/// Copies a file from `src` to `dst`.`

## First sentence: one line, ~15 words

`M-FIRST-DOC-SENTENCE` — it becomes the module-summary line, rendered at fixed width. Long first
sentences produce widows and unskimmable indexes. Clippy's `too_long_first_doc_paragraph` enforces it.

## Every public module gets `//!`

`M-MODULE-DOCS` — comprehensive, covering what the module contains, when to use it (and when not),
examples, subsystem specifications, observable side effects and their guarantees, and relevant
implementation details such as the system APIs used. `std::fmt`, `std::pin` and `std::option` are the
reference standard.

## Re-exports

`M-DOC-INLINE` — annotate `pub use` of **own** crate items with `#[doc(inline)]` so they render
alongside their siblings. Third-party and `std` re-exports are never inlined: readers must see that
they are external. (Globs are still forbidden — see `rust-api-surface`.)

## Magic values

`M-DOCUMENTED-MAGIC` — every hardcoded value carries a comment explaining *why* that value, what
breaks if it changes, and which external system it depends on. Prefer a named `const` with doc
comment over an inline literal.

## Never document the design journey

`M-NO-META-DESIGN-DOCUMENTATION` — user-facing docs describe the **end state**, not how we got
there. Specifically forbidden, and a frequent agent failure mode:

- "why we picked X over Y" essays in module docs
- design journals and change narratives
- **self-report tables claiming which guidelines were followed** (`| M-SHORT-NAMES | ✅ | … |`)

Never add such a table to code, docs, or a commit message. Enduring architectural goals belong in
the README's design-principles section, not in item docs.

## Argos application

- **Every file signature is a documented named constant.** This is the project's densest cluster of
  magic values and the place where a wrong byte silently ruins recovery:

  ```rust,ignore
  /// JPEG start-of-image marker.
  ///
  /// Every JFIF/EXIF file begins with these bytes, followed by an APPn marker.
  /// Changing this breaks detection of all JPEG variants.
  /// Source: ITU-T T.81 (JPEG), Annex B.1.
  const JPEG_SOI: [u8; 3] = [0xFF, 0xD8, 0xFF];
  ```

  The same applies to sector sizes, cluster limits, `O_DIRECT` alignment, MFT record size (1024),
  ext4 superblock offset (1024) and every scan-window constant.
- Module docs for `argos_device` must state the observable side effects: which devices are opened,
  in which mode, and the guarantee that nothing is ever written — that statement is part of the
  forensic contract.
- Parser modules document the on-disk layout they implement and cite the specification.
- Document what recovery **cannot** do (fragmented files without metadata, TRIMmed SSD blocks,
  encrypted volumes). Overstated capability is a correctness problem in a forensic tool.
- Doc examples must run without a real disk — build them on in-memory images (`rust-testing`).

## Full rule text

[reference.md](reference.md) — `M-CANONICAL-DOCS`, `M-MODULE-DOCS`, `M-FIRST-DOC-SENTENCE`,
`M-DOC-INLINE`, `M-DOCUMENTED-MAGIC`, `M-NO-META-DESIGN-DOCUMENTATION`.
