# ADR 0003 — PUP + SHT for HDD recovery

- **Status:** Accepted
- **Date:** 2026-05-07

## Context

On rotational media, deleted files commonly survive in unallocated sectors but may be fragmented. Naive header/footer carving fails on fragmented files. The literature offers two complementary techniques: Parallel Unique Path (PUP) for file reassembly and Sequential Hypothesis Testing (SHT / Wald SPRT) for fragmentation-point detection.

## Decision

Argos's HDD pipeline is:
1. Header detection seeds candidate files.
2. PUP extends candidates one block at a time, choosing the best continuation per step under a uniqueness constraint.
3. SHT decides when a candidate has reached a fragmentation point.
4. Format validators (Huffman for JPEG, CRC32 for PNG) supply the per-block scores consumed by PUP and SHT.

## Rationale

- **PUP** (Pal & Memon 2009) is well-studied, has tractable complexity, and naturally enforces block uniqueness across reassembled files.
- **SHT** (Wald 1945; Pal, Sencar, Memon 2008) provides bounded-error fragmentation detection with explicit α/β tradeoffs, suitable for forensic contexts where false-fragmentation rate must be configurable.
- **Format-specific validators** ground the statistical decisions in domain knowledge; without them, PUP/SHT would float on noise.

## Alternatives considered

- **Bifragment Gap Carving (Garfinkel):** simpler but assumes exactly two fragments. PUP generalizes.
- **Graph-theoretic reassembly (e.g., Memon & Pal Hamiltonian path):** higher quality theoretically but NP-hard; not viable for large disks.
- **Pure header/footer carving:** insufficient on fragmented HDDs.

## Consequences

- Implementation must follow `docs/algorithms/pup.md` and `docs/algorithms/sht-wald.md` exactly.
- Per-format validators are part of the contract; new formats require new validators before they can participate.
- Default SPRT thresholds are α=0.01, β=0.01; surfaced in configuration.
