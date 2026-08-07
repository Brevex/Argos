---
name: argos-ondisk-parsing
description: Use BEFORE writing or reviewing any parser of on-disk structures — partition tables, filesystem metadata (MFT records, superblocks, inodes, dirents, APFS objects), or file-format structures (JPEG segments, PNG chunks) — and before adding a fuzz target or a parser fixture. Covers A-PARSER-SAFE, A-BOUNDED-ALLOC, A-FUZZ-EVERY-PARSER, A-FIXTURE-BUILDER.
---

# On-disk structure parsing

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.
Parsers are where the untrusted medium meets our code; `argos-evidence-handling`
(`A-UNTRUSTED-ONDISK`) governs the values, this skill governs how the parsers themselves
are built.

## How a parser is written here

- `A-PARSER-SAFE` — parsers contain zero `unsafe`. Fixed layouts are decoded with explicit
  checked reads (`u32::from_le_bytes` over `get(..)` slices), never by casting bytes to a
  `#[repr(C)]` struct. A helper "cursor over `&[u8]` returning `Result`" pattern per crate keeps
  this terse; write offsets once, as named constants with the on-disk meaning documented
  (`M-DOCUMENTED-MAGIC`).
- `A-BOUNDED-ALLOC` — a parser's memory use is bounded by named constants derived from the format
  specification, independent of any value read from the medium.
- Parse results distinguish three outcomes: valid object, corrupt object (recoverable `Err` —
  the scan continues), and bug (panic per `M-PANIC-ON-BUG`, with values). Corruption is the
  expected condition; reaching for a panic in a parser is almost always wrong.

## Every parser is fuzzed and fixtured

- `A-FUZZ-EVERY-PARSER` — each parser entry point ships a `cargo-fuzz` target in the crate's
  `fuzz/` directory the day it lands, not later. The target feeds raw bytes straight into the
  entry point; any panic, OOM or timeout is a bug by definition.
- `A-FIXTURE-BUILDER` — each parser ships a builder that constructs valid *and* deliberately
  corrupt fixtures (`M-TEST-UTIL`), and its test suite includes corrupt-input cases as
  first-class tests: truncated at every field boundary, overflowing lengths, cyclic references,
  zeroed regions.

## Full rule text

[reference.md](reference.md) — `A-PARSER-SAFE`, `A-BOUNDED-ALLOC`, `A-FUZZ-EVERY-PARSER`,
`A-FIXTURE-BUILDER`.
