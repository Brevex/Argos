# Code hygiene — full rule text

Argos-specific rules (`A-*` namespace). Index: [../../guidelines-index.md](../../guidelines-index.md).

Every other rule in this book governs the moment something is **written**. These five govern the
moment it stops being **used**. A file can satisfy all 89 `M-*` rules and all 18 other `A-*` rules
and still be worthless, because correctness at authoring time says nothing about whether the thing
is still wanted a phase later. That is the gap these close.

## Everything is reachable from something (A-EARNS-ITS-PLACE) { #A-EARNS-ITS-PLACE }

<why>a codebase grown by agents accumulates things that compile, pass review and are read by
nothing; the compiler cannot see most of them, so the check has to be written down.</why>

Every file, module, directory, public item, struct field, function parameter, cargo feature, design
token and configuration key is reachable from at least one of four things:

1. a binary entry point,
2. a test,
3. a CI lane,
4. a rule in `.claude/skills/` that requires it to exist.

Something reachable from none of the four is removed, or the reason it stays is written where it
lives. "It might be needed later" is not that reason — see `A-NO-SPECULATIVE-SURFACE`.

The fourth item is not a loophole, it is the load-bearing one. `crates/argos_carve/fuzz/` is
reachable from no binary and no test, and it stays, because `A-FUZZ-EVERY-PARSER` requires it and
the `fuzz` lane runs it. Reachability is judged by evidence, never by whether a directory looks
like scaffolding.

Two failure shapes this project has actually produced, both of which compiled and passed every
reviewer:

- a design token declared by all three themes that no component ever read;
- a struct field in a wire contract that nothing rendered and nothing consumed.

Neither is visible to `rustc`, `clippy`, `cargo udeps` or `svelte-check`. Only the question "who
reads this?" finds them.

## One behaviour, one implementation (A-ONE-IMPLEMENTATION) { #A-ONE-IMPLEMENTATION }

<why>two implementations of one behaviour do not stay equal; they diverge silently, and in a
recovery tool the divergence is two different answers about the same bytes.</why>

A behaviour is implemented once and called from everywhere it is needed. When a second
implementation appears, name the one that stays and make the other call it.

`A-SHELL-NO-DOMAIN` already forbids this across the UI/engine boundary. This rule is the same
principle everywhere else: two carvers for one format, two checked-cursor helpers, two hash
wrappers, two functions that both decide what a "small" image is.

What this rule does **not** forbid:

- per-crate `fixture` modules behind `test-util`, each building the fixtures of its own domain —
  those are different fixtures, not one fixture written twice;
- a per-OS module under a HAL, which is one behaviour with per-target implementations by design;
- a hand-written narrow implementation that a dependency could nominally cover, **when the module
  doc states what the dependency does not give and why it is needed** — Argos decodes JPEG entropy
  without a full decoder for exactly that reason, and it says so where it lives. An
  undocumented reimplementation of something a dependency already does is a finding; a documented
  one is a decision.

## What is removed takes its dependants with it (A-NO-ORPHANS) { #A-NO-ORPHANS }

<why>a feature is deleted in one commit and the things that existed only to serve it survive
untouched, because nothing references them and nothing complains.</why>

Removing a capability removes, in the same change, everything that existed only to serve it:
fixtures, corpora, generators, model files and weights, benchmarks, fuzz targets, dependencies,
`[workspace.dependencies]` entries, `.gitignore` rules, CI lanes and documentation sections.

Orphans are the single most common form of accumulation in an agent-built codebase, because each
removal is scoped to what the change was about and nothing points back from the residue to the
thing that is gone.

This is the retirement counterpart to `M-LINT-OVERRIDE-EXPECT`, which is the one rule in the book
that already works this way: an `expect` that stops being needed warns, so the lint set cannot rot.
Nothing warns about an orphaned corpus, so it is written down instead.

Note the tooling gap this exists to cover: `cargo udeps` reads per-crate manifests and does not see
an unused entry in `[workspace.dependencies]`, so CI stays green while the dependency of a deleted
feature sits in the root manifest.

## Support directories say who runs them (A-SUPPORT-DECLARED) { #A-SUPPORT-DECLARED }

<why>a directory of non-source files is indistinguishable from abandoned scaffolding unless it
states its purpose, and the cost of guessing wrong is deleting the thing that proves a parser
survives hostile input.</why>

Every directory that holds something other than shipped source — `fuzz/`, `benches/`, `examples/`,
`corpus/`, fixture and model directories — carries one line, in its own README or in the manifest
that owns it, saying **what runs it and what it guarantees**:

```text
Run by the `fuzz` lane in .github/workflows/ci.yml; proves every parser returns errors on
hostile input rather than panicking (A-FUZZ-EVERY-PARSER).
```

A support directory that no lane runs and no rule requires is not support. It is weight, and it is
removed under `A-EARNS-ITS-PLACE`.

Corpus directories additionally hold **only authored seeds**. A fuzzer's own output written back
into the tree is not a seed: it is unbounded, it is named by hash, it carries no intent, and it
grows without limit. Seeds are produced by a committed generator and named for the case they cover.

## No surface without a consumer (A-NO-SPECULATIVE-SURFACE) { #A-NO-SPECULATIVE-SURFACE }

<why>speculative generality is the most recognisable thing an agent adds and the most expensive
thing to carry: every extension point is a place a reader must understand and a maintainer must
keep working, paid for by a use that never arrives.</why>

A trait with one implementor, a type parameter instantiated once, a function parameter passed the
same value at every call site, an enum variant nothing constructs, a cargo feature nothing enables
and a configuration knob nothing sets are all the same defect. Collapse the surface to what is
used; the general form can be reintroduced when a second consumer exists, and it will be a better
generalisation for having two real cases instead of one imagined one.

Two exemptions, both because another rule requires the surface:

- a builder at four or more construction permutations (`M-INIT-BUILDER`);
- an inner `ErrorKind` on a canonical error struct (`M-ERRORS-CANONICAL-STRUCTS`).

Where those two rules apply, the surface is mandated and this rule does not reach it. Everywhere
else, one consumer or none means the abstraction has not earned itself.

The tell that separates this from ordinary design: ask what breaks if the parameter is deleted and
its single value inlined. If the answer is "nothing, and the code reads better", it was
speculative.
