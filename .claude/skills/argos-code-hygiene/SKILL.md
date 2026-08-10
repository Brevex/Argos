---
name: argos-code-hygiene
description: Use BEFORE removing a capability, adding a support directory (fuzz/, benches/, examples/, corpus/, fixtures, models), introducing a trait/generic/parameter/feature with one consumer, or writing a second implementation of something that already exists — and whenever asking whether a file, module, token or dependency is still used. Covers A-EARNS-ITS-PLACE, A-ONE-IMPLEMENTATION, A-NO-ORPHANS, A-SUPPORT-DECLARED, A-NO-SPECULATIVE-SURFACE.
---

# Code hygiene

Read [reference.md](reference.md) for the full rule text. This file is the decision layer.

Every other skill governs the moment code is **written**. This one governs the moment it stops
being **used** — the only axis the other 107 rules do not cover, and the one an agent-built
codebase fails on hardest, because each phase adds and almost nothing subtracts.

## The question that decides everything here

Nothing in this skill can be judged by looking at the thing itself. A directory of binary files is
mandatory infrastructure or dead weight depending entirely on evidence that lives elsewhere. So
before calling anything unnecessary, answer four questions with evidence:

1. **Reachability** — does anything reference it? `cargo tree`, `mod`/`use`/`pub use`, workspace
   members and `exclude`, frontend imports, `git ls-files`.
2. **Mandate** — does a rule in `.claude/skills/` require it to exist? This is the question that
   saves `fuzz/`: `A-FUZZ-EVERY-PARSER` requires it, so it stays whatever it looks like.
3. **Consumption** — does anything read its output? A CI lane, a test, a component, a reader.
4. **Orphanhood** — did it exist to serve something that is now gone?

Only what fails all four is unnecessary. Failing one is a question, not a verdict.

## Removing a capability

`A-NO-ORPHANS` — the removal is not finished when the code is gone. In the same change, remove what
existed only to serve it: fixtures, corpora, generators, model files, benchmarks, fuzz targets,
crate dependencies, `[workspace.dependencies]` entries, `.gitignore` rules, CI lanes and doc
sections. Nothing warns about an orphan, which is why this is written down rather than left to a
tool.

Know the blind spot: `cargo udeps` reads per-crate manifests only. An unused entry in the root
`[workspace.dependencies]` is invisible to it and CI stays green.

## Adding support material

`A-SUPPORT-DECLARED` — a directory holding anything other than shipped source states, in one line,
what runs it and what it guarantees. Without that line nobody can tell it from abandoned
scaffolding, and the cost of guessing wrong is deleting the proof that a parser survives hostile
input.

Corpus directories hold authored seeds only, produced by a committed generator and named for the
case they cover. A fuzzer's own output written back into the tree is not a seed: hash-named,
intentless and unbounded.

## Adding abstraction

`A-NO-SPECULATIVE-SURFACE` — a trait with one implementor, a generic instantiated once, a parameter
that always receives the same value, a variant nothing constructs, a feature nothing enables. Ask
what breaks if it is deleted and its single value inlined; "nothing, and it reads better" means it
was speculative. Two exemptions only, because two other rules mandate the surface:
`M-INIT-BUILDER` and `M-ERRORS-CANONICAL-STRUCTS`.

`A-ONE-IMPLEMENTATION` — before writing something, look for it. A second implementation of one
behaviour will diverge from the first, and in a recovery tool divergence means two different
answers about the same bytes. Reimplementing what a dependency provides is allowed and sometimes
right; the module doc must then say what the dependency does not give.

## Full rule text

[reference.md](reference.md) — `A-EARNS-ITS-PLACE`, `A-ONE-IMPLEMENTATION`, `A-NO-ORPHANS`,
`A-SUPPORT-DECLARED`, `A-NO-SPECULATIVE-SURFACE`.
