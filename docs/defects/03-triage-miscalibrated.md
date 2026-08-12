# 03 — The triage rules are inverted against real media, and decided what was written

The output directory held 154,023 images and 3.25 GiB. Almost all of it was
icons, avatars and cache entries of very low resolution; the photographs the scan
existed to find were a small minority buried in them.

## What was measured

Of the 154,023 artifacts written:

| Triage label | Count | Share |
| --- | --- | --- |
| `ambiguous` / `inconclusive` | 121,555 | 79% |
| `photograph` / `sensor-texture` | 32,404 | 21% |

A sample of 1,499 decoded:

| Long side | Share |
| --- | --- |
| ≤ 64 px | 6.5% |
| 65–128 | 3.1% |
| 129–256 | 7.3% |
| 257–512 | 74.9% |
| 513–1024 | 6.9% |
| **> 1024** | **1.2%** |

57% were at exactly ~258×258 — one size, repeated, which is the signature of a
cache (see [02](02-thumbnail-provenance.md)).

A sample of 3,000, crossing dimensions with the label, is what makes the cause
unambiguous:

| Long side | `photograph` | `ambiguous` |
| --- | --- | --- |
| ≤ 300 px | 611 | 2,025 |
| 301–640 | 38 | 278 |
| **> 640** | **10** | **37** |

Of 47 artifacts above 640 pixels, **only 10 were called photographs** — the 37
others include two of 4128×3096, which are camera originals. Meanwhile 611
thumbnails of 300 pixels or less *were* called photographs.

Reproducing the feature pass of `crates/argos_classify/src/rules.rs` on
individual files gives the mechanism:

| Image | Textured fraction |
| --- | --- |
| 1920×2560 camera frame | 0.442 |
| 768×576 camera frame | 0.411 |
| 256×192 cache entry | 0.292 |
| **`PHOTOGRAPH_MIN_TEXTURE`** | **0.60** |

## Cause

Two, and the second is what turned the first into lost evidence.

1. **The thresholds describe generated images, not photographs.** The constant's
   own documentation states that "the least textured photograph of any slice
   measures 0.651". That number comes from the corpus in
   `crates/argos_classify/src/fixture.rs`, which *draws* its photographs. Drawn
   noise is not sensor noise, and the gap the generated numbers describe does not
   exist in real media: every real photograph measured here falls below the
   threshold, so the rule calls camera originals `Ambiguous`.

2. **That label decided what was written.** The asset screen in the report stage
   omitted anything labelled a synthetic asset and stored everything else,
   including everything it could not judge — which was 79% of the output.

The consequence, had the obvious fix been applied: writing only what the rules
call a photograph would have discarded 37 of the 47 large images in the sample —
about four in five of the full-resolution photographs — while still writing 611
thumbnails.

## Change

- **The label no longer decides what is written.** The asset screen is removed
  from the report stage. Triage still labels and orders; it settles nothing about
  the output directory (`A-TRIAGE-NOT-VERDICT`).
- **The decision is the decoded resolution.** `DEFAULT_MIN_LONG_SIDE` is 300
  pixels: above the caches measured here — 258×258 and 256×192 — and below
  640×480, the smallest resolution a camera of that era produced, so no
  photograph on this medium falls under it. `argos scan --min-long-side <PIXELS>`
  overrides it; `0` writes everything.
- **Nothing is discarded.** An artifact under the floor is recorded with its
  extents, digest and dimensions, `written: false` and
  `omitted_because: "below-size-floor"`. The extents locate its bytes on the
  medium exactly, so `argos scan --min-long-side 0` produces them.
- `PHOTOGRAPH_MIN_TEXTURE` now carries the measurements above and states that it
  is not calibrated against real media.

## Outstanding

Two things.

Recalibrating the rules against a labelled corpus of real recoveries, and only
then letting the label carry weight again. Until that is done the threshold
stands as documented-and-known-wrong rather than documented-and-trusted
(`A-EVAL-GATED`).

And a way to produce an omitted artifact without rescanning the whole medium.
`argos export` reads the session directory and can only copy files that are
there, so today the answer is a rerun with a lower floor. The manifest already
holds everything such a command would need — the extents and the digest to
verify against — so this is a missing capability, not missing information.

## Proof

`an_image_below_the_size_floor_is_recorded_and_not_written` in
`crates/argos/tests/cli.rs`: a 96×72 and a 640×480 image on one fixture disk; the
run with the floor accounts for both, writes only the 640×480, and the record for
the other carries its dimensions, its hash, its extents and the reason.
