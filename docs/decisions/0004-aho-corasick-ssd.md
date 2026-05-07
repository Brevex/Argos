# ADR 0004 — Aho–Corasick header/footer carving for SSDs

- **Status:** Accepted
- **Date:** 2026-05-07

## Context

On SSDs and NVMe devices, TRIM and wear-leveling/garbage collection mathematically eliminate the persistence of evicted blocks within the addressable region. Deep reassembly (PUP-style) operates on data that, on these devices, is overwhelmingly unrecoverable to begin with.

## Decision

The SSD pipeline uses Aho–Corasick multi-pattern matching for header/footer carving over surviving regions. No fragmentation reassembly is attempted on the addressable region. PUP and SHT are not run on SSD inputs.

## Rationale

- **Theoretical ceiling.** Carving over surviving regions is the maximum recovery viable from software alone on a device with active TRIM. Pursuing more would be misleading at best.
- **Aho–Corasick** (Aho & Corasick 1975) matches an arbitrary set of patterns in a single pass with a deterministic automaton. Optimal for our multi-format carving.
- **Streaming.** The automaton state persists across sector-aligned blocks, allowing efficient single-pass scans of large devices.

## Alternatives considered

- **Boyer–Moore per format:** O(format-count × bytes) versus Aho–Corasick's O(bytes + matches). Worse asymptotics.
- **Regex engines (`regex`, `pcre`):** more general than needed; slower constant factors; harder to reason about adversarial inputs.
- **Hardware-assisted recovery (vendor-specific NAND access):** out of scope; would require dropping the software-only premise.

## Consequences

- The pattern set in `carve::ssd::patterns` is the source of supported formats on SSD. Additions are ADRs.
- Users must understand that SSD recovery is best-effort over surviving regions. Documentation must convey this.
- The dispatcher in `carve/` correctly classifies devices; misclassification (treating an SSD as HDD) wastes time but does not produce wrong results.
