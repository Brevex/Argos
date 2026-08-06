---
name: rust-telemetry
description: Use BEFORE emitting any log, metric, span or trace — any `tracing`/`log` call, `event!`, `println!`/`eprintln!`/`dbg!` in library code, progress reporting, or logging that touches file paths, filenames or recovered content. Covers M-LOG-STRUCTURED, M-LOG-NOT-PRINT, M-LOG-OVERHEAD.
---

# Logging and telemetry

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## Telemetry, not println

`M-LOG-NOT-PRINT` — production paths emit through the telemetry framework. `println!` is reserved for
a CLI writing its actual user interface to stdout. `dbg!` never survives a commit.

For Argos this draws a clean line: the `argos` binary prints results and progress to the terminal;
every library crate below it emits events only.

## Structured events, never formatted strings

`M-LOG-STRUCTURED` — named properties plus a message template ([messagetemplates.org](https://messagetemplates.org/)),
so formatting is deferred to viewing time and costs no allocation at runtime:

```rust,ignore
// Bad: allocates on every call.
tracing::info!("scanned {} at offset {}", path.display(), offset);

// Good: named event + named properties + template.
event!(
    name: "device.scan.progress",
    Level::INFO,
    device.path = device.display(),
    scan.offset = offset,
    "scanned {{device.path}} up to {{scan.offset}}",
);
```

- [ ] Every event has a hierarchical name: `<component>.<operation>.<state>`
- [ ] Use `{{property}}` in the template (escapes Rust's format syntax, keeps the literal text)
- [ ] Follow [OpenTelemetry semantic conventions](https://opentelemetry.io/docs/specs/semconv/) for
      common attributes — `file.path`, `file.size`, `file.name`, `file.extension`, `error.type`

## Redact sensitive data

`M-LOG-STRUCTURED` also mandates redaction. Sensitive includes email addresses, **file paths that
reveal user identity**, filenames containing secrets, and file contents with PII. Log
`user.email.redacted`, never `user.email`. The `data_privacy` crate gives consistent redaction.

## Keep it off the hot path

`M-LOG-OVERHEAD` — assume telemetry is permanently enabled and under load. Inner loops stay free of
events entirely; if unavoidable, emit lightweight, allocation-free ones. Best pattern is to log once
per batch and let the reader reconstruct the detail offline.

```rust,ignore
// Bad: one allocation + one event per sector.
for s in sectors { log(format!("scanning sector {}", s.lba)); }

// Good: one event per batch.
event!(name: "device.scan.batch", Level::DEBUG, scan.range = %range, "scanning {{scan.range}}");
for s in sectors { … }
```

## Argos application

Argos processes **other people's private data by design** — this skill is a privacy control, not a
formatting preference.

- Recovered filenames, paths and content are personal data. Never log recovered content; log the
  offset, size and type. Log output paths redacted or relative to the case directory.
- Never log the raw bytes of a candidate artifact "for debugging", not even at `TRACE`.
- Event names to standardise on: `device.open.success`, `device.read.error`, `scan.batch.started`,
  `carve.candidate.found`, `carve.artifact.written`, `report.manifest.written`.
- Per-sector or per-candidate logging will dominate the scan cost — batch it (see
  `rust-performance`).
- Chain-of-custody records (device identifier, examiner, timestamps, hashes) are **audit output**,
  not telemetry: they belong in the report artifact with their own format and retention, not in the
  debug log stream.

## Full rule text

[reference.md](reference.md) — `M-LOG-STRUCTURED`, `M-LOG-NOT-PRINT`, `M-LOG-OVERHEAD`.
