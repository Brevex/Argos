# btrfs support

What Argos recovers from a btrfs volume, and what was measured against real media. The algorithm
itself is specified in `argos-recovery-algorithms` under **Spec: btrfs deleted-file recovery**
(`A-ALGORITHM-FROM-SPEC`); this file records the evidence behind it and the limits it stops at.

Implementation: [btrfs.rs](../crates/argos_fs/src/btrfs.rs), the `volume_at` arm in
[residue.rs](../crates/argos_fs/src/residue.rs), and the `FsKind::Btrfs` arm of `recover_volume` in
[filesystem.rs](../crates/argos_engine/src/pipeline/filesystem.rs).

---

## 1. What it does

**Locates the volume from any surviving superblock copy.** btrfs keeps four, at fixed physical
offsets of 64 KiB, 64 MiB, 256 GiB and 1 EiB, and each states in `bytenr` which one it is. So an
anchor found anywhere on the surface fixes the volume's start at `at - bytenr` rather than assuming
it — the case a re-format creates, where the primary is gone and only a mirror survives.

**Resolves logical addresses through the chunk map.** Every tree pointer and every extent address in
btrfs is logical. The map is bootstrapped from the superblock's `sys_chunk_array` and completed by
walking the chunk tree. A parser that skipped this would read the wrong bytes and report them as
evidence.

**Recovers deleted files by two paths**, both a diff against what the volume's current trees still
describe:

| Path | Source | Tier |
| --- | --- | --- |
| backup roots | the four historical root sets in `super_roots` — a complete, checksummed tree the filesystem itself retains | `FsMetadata` |
| stale tree blocks | checksum-valid leaves on the surface that no live root reaches, which copy-on-write leaves behind when a file is deleted | `JournalResidue` |

The tiers differ because the evidence does: a backup root is metadata the filesystem kept, while an
orphaned leaf names an extent that may since have been reallocated. Either is demoted to
`PartialOrThumbnail` by the engine's double confirmation unless a signature sits at the first extent
*and* the assembled extents pass the format state machine.

**Costs nothing on volumes it does not find.** The whole-surface sweep tests eight bytes at a fixed
offset before anything else; the checksum runs only after that matches.

---

## 2. What was measured

Against a 512 MB image built with `mkfs.btrfs --rootdir` on a plain file — the only way this project
makes filesystem fixtures (`A-READ-ONLY`; the `deny-device-writes` hook refuses `mkfs` against a
device node, and DEVELOPMENT-PLAN P3 says "loopback files only").

| | |
| --- | --- |
| geometry read from the superblock | `sectorsize` 4096, `nodesize` 16384, generation 9 |
| `crc32c` over bytes 32.. | matches the stored seal exactly |
| superblock copies present | 2 (64 KiB and 64 MiB), each naming its own offset in `bytenr` |
| historical root sets in `super_roots` | 4, generations 6, 7, 8, 9 |
| chunk map after the chunk-tree walk | 4 entries, including a `SYSTEM｜DUP` chunk with 2 stripes |
| files read back through the chunk map | 2 of 2, **byte-identical** (SHA-256 matching the sources), each stored in 2 extents |
| volume located by the residue sweep | offset 0, 536870912 bytes, `allocation_bytes` 4096 |
| the same, with the first megabyte zeroed | **unchanged** — the 64 MiB mirror fixed the start |
| full `argos scan` of an image holding two real JPEGs | both recovered byte-identical, 2048×1536 and 1600×1200 |

Selectivity of the stale-block gate, measured by sweeping the whole surface at `nodesize`:

| | |
| --- | --- |
| blocks probed | 32,768 |
| carried the volume's `fsid` | 55 |
| passed `crc32c` | 53 — the 2 failures are the superblock mirrors, which are 4096 bytes rather than `nodesize` |
| reachable from the current tree root | 10 |
| **stale copy-on-write residue** | **43**, spanning generations 1–9 |
| the `fsid` gate over 2,000 random blocks | **0 hits** |

Forty-three stale leaves on a filesystem that has only ever been written by `mkfs`. That is the path
that pays on a used disk, and it is why the sweep exists alongside the backup-root ring, which
reaches only four generations.

This selectivity is also what keeps btrfs from repeating the ext4 defect in
[OPEN-WORK §3.6](OPEN-WORK.md), where an anchor is accepted on a superblock parse alone and one real
disk produced 15,157 phantom volumes. A btrfs anchor must match the magic, name one of the four
mirror offsets in `bytenr`, and pass its checksum.

---

## 3. What it deliberately does not do

Each of these yields **nothing** rather than something unverified. Removing any limitation updates
this section and the spec in the same change.

- **Striped profiles.** `RAID0`, `RAID10`, `RAID5` and `RAID6` put part of every stripe on another
  device, which one scan cannot see. Such a chunk is not mapped at all.
- **Checksum types other than `crc32c`.** `xxhash64`, `sha256` and `blake2b` volumes fail their
  superblock parse.
- **Compressed extents.** The bytes on disk are then not the file's bytes, so the file is not
  reported. This costs little for what Argos looks for: btrfs declines to compress
  already-compressed data, so JPEG and PNG land uncompressed even under `compress=zstd` — both
  planted photographs above have `compression = 0`.
- **Holes and unresolvable extents.** A file missing any piece is not reported, because a gap would
  splice unrelated bytes into the middle of a photograph.
- **The extent tree, the free-space cache, seed devices and multi-device pools.**

---

## 4. What is not yet proven against real media

**The deleted-file diff itself.** Producing genuine deletion residue on btrfs requires mounting the
filesystem, which requires root, and `mkfs.btrfs --rootdir` writes the filesystem tree exactly once —
so an unmounted image has no copy-on-write history of a removal to recover. Both paths are proven on
fixtures, and every structure they read is proven against real btrfs (§2); what a mounted volume
would add is the one step in between.

This has the same standing as the `$UsnJrnl` item in [OPEN-WORK §3.9a](OPEN-WORK.md): the reader and
its fixture agree, and a targeted read of one live volume is what would close it.

**How to close it**, on a machine where mounting is available:

```bash
truncate -s 2G disk.img && mkfs.btrfs -q -f disk.img
sudo mount -o loop disk.img /mnt/t && sudo cp photos/*.jpg /mnt/t/
sudo sync && sudo rm /mnt/t/*.jpg && sudo sync && sudo umount /mnt/t
argos scan disk.img --metadata-only --out ./out
```

The manifest must report the photographs at stage `filesystem`, and
`btrfs inspect-internal dump-tree disk.img` gives the ground truth to check every extent against.
