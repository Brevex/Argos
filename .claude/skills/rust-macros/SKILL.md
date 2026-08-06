---
name: rust-macros
description: Use BEFORE writing or reviewing any `macro_rules!`, procedural macro, derive macro, attribute macro, or a `_proc`/`_proc_impl` crate — including when considering a macro to reduce repetition. Covers M-MACRO-LAST-RESORT, M-EXAMPLE-OVER-PROC, M-MACRO-HELPERS, M-MACRO-MAIN-CRATE, M-MACROS-DONT-LIE, M-PROC-IMPL, M-PROC-IMPLIED-ITEMS.
---

# Macros

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

## Default answer: don't

`M-MACRO-LAST-RESORT` — *"macros are for when you run out of language"*. Rust gives you a lot of
language; use it first. Macros are magic, disproportionately slow down compilation for projects that
otherwise avoid them, and can break subtly at edition boundaries.

Counterintuitively: the more structurally complex the expansion, the worse a macro fits. The ideal
macro makes a reader think *"I know exactly what this generates, I just don't want to type it"*.

Before writing one, try: a generic function, a trait with a default method, a `const` table, a
builder, or plain repetition. Repetition is often the cheaper long-term cost.

## If you must

- [ ] `macro_rules!` over a proc macro whenever it can do the job (`M-EXAMPLE-OVER-PROC`) —
      `make_signature!(Jpeg)` beats `#[make_signature] struct Jpeg;`
- [ ] Never misrepresent what is written (`M-MACROS-DONT-LIE`): don't turn a struct into an enum,
      change a signature, add parameters, or make an item `async`
- [ ] Don't emit implied or hidden items (`M-PROC-IMPLIED-ITEMS`) — they collide with user types,
      with other expansions, and are invisible when re-exporting. The one accepted exception is the
      namespace overload (`fn foo` + `struct foo`), and only inside root crates
- [ ] Third-party items used by an expansion come from a `#[doc(hidden)] pub mod _private` re-export
      in the host crate, and the macro emits fully-qualified paths through it (`M-MACRO-HELPERS`)
- [ ] Assume the main crate name (`M-MACRO-MAIN-CRATE`); don't support being imported under an alias
- [ ] Proc macros are thin shims: `foo_proc` (with `proc-macro = true`) delegates to a regular
      `foo_proc_impl` library crate holding the logic and its tests (`M-PROC-IMPL`), with `insta`
      snapshot tests there and `trybuild` UI tests in the core crate

## Argos application

Low expected use. The one plausible candidate is a table of file-type signatures — and even there a
`const` array of a `Signature` struct is clearer, greppable, debuggable, and needs no expansion:

```rust,ignore
// Prefer this.
const SIGNATURES: &[Signature] = &[
    Signature::new("jpeg", &[0xFF, 0xD8, 0xFF], Some(&[0xFF, 0xD9])),
    Signature::new("png",  &[0x89, b'P', b'N', b'G'], Some(&[0x49, 0x45, 0x4E, 0x44])),
];
```

If a macro ever becomes justified, it must not hide which formats are supported: a forensic tool's
supported-format list has to be readable directly in the source.

## Full rule text

[reference.md](reference.md) — `M-EXAMPLE-OVER-PROC`, `M-MACRO-HELPERS`, `M-MACRO-LAST-RESORT`,
`M-MACRO-MAIN-CRATE`, `M-MACROS-DONT-LIE`, `M-PROC-IMPL`, `M-PROC-IMPLIED-ITEMS`.
