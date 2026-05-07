# Aho–Corasick Multi-Pattern Matching

## Reference

Aho, A. V., & Corasick, M. J. (1975). *Efficient String Matching: An Aid to Bibliographic Search*. Communications of the ACM, 18(6), 333–340.

## Problem

Locate all occurrences of any pattern from a finite set within a streamed input, in time linear in the input plus the number of matches.

## Use in Argos

Argos uses Aho–Corasick on the SSD pipeline to scan streamed device blocks for image-format magic numbers (headers and footers). A single automaton handles all formats simultaneously.

## Pattern set

Header and footer signatures for supported formats. Examples:
- JPEG header: `FF D8 FF (E0|E1|E2|...)`
- JPEG footer: `FF D9`
- PNG header: `89 50 4E 47 0D 0A 1A 0A`
- PNG footer: trailing `IEND` chunk with terminating CRC

The full pattern set lives in `src/carve/ssd/patterns.rs`. Adding patterns requires an ADR.

## Automaton properties

- **Deterministic.** The constructed automaton is a DFA with goto, failure, and output functions.
- **Built once.** Construction is at startup or first scan; the automaton is reused across the entire device scan.
- **Streaming.** State persists across input blocks. Pattern matches that span block boundaries are detected correctly.
- **Linear time.** Matching cost is `O(|input| + |matches|)`.

## Implementation notes

- Use the `aho-corasick` crate for the automaton. The crate's `nfa::contiguous` or `dfa` build is selected based on the pattern set size and the latency vs memory trade-off, decided once.
- Input is fed as sector-aligned `&[u8]` slices. The matcher's state machine is owned by the scan pipeline; it is not recreated per block.
- Matches yield `(pattern_id, start_offset, end_offset)` tuples consumed by the carver to open and close tentative artifacts.

## Carver behavior on matches

- **Header match:** open a tentative `Candidate { id, start_offset }`.
- **Footer match for an open candidate:** close the candidate at `end_offset`, emit it for structural validation in `validate/`.
- **Footer match with no matching open candidate:** discard.
- **Format with unreliable footer (e.g., raw camera formats):** apply a max-length policy and structural validation rather than waiting for a footer.

## Invariants

1. The automaton is built from the full pattern set exactly once per session.
2. Scanning is single-pass; the device is read sequentially, not random-access.
3. State persistence across block boundaries is correct: a pattern that straddles a boundary is matched with the same offsets as if the input were contiguous.
4. No allocation per block during scanning. The matcher uses a fixed-size state.

## Property tests (required)

- For an input synthesized to contain known patterns at known offsets, the automaton reports exactly those matches.
- For random inputs, the false-positive rate matches the theoretical pattern-collision rate (within tolerance).
- Block-boundary correctness: splitting the same input at every possible boundary yields identical match sets.
- Fuzz target: the matcher does not panic on adversarial input.
