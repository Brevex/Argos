# Workspace, crates, features and toolchain — guideline reference

<!-- Rule text adapted from the Pragmatic Rust Guidelines, Copyright (c) Microsoft Corporation, MIT license. -->

**This file is the authoritative text of these rules for the Argos project.** It is
the source, not a copy: treat the wording here as normative and edit it directly when
the project's standard changes. Cite rules by id (e.g. `M-CARGO-WORKSPACE`).

Rules in this file (14): `M-CARGO-WORKSPACE`, `M-CRATES-FLAT-FOLDER`, `M-CRATES-IN-WORKSPACE`, `M-FEATURES-ADDITIVE`, `M-LATEST-EDITION`, `M-LINT-OVERRIDE-EXPECT`, `M-MIMALLOC-APPS`, `M-MSRV`, `M-OOBE`, `M-SMALLER-CRATES`, `M-STATIC-VERIFICATION`, `M-SYS-CRATES`, `M-TARGET-CPU`, `M-UPSTREAM-GUIDELINES`

Decision layer and Argos-specific application: [SKILL.md](SKILL.md). Full rule map: [guidelines index](../../guidelines-index.md).

---

## Use mimalloc for apps (M-MIMALLOC-APPS) { #M-MIMALLOC-APPS }

<why>significant performance at no cost.</why>

Applications should set [mimalloc](https://crates.io/crates/mimalloc) as their global allocator. This usually results in notable performance
increases along allocating hot paths; we have seen up to 25% benchmark improvements.

Changing the allocator only takes a few lines of code. Add mimalloc to your `Cargo.toml` like so:

```toml
[dependencies]
mimalloc = { version = "0.1" } # Or later version if available
```

Then use it from your `main.rs`:

```rust,ignore
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
```

## Applications target highest viable target-cpu (M-TARGET-CPU) { #M-TARGET-CPU }

<why>fleet performance.</why>

Server applications should compile against the highest `target-cpu` that the deployment environment is guaranteed to support, rather than defaulting to the generic baseline.

This can be achieved, for example, by setting inside `.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=x86-64-v3"]

[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-cpu=x86-64-v3"]

# Add other platforms here based on needs ...
```

Note this guideline applies only to applications, as target settings are ignored for libraries.

## Common settings come from the workspace Cargo.toml (M-CARGO-WORKSPACE) { #M-CARGO-WORKSPACE }

<why>consistent, maintainable project configuration.</why>

Any repo with two or more crates that somehow belong together should unify these crates with a workspace `Cargo.toml`. Members then inherit shared metadata and dependency versions from the workspace root via `[workspace.dependencies]`, `[workspace.lints]`, ... rather than duplicating these values in each crate.

Where a dependency is crate-specific, it should still be defined in the workspace. Workspace definitions should generally not enable dependency features (except basic ones such as `["std"]`), and should instead use `default-features = false`.

## All crates are siblings in one folder (M-CRATES-FLAT-FOLDER) { #M-CRATES-FLAT-FOLDER }

<why>simple project navigation and a standard Rust layout.</why>

A repository should contain a single workspace `Cargo.toml`, and all Rust crates should be subordinate to it. All crates should then live in a single, direct subdirectory (e.g., `crates/`) below the workspace (for up to 1-2 dozen of crates), beyond that some folder hierarchy should be used (e.g., `common/`, `server/`, `client/`) to organize siblings.

```bash
# Ideal for most workspaces
Cargo.toml
crates/
  foo/Cargo.toml 
  foo_core/Cargo.toml 
  foo_proc/Cargo.toml 
  foo_tests/Cargo.toml 
  bar/Cargo.toml
  baz/Cargo.toml


# Ok for large workspaces
Cargo.toml
crates/
  server/
    main/Cargo.toml 
    routes/Cargo.toml 
  client/
    foo/Cargo.toml
    bar/Cargo.toml
  common/
    error/Cargo.toml
```

Placing crates inside other crates (at or below their `Cargo.toml`), or even inside their `src/` folder is never acceptable. If a crate relationship should be expressed, this is done via common prefixes instead (e.g., `foo`, `foo_util`, `foo_build`).

```bash
# Never acceptable, crates inside `src/` folder
Cargo.toml
crates/
  foo/Cargo.toml 
    src/lib.rs
       deps/bar/Cargo.toml 
```

Rare exceptions to this rule can occur if your crate is in the business of processing workspaces and has a collection of UI tests or similar it relies on; but even then these are usually dummy crates in nature.

## The workspace lists and versions all crates (M-CRATES-IN-WORKSPACE) { #M-CRATES-IN-WORKSPACE }

<why>simple inter-crate dependencies and debugging.</why>

Every crate produced by the project should be listed as a workspace member, and its version should be declared in `[workspace.dependencies]` so that intra-workspace dependencies resolve to a single canonical version.

```toml
# Bad, crate links its sibling directly
[dependencies]
sibling.path = "../sibling"


# Good, going through workspace
[dependencies]
sibling.workspace = true

[workspace.dependencies]
sibling = { path = "crates/sibling", version = "0.5.2" }
```

## New crates target latest edition (M-LATEST-EDITION) { #M-LATEST-EDITION }

<why>access to the latest Rust features.</why>

When creating a new crate or workspace, set `edition` to the latest stable edition (at least `2024` at the time of writing); the `resolver` field is generally not needed.

Using an older edition generally has no upsides for new projects, but forces you to write 'old Rust' that is less idiomatic and has worse UX edge cases. Notably, using an older edition does _not_ grant any compatibility benefits with the rest of the ecosystem. An application based on `2015` can use libraries written for `2024` just fine.

## MSRV is conservatively updated (M-MSRV) { #M-MSRV }

<why>modern features with stability for users.</why>

A Minimum Supported Rust Version (MSRV) should be set when libraries are first created. It can be updated as new Rust features are needed, but should be kept a few versions behind the most recent compiler release.

The ecosystem expectation is that projects are compiled with a _reasonably modern_ Rust compiler.

Bumping MSRV therefore does not require a major release, but can be handled through a minor update (e.g., `1.3` to `1.4`). In fact, any project depending on 3rd party crates is already inherently bound to this contract; forcing a major version bump will not confer any benefits, but could possibly bifurcate downstream dependencies.

## Lint overrides should use `#[expect]` (M-LINT-OVERRIDE-EXPECT) { #M-LINT-OVERRIDE-EXPECT }

<why>a current, tidy lint set.</why>

When overriding project-global lints inside a submodule or item, you should do so via `#[expect]`, not `#[allow]`.

Expected lints emit a warning if the marked warning was not encountered, thus preventing the accumulation of stale lints.
That said, `#[allow]` lints are still useful when applied to generated code, and can appear in macros.

Overrides should be accompanied by a `reason`:

```rust,edition2021
#[expect(clippy::unused_async, reason = "API fixed, will use I/O later")]
pub async fn ping_server() {
  // Stubbed out for now
}
```

## If in doubt, split the crate (M-SMALLER-CRATES) { #M-SMALLER-CRATES }

<why>fast compile times and good modularity.</why>

You should err on the side of having too many crates rather than too few, as this leads to dramatic compile time improvements—especially
during the development of these crates—and prevents cyclic component dependencies.

Essentially, if a submodule can be used independently, its contents should be moved into a separate crate.

Performing this crate split may cause you to lose access to some `pub(crate)` fields or methods. In many situations, this is a desirable
side-effect and should prompt you to design more flexible abstractions that would give your users similar affordances.

In some cases, it is desirable to re-join individual crates back into a single _umbrella crate_, such as when dealing with proc macros, or runtimes.
Functionality split for technical reasons (e.g., a `foo_proc` proc macro crate) should always be re-exported. Otherwise, re-exports should be used sparingly.

> ### <tip></tip> Features vs. Crates
>
> As a rule of thumb, crates are for items that can reasonably be used on their own. Features should unlock extra functionality that
> can't live on its own. In the case of umbrella crates, see below, features may also be used to enable constituents (but then that functionality
> was extracted into crates already).
>
> For example, if you defined a `web` crate with the following modules, users only needing client calls would also have to pay for the compilation of server code:
>
> ```text
> web::server
> web::client
> web::protocols
> ```
>
> Instead, you should introduce individual crates that give users the ability to pick and choose:
>
> ```text
> web_server
> web_client
> web_protocols
> ```

## Use static verification (M-STATIC-VERIFICATION) { #M-STATIC-VERIFICATION }

<why>consistency and freedom from common issues.</why>

Projects should use the following static verification tools to help maintain the quality of the code. These tools can be
configured to run on a developer's machine during normal work, and should be used as part of check-in gates.

* [compiler lints](https://doc.rust-lang.org/rustc/lints/index.html) offer many lints to avoid bugs and improve code quality.
* [clippy lints](https://doc.rust-lang.org/clippy/) contain hundreds of lints to avoid bugs and improve code quality.
* [rustfmt](https://github.com/rust-lang/rustfmt) ensures consistent source formatting.
* [cargo-audit](https://crates.io/crates/cargo-audit) verifies crate dependencies for security vulnerabilities.
* [cargo-hack](https://crates.io/crates/cargo-hack) validates that all combinations of crate features work correctly.
* [cargo-udeps](https://crates.io/crates/cargo-udeps) detects unused dependencies in Cargo.toml files.
* [miri](https://github.com/rust-lang/miri) validates the correctness of unsafe code.

### Compiler Lints

The Rust compiler generally produces exceptionally good diagnostics. In addition to the default set of diagnostics, projects
should explicitly enable the following set of compiler lints:

```toml
[lints.rust]
ambiguous_negative_literals = "warn"
missing_debug_implementations = "warn"
redundant_imports = "warn"
redundant_lifetimes = "warn"
trivial_numeric_casts = "warn"
unsafe_op_in_unsafe_fn = "warn"
unused_lifetimes = "warn"
```

### Clippy Lints

For clippy, projects should enable all major lint categories, and additionally enable some lints from the `restriction` lint group.
Undesired lints (e.g., numeric casts) can be opted back out of on a case-by-case basis:

```toml
[lints.clippy]
cargo = { level = "warn", priority = -1 }
complexity = { level = "warn", priority = -1 }
correctness = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
perf = { level = "warn", priority = -1 }
style = { level = "warn", priority = -1 }
suspicious = { level = "warn", priority = -1 }
# nursery = { level = "warn", priority = -1 }  # optional, might cause more false positives

# Additional lints are from the `restriction` and `nursery` group that prevent specific
# constructs being used in source code in order to drive up consistency,
# quality, and brevity
allow_attributes_without_reason = "warn"
as_pointer_underscore = "warn"
assertions_on_result_states = "warn"
clone_on_ref_ptr = "warn"
deref_by_slicing = "warn"
disallowed_script_idents = "warn"
empty_drop = "warn"
empty_enum_variants_with_brackets = "warn"
empty_structs_with_brackets = "warn"
fn_to_numeric_cast_any = "warn"
if_then_some_else_none = "warn"
map_err_ignore = "warn"
redundant_type_annotations = "warn"
renamed_function_params = "warn"
semicolon_outside_block = "warn"
undocumented_unsafe_blocks = "warn"
unnecessary_safety_comment = "warn"
unnecessary_safety_doc = "warn"
unneeded_field_pattern = "warn"
unused_result_ok = "warn"
too_long_first_doc_paragraph = "warn"

# May cause issues with structured logging otherwise.
literal_string_with_formatting_args = "allow"

# Define custom opt outs here
# ...
```

## Follow the upstream guidelines (M-UPSTREAM-GUIDELINES) { #M-UPSTREAM-GUIDELINES }

<why>a codebase that reflects community lessons and does not surprise users or contributors.</why>

The guidelines in this book complement existing Rust guidelines, in particular:

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)
- [Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/)
- [Rust Design Patterns](https://rust-unofficial.github.io/patterns//intro.html)
- [Rust Reference - Undefined Behavior](https://doc.rust-lang.org/reference/behavior-considered-undefined.html)

We recommend you read through these as well, and apply them in addition to this book's items. Pay special attention to the ones below, as they are frequently forgotten:

- [ ] [C-CONV](https://rust-lang.github.io/api-guidelines/naming.html#ad-hoc-conversions-follow-as_-to_-into_-conventions-c-conv) - Ad-hoc conversions
  follow  `as_`, `to_`, `into_` conventions
- [ ] [C-GETTER](https://rust-lang.github.io/api-guidelines/naming.html#getter-names-follow-rust-convention-c-getter) - Getter names follow Rust convention
- [ ] [C-COMMON-TRAITS](https://rust-lang.github.io/api-guidelines/interoperability.html#c-common-traits) - Types eagerly implement common traits
  - `Copy`, `Clone`, `Eq`, `PartialEq`, `Ord`, `PartialOrd`, `Hash`, `Default`, `Debug`
  - `Display` where type wants to be displayed
- [ ] [C-CTOR](https://rust-lang.github.io/api-guidelines/predictability.html?highlight=new#constructors-are-static-inherent-methods-c-ctor) -
  Constructors are static, inherent methods
  - In particular, have `Foo::new()`, even if you have `Foo::default()`
- [ ] [C-FEATURE](https://rust-lang.github.io/api-guidelines/naming.html#feature-names-are-free-of-placeholder-words-c-feature) - Feature names
  are free of placeholder words

## Features are additive (M-FEATURES-ADDITIVE) { #M-FEATURES-ADDITIVE }

<why>reliable compilation in large, complex projects.</why>

All library features must be additive, and any combination must work, as long as the feature itself would work on the current platform. This implies:

- [ ] You must not introduce a `no-std` feature, use a `std` feature instead
- [ ] Adding any feature `foo` must not disable or modify any public item
  - Adding enum variants is fine if these enums are `#[non_exhaustive]`
- [ ] Features must not rely on other features to be manually enabled
- [ ] Features must not rely on their parent to skip-enable a feature in one of their children

Further Reading

- [Feature Unification](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification)
- [Mutually Exclusive Features](https://doc.rust-lang.org/cargo/reference/features.html#mutually-exclusive-features)

## Libraries work out of the box (M-OOBE) { #M-OOBE }

<why>easy adoption across the Rust ecosystem.</why>

Libraries must _just work_ on all supported platforms, with the exception of libraries that are expressly platform or target specific.

Rust crates often come with dozens of dependencies, applications with 100's. Users expect `cargo build` and `cargo install`
to _just work_. Consider this installation of `bat` that pulls in ~250 dependencies:

```text
Compiling writeable v0.5.5
Compiling strsim v0.11.1
Compiling litemap v0.7.5
Compiling crossbeam-utils v0.8.21
Compiling icu_properties_data v1.5.1
Compiling ident_case v1.0.1
Compiling once_cell v1.21.3
Compiling icu_normalizer_data v1.5.1
Compiling fnv v1.0.7
Compiling regex-syntax v0.8.5
Compiling anstyle v1.0.10
Compiling vcpkg v0.2.15
Compiling utf8parse v0.2.2
Compiling aho-corasick v1.1.3
Compiling utf16_iter v1.0.5
Compiling hashbrown v0.15.2
Building [==>                       ] 29/251: icu_locid_transform_data, serde, winnow, indexma...
```

This compilation, like practically all other applications and libraries, will _just work_.

While there are tools targeting specific functionality (e.g., a Wayland compositor) or platform crates like
`windows`; unless a crate is _obviously_ platform specific, the expectation is that it will otherwise _just work_.

This means crates must build, ultimately

- [ ] on all [Tier 1 platforms](https://doc.rust-lang.org/rustc/platform-support.html),<sup>1</sup> and
- [ ] without any additional prerequisites beyond `cargo` and `rust`.<sup>2</sup>

<footnotes>

<sup>1</sup> It is ok to not support Tier 1 platforms "for now", but abstractions must be present so support can easily be extended. This is usually
done by introducing an internal `HAL` ([Hardware Abstraction Layer](https://en.wikipedia.org/wiki/HAL_(software))) module with a `dummy` fallback target.<br/>
<sup>2</sup> A default Rust installation will also have `cc` and a linker present.

</footnotes>

In particular, non-platform crates must not, by default, require the user to install additional tools, or expect environment variables
to compile. If tools were somehow needed (like the generation of Rust from `.proto` files) these tools should be run as part of the
publishing workflow or earlier, and the resulting artifacts (e.g., `.rs` files) be contained inside the published crate.

If a dependency is known to be platform specific, the parent must use conditional (platform) compilation or opt-in feature gates.

> **<alert></alert> Libraries are Responsible for Their Dependencies.**
>
> Imagine you author a `Copilot` crate, which in turn uses an `HttpClient`, which in turn depends on a `perl` script to compile.
>
> Then every one of your users, and your user's users, and everyone above, would need to install Perl to compile _their_ crate. In large projects you would
> have 100's of people who don't know or don't care about your library or Perl, encounter a cryptic compilation error, and now have to figure out how to
> install it on their system.
>
> In practical terms, such behavior is largely a self-inflicted death sentence in the open source space, since the moment alternatives
> are available, people will switch to those that _just work_.

## Native `-sys` crates compile without dependencies (M-SYS-CRATES) { #M-SYS-CRATES }

<why>libraries that just work on all platforms.</why>

If you author a pair of `foo` and `foo-sys` crates wrapping a native `foo.lib`, you are likely to run into the issues described
in [M-OOBE].

Follow these steps to produce a crate that _just works_ across platforms:

- [ ] fully govern the build of `foo.lib` from `build.rs` inside `foo-sys`. Only use hand-crafted compilation via the
  [cc](https://crates.io/crates/cc) crate, do _not_ run Makefiles or external build scripts, as that will require the installation of external dependencies,
- [ ] make all external tools optional, such as `nasm`,
- [ ] embed the upstream source code in your crate,
- [ ] make the embedded sources verifiable (e.g., include Git URL + hash),
- [ ] pre-generate `bindgen` glue if possible,
- [ ] support both static linking, and dynamic linking via [libloading](https://crates.io/crates/libloading).

Deviations from these points can work, and can be considered on a case-by-case basis:

If the native build system is available as an _OOBE_ crate, that can be used instead of `cc` invocations. The same applies to external tools.

Source code might have to be downloaded if it does not fit crates.io size limitations. In any case, only servers with an availability
comparable to crates.io should be used. In addition, the specific hashes of acceptable downloads should be stored in the crate and verified.

Downloading sources can fail on hermetic build environments, therefore alternative source roots should also be specifiable (e.g., via environment variables).

[M-OOBE]: #M-OOBE

