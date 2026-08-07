---
name: argos-recovery-algorithms
description: Use BEFORE implementing or modifying any recovery technique — residue sweep, NTFS/ext4/FAT/APFS deleted-file recovery, JPEG/PNG validation state machines, block classification, bifragment gap carving, graph reassembly — so the implementation follows the written spec instead of improvising. Covers A-ALGORITHM-FROM-SPEC.
---

# Recovery algorithms

Read [reference.md](reference.md) for the specs. This file is the decision layer.

## One rule

`A-ALGORITHM-FROM-SPEC` — every recovery technique in Argos is implemented from the written
specification in [reference.md](reference.md), which fixes terminology, invariants and the
literature it derives from. If the implementation needs to deviate (a better bound, a simplification,
a case the spec missed), **update the spec in the same change**, then implement. An algorithm that
exists only in code drifts, and two agents will implement it two ways.

## What lives in the spec file

| Spec | Used by stage |
| --- | --- |
| Acquisition (multi-pass imaging) | A |
| Residue sweep (orphaned volume anchors after re-formats) | B |
| NTFS deleted/orphan MFT recovery, `$I30` slack, `$UsnJrnl` | C |
| ext4 backup superblocks + jbd2 journal inode mining | C |
| FAT32/exFAT deleted dirents + contiguity assumption | C |
| APFS checkpoint history, omap, snapshots | C |
| JPEG validation state machine (+ EXIF thumbnails) | D |
| PNG validation state machine | D |
| Block classification (entropy/histogram/JPEG-stream/zlib) | E |
| Bifragment gap carving | E |
| Graph reassembly (PUP) + restart-marker orphan decoding | E |
| Confidence tiers and finding merge/dedup | F |

When implementing, also invoke `argos-ondisk-parsing` (the parsers these specs need) and
`argos-evidence-handling` (the values they consume).

## Full text

[reference.md](reference.md) — `A-ALGORITHM-FROM-SPEC` plus the twelve specs above.
