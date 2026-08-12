# 02 — A cached preview was reported as the photograph

A photograph recovered from the disk came back at 256×192 — far below anything a
camera of its era produced. Dozens more arrived as similar "miniatures". The
question the recovery raised, and could not answer, was whether the full image
was still there.

## What was measured

The artifact, from the manifest:

```
name          337018.jpg
stage         carve
confidence    contiguous-carve
source_offset 884,680,840,088
length        7,124 bytes
```

Its structure: JFIF, no EXIF, baseline, 256×192. A **whole, contiguous JPEG
stream** — not a fragment, and not an EXIF thumbnail embedded in a larger file.

Its neighbourhood, which is what settles it. Of the artifacts within ±4 MiB of
that offset, a sample of 60 decoded to:

| Dimensions | Count |
| --- | --- |
| 256×192 | 51 |
| 192×256 | 6 |
| 256×171 | 2 |
| 336×280 | 1 |

The near-duplicate the manifest linked it to is 2,528 bytes, at an offset 771 GB
away — another entry of the same kind.

## Conclusion

A run of hundreds of whole images that share one size to the pixel, packed
consecutively, is a **thumbnail cache**: the preview database a desktop or a
phone keeps of every picture it has displayed. It is written once and rarely
rewritten, which is why it outlives the photographs it describes — large files
are overwritten first.

So the artifact is a complete small file that was on the medium, not a piece of
the original. **The original cannot be reconstructed from it.** A 256×192 image
carries no information about the pixels the full frame held; no reassembly, and
no amount of further scanning of the same bytes, can put them back. Whether the
original survives elsewhere is a separate question, and in this run its bytes did
not appear.

Full-resolution photographs *were* recovered from the same disk — 4128×3096 and
1920×2560 among them — so the pipeline finds originals when they are there.

## Cause of the defect

Not the recovery, which was correct: the bytes were carved whole and hashed. The
defect is the report. A cache entry and a photograph were described identically,
because the manifest recorded a byte count and nothing about the picture, so
nothing distinguished a 7 KB preview from a 7 KB photograph without opening both.
That is what led to the wrong conclusion, and a report that leads to a wrong
conclusion is the failure this project exists to avoid (`A-CONFIDENCE-HONEST`).

## Change

- Every artifact record now carries `width` and `height`, taken from the decode
  the report stage already performs.
- `crates/argos_engine/src/cache_run.rs` recognises a run of eight or more
  consecutive artifacts of identical dimensions within four megabytes of each
  other, and records `same_size_neighbours` on each of them. A record that says
  it was found among 51 images of exactly its size is a record that cannot be
  mistaken for a photograph.
- Nothing is removed, reclassified or ranked by it. It is a count of neighbours,
  stated as a fact about the medium's layout.

## Proof

`cache_run::tests`: a run of 40 identical sizes is named with its length; fewer
than eight of one size is not a run; a camera frame between two runs is left
alone; a distant neighbour starts a new run; and an artifact that did not decode
belongs to none.
