# ADR 0005 — No comments in source code

- **Status:** Accepted
- **Date:** 2026-05-07

## Context

Comments rot. They drift from the code they describe and become silent lies. They also let authors avoid the harder work of making code self-explanatory.

## Decision

Argos source files contain zero comments. No `//`, no `/* */`, no `///` doc-comments. The code itself is the documentation.

This applies to Rust, TypeScript, JSX/TSX, and shell scripts. Configuration files (TOML, JSON, YAML) are exempt because comments there cannot rot relative to active code paths.

## Rationale

- **Names enforced by the compiler.** A renamed function fails to compile until callers update; a stale comment compiles forever.
- **Forces refactoring.** If a block needs explanation, the structure is wrong: extract a function, introduce a newtype, or split a complex expression into named bindings.
- **Architecture lives in `docs/`.** Long-form rationale belongs in ADRs and algorithm specs, not interleaved with implementation.
- **Self-documenting code is testable.** A comment cannot be tested; a named function with a clear type signature can.

## Consequences

- Naming is hard and important. Identifier review is part of `rust-reviewer`'s checklist.
- `unsafe` blocks cannot rely on `// SAFETY:` comments. The invariant must be encoded in the function name and type signature wrapping the unsafe block.
- API documentation (e.g., for library consumers) is generated from `docs/`, not from doc-comments. If an external API surface emerges that requires generated docs, this ADR is revisited.

## Trade-offs

- We lose `cargo doc` output. Acceptable: Argos is an application, not a library.
- New contributors will find the code less inviting at first. Mitigated by `docs/architecture.md` and module-level READMEs in `docs/` (not in source).
