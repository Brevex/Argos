---
name: rust-errors-panics
description: Use BEFORE writing anything that can fail — any `Result`, `?`, custom error type, `impl From` for an error, `map_err`, `panic!`, `assert!`, `unreachable!`, `todo!`, `unwrap`, `expect`, or `catch_unwind`. Also when deciding whether a failure is an error or a bug. Covers M-ERRORS-CANONICAL-STRUCTS, M-FROM-ERROR, M-APP-ERROR, M-PANIC-IS-STOP, M-PANIC-ON-BUG, M-PANIC-MESSAGE, M-PANIC-CONTINUATION.
---

# Errors and panics

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## First decision: error or panic?

| Situation | Answer |
| --- | --- |
| Malformed input, missing device, bad sector, unsupported filesystem | `Result` |
| Broken invariant / contract violation *inside our own code* | `panic!` (`M-PANIC-ON-BUG`) |
| Something the caller could act on at runtime | `Result` |
| Something no runtime handler could fix | `panic!` |

Panics are **not** exceptions (`M-PANIC-IS-STOP`): they mean *stop this program now*. Never use them
to signal upstream, never assume they get caught — a downstream app may build with `panic = "abort"`.
Valid reasons: programming errors, `const` contexts, user-requested `unwrap()`, poisoned locks.

`catch_unwind` is last resort (`M-PANIC-CONTINUATION`) and must be followed by a controlled restart;
in a long scan it may finish in-flight work but never resume indefinitely.

Every intentional panic carries a message with the actual values (`M-PANIC-MESSAGE`):

```rust,ignore
assert!(buf.len() >= SECTOR_SIZE, "buffer too small for sector: got {} bytes, need {SECTOR_SIZE}", buf.len());
```

Better than panicking: make it impossible. Push the check into a newtype constructor
(see `rust-type-design`, `M-STRONG-TYPES-GUARD`) so the bad state cannot be built.

## Library errors are canonical structs

For every crate that is *not* the top-level binary (`M-ERRORS-CANONICAL-STRUCTS`):

- [ ] A **struct**, not a public enum, one per situation (`DeviceError`, `CarveError`), reused
      across related functions — not one type per function, not one god-enum for the workspace
- [ ] Holds a `Backtrace` captured at construction (`Backtrace::capture()` is nearly free unless
      `RUST_BACKTRACE` is set)
- [ ] If it aggregates causes, keep a **private** `ErrorKind` enum and expose `is_io()`,
      `is_corrupt()` … — never make the kind public, or every internal failure mode becomes API
- [ ] Contextual accessors where they help the caller: `fn offset(&self) -> Lba`, `fn device(&self) -> &Path`
- [ ] `impl Debug`, `impl Display` (summary + backtrace + upstream cause), `impl std::error::Error`
- [ ] Conversions via `impl From<..>` so `?` just works (`M-FROM-ERROR`) — `map_err` only for
      foreign types or when adding context
- [ ] A private `bail!()` helper if the crate emits many errors

## The application may use anyhow

`M-APP-ERROR`: the Argos CLI binary (and crates used only by it) may use `anyhow`/`eyre`/
`ohno::AppError` instead of hand-rolled types. Pick **one** and use it everywhere in the app layer.
Libraries never do this.

The boundary is explicit:

```
crates/argos/          → anyhow::Result       (app layer, one chosen crate)
crates/argos_core/     → CoreError            (canonical struct)
crates/argos_device/   → DeviceError          (canonical struct)
crates/argos_carve/    → CarveError           (canonical struct)
```

## Argos application

- A **bad sector, truncated image, or unrecognised filesystem is normal**, not exceptional: it is
  `Result`, and the scan continues. Recovery tools that abort on the first damaged region are
  useless — corruption is the expected operating condition.
- Partial success is a first-class outcome, not an error: prefer returning recovered artifacts plus
  a defect report over failing the whole scan.
- Errors carry forensic context: device path, LBA/byte offset, and what was being parsed. That
  context is what makes a report auditable.
- `unwrap()`/`expect()` are allowed only where the invariant is locally provable, and `expect` must
  say *why it cannot happen*, not what happened.
- Do not log-and-return the same error (see `rust-telemetry`): emit the event once, at the layer
  that decides what to do with it.

## Full rule text

[reference.md](reference.md) — `M-ERRORS-CANONICAL-STRUCTS`, `M-FROM-ERROR`, `M-APP-ERROR`,
`M-PANIC-IS-STOP`, `M-PANIC-ON-BUG`, `M-PANIC-MESSAGE`, `M-PANIC-CONTINUATION`.
