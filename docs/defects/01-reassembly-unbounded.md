# 01 — Reassembly worked without limit, in silence, and could not be stopped

A scan of a 1 TB mechanical disk ran for over six hours. The progress bar stopped
moving partway, Cancel did nothing, the window stopped responding, memory reached
2.5 GB, and the process had to be killed from the system's process manager.

## What was measured

From the run's own manifest, and from the timestamps of the files it wrote:

| Measurement | Value |
| --- | --- |
| artifacts recorded | 345,862 |
| recovered by carving | 345,771 |
| recovered from filesystem metadata | 91 |
| **recovered by reassembly** | **0** |
| report stage, first to last file written | 02:08:28 → 02:28:50 — 20 minutes |
| unreadable regions | 98 |
| manifest size | 194 MB |

The report stage is the last one, and it took twenty minutes. Everything else in
the run happened before it, producing nothing from the stage that took the time.

## Cause

Four faults in the second half of `reassemble_broken`, all in
`crates/argos_engine/src/pipeline.rs`:

1. **The window was read once per fragmentation point.** `classify_windows`
   read `REASSEMBLY_WINDOW_BYTES` either side of every unresolved header — 32 MiB
   each — and headers cluster: the manifest shows 203 artifacts within eight
   megabytes of a single offset. The same bytes were therefore read once per
   neighbouring header. A few thousand unresolved headers is hundreds of
   gigabytes of seeking on a disk that seeks slowly.

2. **The memory grew with the reading, not with the result.** A
   `HashSet<u64>` held one entry per 4 KiB block examined, to avoid classifying a
   block twice. At 160 GiB of windows that is 40 million entries.

3. **The ceiling bounded the output, not the work.** `MAX_REASSEMBLY_BLOCKS`
   stopped the pass once 65,536 *image-capable* blocks had been collected. On a
   medium whose windows hold almost nothing image-like it is never reached, so
   the reading ran to the end of the last window whatever it cost.

4. **Neither half reported progress nor read the cancellation flag.** The only
   `is_cancelled` check was in the gap-search loop that precedes them. A stage
   that emits nothing is indistinguishable from a stage that has stopped, and a
   Cancel button wired to a flag nobody reads does nothing.

Two smaller faults compounded it. `Store::finish` serialized the whole manifest
into memory before writing it — 194 MB in one allocation on top of the records
themselves. And there was no log: the only diagnostic output was an `ARGOS_TRACE`
trace on stderr, which a desktop launch discards, so nothing survived the kill to
say where the run had been.

## Change

- `merged_windows` sorts the windows and merges the overlapping ones before
  anything is read. Every block is now visited once by construction, which is
  what removes the set of seen blocks rather than capping it.
- `MAX_CLASSIFY_BYTES` bounds the bytes the pass may read, and the run reports
  it through `Ceilings::reassembly_search` when it is reached.
- Both halves step the stage's `Counter` and check `Control::is_cancelled`. The
  graph walk is driven one header at a time from the engine, carrying the claimed
  extents forward, so the stage can say how far it has got and can be stopped
  between headers.
- The manifest is streamed with `serde_json::to_writer_pretty`.
- `crates/argos/src/scanlog.rs` writes `scan.log` into the session directory: a
  stage timeline, progress at most every five seconds, the final counts and every
  ceiling reached. It is flushed per line, so it survives a kill. It records the
  shape of the work and nothing about its content (`A-NO-CONTENT-IN-LOGS`).

## Proof

`pipeline::tests` in `crates/argos_engine/src/pipeline.rs`, reproducing the
measured shape: 203 headers 40 KiB apart merge into **one** sequential read
covering less than a fifth of what separate windows would have read; runs never
overlap, which is the property the removed set used to provide; and a window
never runs past the end of the medium.
