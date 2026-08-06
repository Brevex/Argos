# Guidelines index

The 89 `M-*` rules Argos follows, each mapped to exactly one skill. The rule text itself lives in
`skills/<skill>/reference.md` — **those files are the source**, not a copy of anything. This index
exists so a rule can be located by id in seconds and so the split stays free of duplicates: no rule
is reachable through two paths.

Rule text adapted from the Pragmatic Rust Guidelines, Copyright (c) Microsoft Corporation, MIT
license.

## Working with these rules

- Writing code: invoke the skill (its `SKILL.md` is the decision layer, with the Argos-specific
  application); open `reference.md` for the full text.
- Reviewing code: the four `agents/` reviewers read these same files and must cite an id for every
  finding.
- Changing a rule: edit the `reference.md` that owns it, and keep this index in sync. If you add a
  rule, give it an id, put it in exactly one file, and add a row here.

Integrity check — every id defined exactly once across all reference files:

```bash
grep -rhoE '^## .*\((M-[A-Z0-9-]+)\)' .claude/skills/*/reference.md \
  | grep -oE 'M-[A-Z0-9-]+' | sort | uniq -d          # must print nothing
grep -rhoE '^## .*\((M-[A-Z0-9-]+)\)' .claude/skills/*/reference.md | wc -l   # must print 89
```

## Rules per skill

| Skill | Scope | Rules |
| --- | --- | ---: |
| [rust-type-design](skills/rust-type-design/SKILL.md) | Types, traits, signatures and construction | 16 |
| [rust-workspace-setup](skills/rust-workspace-setup/SKILL.md) | Workspace, crates, features and toolchain | 14 |
| [rust-api-surface](skills/rust-api-surface/SKILL.md) | Naming, modules and public surface | 11 |
| [rust-errors-panics](skills/rust-errors-panics/SKILL.md) | Errors and panics | 7 |
| [rust-unsafe-ffi](skills/rust-unsafe-ffi/SKILL.md) | Unsafe, soundness and FFI | 7 |
| [rust-macros](skills/rust-macros/SKILL.md) | Macros | 7 |
| [rust-concurrency](skills/rust-concurrency/SKILL.md) | Concurrency, async and shared state | 7 |
| [rust-performance](skills/rust-performance/SKILL.md) | Hot paths, memory and allocations | 7 |
| [rust-docs](skills/rust-docs/SKILL.md) | Documentation | 6 |
| [rust-testing](skills/rust-testing/SKILL.md) | Tests and mockable I/O | 4 |
| [rust-telemetry](skills/rust-telemetry/SKILL.md) | Logging and telemetry | 3 |
| | **Total** | **89** |

## Rules by skill

Each id links to its text.

### rust-type-design — Types, traits, signatures and construction (16)

| Rule | Topic |
| --- | --- |
| [`M-PUBLIC-DEBUG`](skills/rust-type-design/reference.md#M-PUBLIC-DEBUG) | Universal |
| [`M-PUBLIC-DISPLAY`](skills/rust-type-design/reference.md#M-PUBLIC-DISPLAY) | Universal |
| [`M-IMPL-ASREF`](skills/rust-type-design/reference.md#M-IMPL-ASREF) | Libraries / Interoperability |
| [`M-IMPL-IO`](skills/rust-type-design/reference.md#M-IMPL-IO) | Libraries / Interoperability |
| [`M-IMPL-RANGEBOUNDS`](skills/rust-type-design/reference.md#M-IMPL-RANGEBOUNDS) | Libraries / Interoperability |
| [`M-BUILD-RESULT`](skills/rust-type-design/reference.md#M-BUILD-RESULT) | Libraries / Resilience |
| [`M-STRONG-TYPES-GUARD`](skills/rust-type-design/reference.md#M-STRONG-TYPES-GUARD) | Libraries / Resilience |
| [`M-STRONG-TYPES`](skills/rust-type-design/reference.md#M-STRONG-TYPES) | Libraries / Resilience |
| [`M-AVOID-WRAPPERS`](skills/rust-type-design/reference.md#M-AVOID-WRAPPERS) | Libraries / UX |
| [`M-COLLECTION-TRAITS`](skills/rust-type-design/reference.md#M-COLLECTION-TRAITS) | Libraries / UX |
| [`M-DI-HIERARCHY`](skills/rust-type-design/reference.md#M-DI-HIERARCHY) | Libraries / UX |
| [`M-ESSENTIAL-FN-INHERENT`](skills/rust-type-design/reference.md#M-ESSENTIAL-FN-INHERENT) | Libraries / UX |
| [`M-INIT-BUILDER`](skills/rust-type-design/reference.md#M-INIT-BUILDER) | Libraries / UX |
| [`M-INIT-CASCADED`](skills/rust-type-design/reference.md#M-INIT-CASCADED) | Libraries / UX |
| [`M-PARAMETER-CONSISTENCY`](skills/rust-type-design/reference.md#M-PARAMETER-CONSISTENCY) | Libraries / UX |
| [`M-SIMPLE-ABSTRACTIONS`](skills/rust-type-design/reference.md#M-SIMPLE-ABSTRACTIONS) | Libraries / UX |

### rust-workspace-setup — Workspace, crates, features and toolchain (14)

| Rule | Topic |
| --- | --- |
| [`M-MIMALLOC-APPS`](skills/rust-workspace-setup/reference.md#M-MIMALLOC-APPS) | Application |
| [`M-TARGET-CPU`](skills/rust-workspace-setup/reference.md#M-TARGET-CPU) | Application |
| [`M-CARGO-WORKSPACE`](skills/rust-workspace-setup/reference.md#M-CARGO-WORKSPACE) | Project |
| [`M-CRATES-FLAT-FOLDER`](skills/rust-workspace-setup/reference.md#M-CRATES-FLAT-FOLDER) | Project |
| [`M-CRATES-IN-WORKSPACE`](skills/rust-workspace-setup/reference.md#M-CRATES-IN-WORKSPACE) | Project |
| [`M-LATEST-EDITION`](skills/rust-workspace-setup/reference.md#M-LATEST-EDITION) | Project |
| [`M-MSRV`](skills/rust-workspace-setup/reference.md#M-MSRV) | Project |
| [`M-LINT-OVERRIDE-EXPECT`](skills/rust-workspace-setup/reference.md#M-LINT-OVERRIDE-EXPECT) | Universal |
| [`M-SMALLER-CRATES`](skills/rust-workspace-setup/reference.md#M-SMALLER-CRATES) | Universal |
| [`M-STATIC-VERIFICATION`](skills/rust-workspace-setup/reference.md#M-STATIC-VERIFICATION) | Universal |
| [`M-UPSTREAM-GUIDELINES`](skills/rust-workspace-setup/reference.md#M-UPSTREAM-GUIDELINES) | Universal |
| [`M-FEATURES-ADDITIVE`](skills/rust-workspace-setup/reference.md#M-FEATURES-ADDITIVE) | Libraries / Building |
| [`M-OOBE`](skills/rust-workspace-setup/reference.md#M-OOBE) | Libraries / Building |
| [`M-SYS-CRATES`](skills/rust-workspace-setup/reference.md#M-SYS-CRATES) | Libraries / Building |

### rust-api-surface — Naming, modules and public surface (11)

| Rule | Topic |
| --- | --- |
| [`M-DESIGN-FOR-AI`](skills/rust-api-surface/reference.md#M-DESIGN-FOR-AI) | AI |
| [`M-RUST-SHAPED`](skills/rust-api-surface/reference.md#M-RUST-SHAPED) | AI |
| [`M-SINGLE-ITEM-PATH`](skills/rust-api-surface/reference.md#M-SINGLE-ITEM-PATH) | AI |
| [`M-REGULAR-FN`](skills/rust-api-surface/reference.md#M-REGULAR-FN) | Universal |
| [`M-SHORT-NAMES`](skills/rust-api-surface/reference.md#M-SHORT-NAMES) | Universal |
| [`M-WEASEL-WORDS`](skills/rust-api-surface/reference.md#M-WEASEL-WORDS) | Universal |
| [`M-DONT-LEAK-TYPES`](skills/rust-api-surface/reference.md#M-DONT-LEAK-TYPES) | Libraries / Interoperability |
| [`M-FOREIGN-REEXPORTS`](skills/rust-api-surface/reference.md#M-FOREIGN-REEXPORTS) | Libraries / Interoperability |
| [`M-NO-GLOB-REEXPORTS`](skills/rust-api-surface/reference.md#M-NO-GLOB-REEXPORTS) | Libraries / Resilience |
| [`M-BALANCED-MODULES`](skills/rust-api-surface/reference.md#M-BALANCED-MODULES) | Libraries / UX |
| [`M-NO-PRELUDE`](skills/rust-api-surface/reference.md#M-NO-PRELUDE) | Libraries / UX |

### rust-errors-panics — Errors and panics (7)

| Rule | Topic |
| --- | --- |
| [`M-APP-ERROR`](skills/rust-errors-panics/reference.md#M-APP-ERROR) | Application |
| [`M-PANIC-CONTINUATION`](skills/rust-errors-panics/reference.md#M-PANIC-CONTINUATION) | Correctness |
| [`M-PANIC-IS-STOP`](skills/rust-errors-panics/reference.md#M-PANIC-IS-STOP) | Correctness |
| [`M-PANIC-MESSAGE`](skills/rust-errors-panics/reference.md#M-PANIC-MESSAGE) | Correctness |
| [`M-PANIC-ON-BUG`](skills/rust-errors-panics/reference.md#M-PANIC-ON-BUG) | Correctness |
| [`M-ERRORS-CANONICAL-STRUCTS`](skills/rust-errors-panics/reference.md#M-ERRORS-CANONICAL-STRUCTS) | Libraries / UX |
| [`M-FROM-ERROR`](skills/rust-errors-panics/reference.md#M-FROM-ERROR) | Libraries / UX |

### rust-unsafe-ffi — Unsafe, soundness and FFI (7)

| Rule | Topic |
| --- | --- |
| [`M-UNSAFE-IMPLIES-UB`](skills/rust-unsafe-ffi/reference.md#M-UNSAFE-IMPLIES-UB) | Correctness |
| [`M-UNSAFE`](skills/rust-unsafe-ffi/reference.md#M-UNSAFE) | Correctness |
| [`M-UNSOUND`](skills/rust-unsafe-ffi/reference.md#M-UNSOUND) | Correctness |
| [`M-FFI-NAMING`](skills/rust-unsafe-ffi/reference.md#M-FFI-NAMING) | FFI |
| [`M-FFI-TRANSLATES`](skills/rust-unsafe-ffi/reference.md#M-FFI-TRANSLATES) | FFI |
| [`M-ISOLATE-DLL-STATE`](skills/rust-unsafe-ffi/reference.md#M-ISOLATE-DLL-STATE) | FFI |
| [`M-ESCAPE-HATCHES`](skills/rust-unsafe-ffi/reference.md#M-ESCAPE-HATCHES) | Libraries / Interoperability |

### rust-macros — Macros (7)

| Rule | Topic |
| --- | --- |
| [`M-EXAMPLE-OVER-PROC`](skills/rust-macros/reference.md#M-EXAMPLE-OVER-PROC) | Macros |
| [`M-MACRO-HELPERS`](skills/rust-macros/reference.md#M-MACRO-HELPERS) | Macros |
| [`M-MACRO-LAST-RESORT`](skills/rust-macros/reference.md#M-MACRO-LAST-RESORT) | Macros |
| [`M-MACRO-MAIN-CRATE`](skills/rust-macros/reference.md#M-MACRO-MAIN-CRATE) | Macros |
| [`M-MACROS-DONT-LIE`](skills/rust-macros/reference.md#M-MACROS-DONT-LIE) | Macros |
| [`M-PROC-IMPL`](skills/rust-macros/reference.md#M-PROC-IMPL) | Macros |
| [`M-PROC-IMPLIED-ITEMS`](skills/rust-macros/reference.md#M-PROC-IMPLIED-ITEMS) | Macros |

### rust-concurrency — Concurrency, async and shared state (7)

| Rule | Topic |
| --- | --- |
| [`M-ASYNC-STACK-SIZE`](skills/rust-concurrency/reference.md#M-ASYNC-STACK-SIZE) | Performance |
| [`M-THROUGHPUT`](skills/rust-concurrency/reference.md#M-THROUGHPUT) | Performance |
| [`M-YIELD-POINTS`](skills/rust-concurrency/reference.md#M-YIELD-POINTS) | Performance |
| [`M-TYPES-SEND`](skills/rust-concurrency/reference.md#M-TYPES-SEND) | Libraries / Interoperability |
| [`M-AVOID-STATICS`](skills/rust-concurrency/reference.md#M-AVOID-STATICS) | Libraries / Resilience |
| [`M-ASYNC-FN`](skills/rust-concurrency/reference.md#M-ASYNC-FN) | Libraries / UX |
| [`M-SERVICES-CLONE`](skills/rust-concurrency/reference.md#M-SERVICES-CLONE) | Libraries / UX |

### rust-performance — Hot paths, memory and allocations (7)

| Rule | Topic |
| --- | --- |
| [`M-AVOID-INDIRECTION`](skills/rust-performance/reference.md#M-AVOID-INDIRECTION) | Performance |
| [`M-BOX-DST`](skills/rust-performance/reference.md#M-BOX-DST) | Performance |
| [`M-FAST-HASHER`](skills/rust-performance/reference.md#M-FAST-HASHER) | Performance |
| [`M-HOTPATH`](skills/rust-performance/reference.md#M-HOTPATH) | Performance |
| [`M-INITIAL-CAPACITY`](skills/rust-performance/reference.md#M-INITIAL-CAPACITY) | Performance |
| [`M-MEM-REUSE`](skills/rust-performance/reference.md#M-MEM-REUSE) | Performance |
| [`M-SHRINK-TO-FIT`](skills/rust-performance/reference.md#M-SHRINK-TO-FIT) | Performance |

### rust-docs — Documentation (6)

| Rule | Topic |
| --- | --- |
| [`M-NO-META-DESIGN-DOCUMENTATION`](skills/rust-docs/reference.md#M-NO-META-DESIGN-DOCUMENTATION) | AI |
| [`M-CANONICAL-DOCS`](skills/rust-docs/reference.md#M-CANONICAL-DOCS) | Documentation |
| [`M-DOC-INLINE`](skills/rust-docs/reference.md#M-DOC-INLINE) | Documentation |
| [`M-FIRST-DOC-SENTENCE`](skills/rust-docs/reference.md#M-FIRST-DOC-SENTENCE) | Documentation |
| [`M-MODULE-DOCS`](skills/rust-docs/reference.md#M-MODULE-DOCS) | Documentation |
| [`M-DOCUMENTED-MAGIC`](skills/rust-docs/reference.md#M-DOCUMENTED-MAGIC) | Universal |

### rust-testing — Tests and mockable I/O (4)

| Rule | Topic |
| --- | --- |
| [`M-TAUTOLOGICAL-TESTS`](skills/rust-testing/reference.md#M-TAUTOLOGICAL-TESTS) | AI |
| [`M-INTEGRATION-TESTS`](skills/rust-testing/reference.md#M-INTEGRATION-TESTS) | Libraries / Resilience |
| [`M-MOCKABLE-SYSCALLS`](skills/rust-testing/reference.md#M-MOCKABLE-SYSCALLS) | Libraries / Resilience |
| [`M-TEST-UTIL`](skills/rust-testing/reference.md#M-TEST-UTIL) | Libraries / Resilience |

### rust-telemetry — Logging and telemetry (3)

| Rule | Topic |
| --- | --- |
| [`M-LOG-OVERHEAD`](skills/rust-telemetry/reference.md#M-LOG-OVERHEAD) | Performance |
| [`M-LOG-STRUCTURED`](skills/rust-telemetry/reference.md#M-LOG-STRUCTURED) | Universal |
| [`M-LOG-NOT-PRINT`](skills/rust-telemetry/reference.md#M-LOG-NOT-PRINT) | Libraries / Resilience |

## Alphabetical

| Rule | Skill |
| --- | --- |
| [`M-APP-ERROR`](skills/rust-errors-panics/reference.md#M-APP-ERROR) | rust-errors-panics |
| [`M-ASYNC-FN`](skills/rust-concurrency/reference.md#M-ASYNC-FN) | rust-concurrency |
| [`M-ASYNC-STACK-SIZE`](skills/rust-concurrency/reference.md#M-ASYNC-STACK-SIZE) | rust-concurrency |
| [`M-AVOID-INDIRECTION`](skills/rust-performance/reference.md#M-AVOID-INDIRECTION) | rust-performance |
| [`M-AVOID-STATICS`](skills/rust-concurrency/reference.md#M-AVOID-STATICS) | rust-concurrency |
| [`M-AVOID-WRAPPERS`](skills/rust-type-design/reference.md#M-AVOID-WRAPPERS) | rust-type-design |
| [`M-BALANCED-MODULES`](skills/rust-api-surface/reference.md#M-BALANCED-MODULES) | rust-api-surface |
| [`M-BOX-DST`](skills/rust-performance/reference.md#M-BOX-DST) | rust-performance |
| [`M-BUILD-RESULT`](skills/rust-type-design/reference.md#M-BUILD-RESULT) | rust-type-design |
| [`M-CANONICAL-DOCS`](skills/rust-docs/reference.md#M-CANONICAL-DOCS) | rust-docs |
| [`M-CARGO-WORKSPACE`](skills/rust-workspace-setup/reference.md#M-CARGO-WORKSPACE) | rust-workspace-setup |
| [`M-COLLECTION-TRAITS`](skills/rust-type-design/reference.md#M-COLLECTION-TRAITS) | rust-type-design |
| [`M-CRATES-FLAT-FOLDER`](skills/rust-workspace-setup/reference.md#M-CRATES-FLAT-FOLDER) | rust-workspace-setup |
| [`M-CRATES-IN-WORKSPACE`](skills/rust-workspace-setup/reference.md#M-CRATES-IN-WORKSPACE) | rust-workspace-setup |
| [`M-DESIGN-FOR-AI`](skills/rust-api-surface/reference.md#M-DESIGN-FOR-AI) | rust-api-surface |
| [`M-DI-HIERARCHY`](skills/rust-type-design/reference.md#M-DI-HIERARCHY) | rust-type-design |
| [`M-DOC-INLINE`](skills/rust-docs/reference.md#M-DOC-INLINE) | rust-docs |
| [`M-DOCUMENTED-MAGIC`](skills/rust-docs/reference.md#M-DOCUMENTED-MAGIC) | rust-docs |
| [`M-DONT-LEAK-TYPES`](skills/rust-api-surface/reference.md#M-DONT-LEAK-TYPES) | rust-api-surface |
| [`M-ERRORS-CANONICAL-STRUCTS`](skills/rust-errors-panics/reference.md#M-ERRORS-CANONICAL-STRUCTS) | rust-errors-panics |
| [`M-ESCAPE-HATCHES`](skills/rust-unsafe-ffi/reference.md#M-ESCAPE-HATCHES) | rust-unsafe-ffi |
| [`M-ESSENTIAL-FN-INHERENT`](skills/rust-type-design/reference.md#M-ESSENTIAL-FN-INHERENT) | rust-type-design |
| [`M-EXAMPLE-OVER-PROC`](skills/rust-macros/reference.md#M-EXAMPLE-OVER-PROC) | rust-macros |
| [`M-FAST-HASHER`](skills/rust-performance/reference.md#M-FAST-HASHER) | rust-performance |
| [`M-FEATURES-ADDITIVE`](skills/rust-workspace-setup/reference.md#M-FEATURES-ADDITIVE) | rust-workspace-setup |
| [`M-FFI-NAMING`](skills/rust-unsafe-ffi/reference.md#M-FFI-NAMING) | rust-unsafe-ffi |
| [`M-FFI-TRANSLATES`](skills/rust-unsafe-ffi/reference.md#M-FFI-TRANSLATES) | rust-unsafe-ffi |
| [`M-FIRST-DOC-SENTENCE`](skills/rust-docs/reference.md#M-FIRST-DOC-SENTENCE) | rust-docs |
| [`M-FOREIGN-REEXPORTS`](skills/rust-api-surface/reference.md#M-FOREIGN-REEXPORTS) | rust-api-surface |
| [`M-FROM-ERROR`](skills/rust-errors-panics/reference.md#M-FROM-ERROR) | rust-errors-panics |
| [`M-HOTPATH`](skills/rust-performance/reference.md#M-HOTPATH) | rust-performance |
| [`M-IMPL-ASREF`](skills/rust-type-design/reference.md#M-IMPL-ASREF) | rust-type-design |
| [`M-IMPL-IO`](skills/rust-type-design/reference.md#M-IMPL-IO) | rust-type-design |
| [`M-IMPL-RANGEBOUNDS`](skills/rust-type-design/reference.md#M-IMPL-RANGEBOUNDS) | rust-type-design |
| [`M-INIT-BUILDER`](skills/rust-type-design/reference.md#M-INIT-BUILDER) | rust-type-design |
| [`M-INIT-CASCADED`](skills/rust-type-design/reference.md#M-INIT-CASCADED) | rust-type-design |
| [`M-INITIAL-CAPACITY`](skills/rust-performance/reference.md#M-INITIAL-CAPACITY) | rust-performance |
| [`M-INTEGRATION-TESTS`](skills/rust-testing/reference.md#M-INTEGRATION-TESTS) | rust-testing |
| [`M-ISOLATE-DLL-STATE`](skills/rust-unsafe-ffi/reference.md#M-ISOLATE-DLL-STATE) | rust-unsafe-ffi |
| [`M-LATEST-EDITION`](skills/rust-workspace-setup/reference.md#M-LATEST-EDITION) | rust-workspace-setup |
| [`M-LINT-OVERRIDE-EXPECT`](skills/rust-workspace-setup/reference.md#M-LINT-OVERRIDE-EXPECT) | rust-workspace-setup |
| [`M-LOG-NOT-PRINT`](skills/rust-telemetry/reference.md#M-LOG-NOT-PRINT) | rust-telemetry |
| [`M-LOG-OVERHEAD`](skills/rust-telemetry/reference.md#M-LOG-OVERHEAD) | rust-telemetry |
| [`M-LOG-STRUCTURED`](skills/rust-telemetry/reference.md#M-LOG-STRUCTURED) | rust-telemetry |
| [`M-MACRO-HELPERS`](skills/rust-macros/reference.md#M-MACRO-HELPERS) | rust-macros |
| [`M-MACRO-LAST-RESORT`](skills/rust-macros/reference.md#M-MACRO-LAST-RESORT) | rust-macros |
| [`M-MACRO-MAIN-CRATE`](skills/rust-macros/reference.md#M-MACRO-MAIN-CRATE) | rust-macros |
| [`M-MACROS-DONT-LIE`](skills/rust-macros/reference.md#M-MACROS-DONT-LIE) | rust-macros |
| [`M-MEM-REUSE`](skills/rust-performance/reference.md#M-MEM-REUSE) | rust-performance |
| [`M-MIMALLOC-APPS`](skills/rust-workspace-setup/reference.md#M-MIMALLOC-APPS) | rust-workspace-setup |
| [`M-MOCKABLE-SYSCALLS`](skills/rust-testing/reference.md#M-MOCKABLE-SYSCALLS) | rust-testing |
| [`M-MODULE-DOCS`](skills/rust-docs/reference.md#M-MODULE-DOCS) | rust-docs |
| [`M-MSRV`](skills/rust-workspace-setup/reference.md#M-MSRV) | rust-workspace-setup |
| [`M-NO-GLOB-REEXPORTS`](skills/rust-api-surface/reference.md#M-NO-GLOB-REEXPORTS) | rust-api-surface |
| [`M-NO-META-DESIGN-DOCUMENTATION`](skills/rust-docs/reference.md#M-NO-META-DESIGN-DOCUMENTATION) | rust-docs |
| [`M-NO-PRELUDE`](skills/rust-api-surface/reference.md#M-NO-PRELUDE) | rust-api-surface |
| [`M-OOBE`](skills/rust-workspace-setup/reference.md#M-OOBE) | rust-workspace-setup |
| [`M-PANIC-CONTINUATION`](skills/rust-errors-panics/reference.md#M-PANIC-CONTINUATION) | rust-errors-panics |
| [`M-PANIC-IS-STOP`](skills/rust-errors-panics/reference.md#M-PANIC-IS-STOP) | rust-errors-panics |
| [`M-PANIC-MESSAGE`](skills/rust-errors-panics/reference.md#M-PANIC-MESSAGE) | rust-errors-panics |
| [`M-PANIC-ON-BUG`](skills/rust-errors-panics/reference.md#M-PANIC-ON-BUG) | rust-errors-panics |
| [`M-PARAMETER-CONSISTENCY`](skills/rust-type-design/reference.md#M-PARAMETER-CONSISTENCY) | rust-type-design |
| [`M-PROC-IMPL`](skills/rust-macros/reference.md#M-PROC-IMPL) | rust-macros |
| [`M-PROC-IMPLIED-ITEMS`](skills/rust-macros/reference.md#M-PROC-IMPLIED-ITEMS) | rust-macros |
| [`M-PUBLIC-DEBUG`](skills/rust-type-design/reference.md#M-PUBLIC-DEBUG) | rust-type-design |
| [`M-PUBLIC-DISPLAY`](skills/rust-type-design/reference.md#M-PUBLIC-DISPLAY) | rust-type-design |
| [`M-REGULAR-FN`](skills/rust-api-surface/reference.md#M-REGULAR-FN) | rust-api-surface |
| [`M-RUST-SHAPED`](skills/rust-api-surface/reference.md#M-RUST-SHAPED) | rust-api-surface |
| [`M-SERVICES-CLONE`](skills/rust-concurrency/reference.md#M-SERVICES-CLONE) | rust-concurrency |
| [`M-SHORT-NAMES`](skills/rust-api-surface/reference.md#M-SHORT-NAMES) | rust-api-surface |
| [`M-SHRINK-TO-FIT`](skills/rust-performance/reference.md#M-SHRINK-TO-FIT) | rust-performance |
| [`M-SIMPLE-ABSTRACTIONS`](skills/rust-type-design/reference.md#M-SIMPLE-ABSTRACTIONS) | rust-type-design |
| [`M-SINGLE-ITEM-PATH`](skills/rust-api-surface/reference.md#M-SINGLE-ITEM-PATH) | rust-api-surface |
| [`M-SMALLER-CRATES`](skills/rust-workspace-setup/reference.md#M-SMALLER-CRATES) | rust-workspace-setup |
| [`M-STATIC-VERIFICATION`](skills/rust-workspace-setup/reference.md#M-STATIC-VERIFICATION) | rust-workspace-setup |
| [`M-STRONG-TYPES`](skills/rust-type-design/reference.md#M-STRONG-TYPES) | rust-type-design |
| [`M-STRONG-TYPES-GUARD`](skills/rust-type-design/reference.md#M-STRONG-TYPES-GUARD) | rust-type-design |
| [`M-SYS-CRATES`](skills/rust-workspace-setup/reference.md#M-SYS-CRATES) | rust-workspace-setup |
| [`M-TARGET-CPU`](skills/rust-workspace-setup/reference.md#M-TARGET-CPU) | rust-workspace-setup |
| [`M-TAUTOLOGICAL-TESTS`](skills/rust-testing/reference.md#M-TAUTOLOGICAL-TESTS) | rust-testing |
| [`M-TEST-UTIL`](skills/rust-testing/reference.md#M-TEST-UTIL) | rust-testing |
| [`M-THROUGHPUT`](skills/rust-concurrency/reference.md#M-THROUGHPUT) | rust-concurrency |
| [`M-TYPES-SEND`](skills/rust-concurrency/reference.md#M-TYPES-SEND) | rust-concurrency |
| [`M-UNSAFE`](skills/rust-unsafe-ffi/reference.md#M-UNSAFE) | rust-unsafe-ffi |
| [`M-UNSAFE-IMPLIES-UB`](skills/rust-unsafe-ffi/reference.md#M-UNSAFE-IMPLIES-UB) | rust-unsafe-ffi |
| [`M-UNSOUND`](skills/rust-unsafe-ffi/reference.md#M-UNSOUND) | rust-unsafe-ffi |
| [`M-UPSTREAM-GUIDELINES`](skills/rust-workspace-setup/reference.md#M-UPSTREAM-GUIDELINES) | rust-workspace-setup |
| [`M-WEASEL-WORDS`](skills/rust-api-surface/reference.md#M-WEASEL-WORDS) | rust-api-surface |
| [`M-YIELD-POINTS`](skills/rust-concurrency/reference.md#M-YIELD-POINTS) | rust-concurrency |
