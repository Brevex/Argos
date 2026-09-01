# On-disk structure parsing — full rule text

Argos-specific rules (`A-*` namespace). Index: [../../guidelines-index.md](../../guidelines-index.md).

## Parsers contain no unsafe code (A-PARSER-SAFE) { #A-PARSER-SAFE }

<why>parsers process hostile bytes; combining that with `unsafe` multiplies risk for a decoding
speedup that has never been the bottleneck.</why>

On-disk structures are decoded with explicit, checked, endian-aware reads:

```rust,ignore
let bytes_per_sector = buf
    .get(BPB_BYTES_PER_SECTOR..BPB_BYTES_PER_SECTOR + 2)
    .map(|b| u16::from_le_bytes([b[0], b[1]]))
    .ok_or_else(|| NtfsParseError::truncated("boot sector", buf.len()))?;
```

- Never cast or transmute a byte buffer to a `#[repr(C)]` layout struct — alignment and validity
  of hostile bytes cannot be guaranteed, and per project policy `unsafe` exists only in
  `argos_device` (`M-UNSAFE`, CLAUDE.md).
- Field offsets are named constants carrying the on-disk meaning and the specification source in
  their doc comment (`M-DOCUMENTED-MAGIC`).
- Each parsing crate may define one small checked-cursor helper over `&[u8]` to keep call sites
  terse; the helper returns `Result` on every read and never panics.

## Parser memory use is bounded by the format, not the medium (A-BOUNDED-ALLOC) { #A-BOUNDED-ALLOC }

<why>a single crafted 64-bit length field must not be able to OOM the scan of a 4 TB disk.</why>

Every allocation and every accumulation inside a parser is capped by a named constant justified by
the format specification (e.g. an MFT record is ≤ 4 KiB by definition; a PNG chunk length is
≤ 2³¹−1 by spec, but Argos caps decompressed IDAT accumulation far lower per pass). Values that
exceed the cap fail that object's parse; they never grow the cap. Streaming beats buffering
wherever the format allows it.

## Every parser has a fuzz target from day one (A-FUZZ-EVERY-PARSER) { #A-FUZZ-EVERY-PARSER }

<why>checked reads make hostile input *return errors*; only fuzzing demonstrates it.</why>

Each public parser entry point gets a `cargo-fuzz` target feeding it arbitrary bytes, committed in
the same change that introduces the parser. The property under fuzz is total: for any input, the
parser returns `Ok` or `Err` in bounded time and bounded memory — no panic, no OOM, no hang.
Fuzz targets live in the owning crate's `fuzz/` directory; CI runs each target for a fixed budget,
and the seed corpus under `fuzz/corpus/` is **generated, not committed**: a committed generator
writes one named seed per case, and the lane that fuzzes runs it first. A reproducer found by hand
is added to the generator, where it acquires the name and the intent a seed is required to carry.

## Every parser ships a fixture builder and corrupt-input tests (A-FIXTURE-BUILDER) { #A-FIXTURE-BUILDER }

<why>real corrupt media is what Argos exists for; a parser tested only on well-formed input is
untested.</why>

Alongside each parser lives a builder (behind `test-util`, per `M-TEST-UTIL`) able to produce:

- a minimal valid instance of the structure;
- systematically corrupt variants: truncation at each field boundary, each length/count field
  overflowed, cross-references cycled, and the structure zero-filled;
- composed fixtures (a structure embedded at an arbitrary offset in a larger image).

The parser's test suite exercises all of these as first-class named tests asserting the specific
`Err` classification, not just "doesn't crash" (`M-TAUTOLOGICAL-TESTS` applies: assert the
property, not the parser's branch structure).
