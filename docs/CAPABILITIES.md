# Capabilities

What Argos does, stated in the register of a user rather than of the code, and where each capability
is reachable from today.

This file is a **contract**. Every line of it is true of the current build, and a change that makes
any line false is a regression regardless of what else it improves. A cleanup, a refactor or a
boundary change is verified against this table before it is accepted — that is what this document
is for.

It says nothing about how anything is implemented and records no design history
(`M-NO-META-DESIGN-DOCUMENTATION`). Remaining work is in [OPEN-WORK.md](OPEN-WORK.md); the target
architecture is in [DEVELOPMENT-PLAN.md](DEVELOPMENT-PLAN.md).

## Reading the tables

**Reachable from** — `CLI` is `argos` as a subcommand or flag; `GUI` is the Tauri window; `wire` is
a JSON-RPC method of `argos serve` that no client calls yet. A capability marked `CLI` only is
implemented and tested; it simply has no graphical surface.

**Cost** is wall clock from one measured run: `/dev/sdc`, 1,000,204,886,016 bytes, 12 workers,
5 h 31 m total, state `finished`. The full measurement is [OPEN-WORK.md §1](OPEN-WORK.md). One disk
is one sample: the ordering between stages is dependable, the absolute figures are not.

---

## 1. Reaching the medium

| # | Capability | Reachable from | Cost |
| --- | --- | --- | --- |
| C01 | Open a block device read-only on Linux, Windows and macOS | CLI, GUI | — |
| C02 | Open a raw image file read-only | CLI, GUI | — |
| C03 | List the media this machine exposes, whole disks before partitions, without privileges | CLI, GUI | instant |
| C04 | Report each medium's class (HDD/SSD/NVMe), TRIM state, model and current mounts | CLI, GUI | instant |
| C05 | List shadow copies, where the platform has them | CLI, GUI | instant |
| C06 | Warn that a medium is mounted writable, or that the target is one partition rather than a disk | CLI, GUI | — |
| C07 | Copy a medium to a raw image in multiple passes, skipping failing regions and revisiting each sector by sector | CLI | hours, once |
| C08 | Zero-fill what stayed unreadable and list it, never presenting those zeroes as data that was read | CLI | — |

**Never**: a write path to the source medium. Devices are opened read-only at the lowest layer.

## 2. Finding images

| # | Capability | Reachable from | Cost |
| --- | --- | --- | --- |
| C09 | Carve the whole surface by signature | CLI, GUI | **2 h 01 m** (138 MB/s) |
| C10 | Recover from filesystem metadata: NTFS, ext2/3/4, FAT32, exFAT, APFS | CLI, GUI | **32 m** |
| C11 | Sweep for residue of filesystems that were on the medium before the current one | CLI, GUI | in C10 |
| C12 | Read partition tables | CLI, GUI | instant |
| C13 | Validate every candidate through its format's state machine (JPEG, PNG) | CLI, GUI | **41 m** (2,026,184 candidates) |
| C14 | Reassemble images the medium stored in pieces | CLI, GUI | **2 h 00 m** — a ceiling reached, not a completion |
| C15 | Report the decodable prefix of a photograph whose remainder was overwritten | CLI, GUI | in C13 |
| C16 | Read `$UsnJrnl:$J` for deletion names and moments | CLI, GUI | in C10 |
| C17 | Skip the filesystem stage (`--carve-only`) | CLI, GUI | saves 32 m |
| C18 | Skip carving (`--metadata-only`) | CLI, GUI | saves 2 h 01 m |
| C19 | Skip reassembly (`--no-reassemble`) | CLI, GUI | saves up to 2 h |
| C20 | Bound reassembly by wall clock (`--reassembly-budget`), or let it search every candidate | CLI | sets the ceiling |
| C21 | Scan one byte range of the medium rather than all of it (`--range`) | CLI | hours → minutes |
| C22 | Search a previous session's fragmentation points again without re-reading the medium (`reassemble --from`) | CLI | saves the whole sweep |
| C23 | Choose the worker count (`--jobs`) | CLI, GUI | — |

**Formats carved**: JPEG and PNG. Nothing else is claimed.

**Never**: report a signature hit that did not validate, or fabricate bytes for a region the medium
refused. Both are counted in the report instead.

## 3. Judging what was found

| # | Capability | Reachable from | Cost |
| --- | --- | --- | --- |
| C24 | Label each artifact photograph vs synthetic asset, by deterministic image statistics | CLI, GUI | **3 m** |
| C25 | Mark near-duplicates by perceptual hash. Both copies are kept | CLI, GUI | in C24 |
| C26 | Collapse artifacts with identical SHA-256 | CLI, GUI | in C13 |
| C27 | Order artifacts by the evidence they carry: `camera-named`, `dated`, `photograph-sized`, `unremarkable`, `cache-neighbour` | CLI, GUI | in C30 |
| C28 | Skip triage (`--no-triage`); nothing recovered changes, only the labels | CLI, GUI | saves 3 m |
| C29 | Set the smallest long side an image is written to disk for (`--min-long-side`, default 300 px) | CLI, GUI | changes what fills the output directory |

**Evidence tiers** recorded per artifact, weakest first: `partial-or-thumbnail`, `reassembled`,
`contiguous-carve`, `journal-residue`, `fs-metadata`.

**Never**: raise an artifact's tier, or let triage decide what is recovered. Triage labels; it does
not judge.

## 4. What comes out

| # | Capability | Reachable from | Cost |
| --- | --- | --- | --- |
| C30 | Write a manifest recording every artifact with its extents, SHA-256, dimensions, provenance, the coverage the run reached and the volumes it found | CLI, GUI | **12 m** |
| C31 | Record an artifact that was deliberately not written (below the size floor), with its extents and digest, so the manifest stays a complete account of the medium | CLI, GUI | in C30 |
| C32 | Render a small preview of every artifact that decodes | CLI, GUI | in C30 |
| C33 | Refuse an output directory that would contain the source | CLI, GUI | — |
| C34 | Print what a finished session recovered, read back from its manifest (`report`, `--all`) | CLI | instant |
| C35 | Read one page of a session's artifacts, ordered strongest evidence first, filtered by standing | CLI, GUI | instant |
| C36 | Copy artifacts out of a session, verifying each hash and refusing any whose bytes no longer reproduce it | CLI, wire | I/O bound |
| C37 | Select what to export by SHA-256 or an unambiguous prefix | CLI, wire | — |
| C38 | Select what to export by smallest long side | CLI | — |
| C39 | Select what to export by standing | CLI | — |
| C40 | Select what to export by recorded camera make or model | CLI | — |
| C41 | Select what to export by capture date range | CLI | — |

**Never**: present a count of artifacts recorded as a count of artifacts recovered. What was written
and what was only recorded are counted apart.

## 5. Running a scan

| # | Capability | Reachable from | Cost |
| --- | --- | --- | --- |
| C42 | Report progress per stage as it happens, pushed rather than polled, with the unit named so a candidate count is never read as a byte count | CLI, GUI | — |
| C43 | Stop a running scan, keeping everything recovered so far | CLI, GUI | — |
| C44 | Pause a running scan and resume it | CLI (`p`/`r`), GUI | — |
| C45 | Report a running total of regions the medium refused | CLI, GUI | — |
| C46 | Warn about anything the user should know before trusting the result | CLI, GUI | — |
| C47 | Run the engine as a JSON-RPC server on stdin/stdout (`serve`) | CLI | — |
| C48 | Acquire the privileges a raw device needs, and run the engine as a child that inherits them | GUI | — |
| C49 | Choose a theme, stored in the invoking user's home rather than the administrator's | GUI | — |

**Never**: log or transmit recovered content, recovered file names, or identifying paths.

---

## Totals

49 capabilities.

| | Count |
| --- | --- |
| Reachable from the command line | **47** — every one except C48 and C49, which are the window itself |
| Reachable from the window | **36** |
| Reachable from the command line but not the window | **13** — C07, C08, C20, C21, C22, C34, C36–C41, C47 |
| Expressible on the wire but called by no client | **2** — C36, C37 |

The command line reaches every capability the tool has, headless, before any interface exposes it
(`A-CLI-FIRST`). Where the window reaches fewer, the difference is a surface that has not been
built — never a second implementation.
