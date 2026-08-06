# Naming, modules and public surface — guideline reference

<!-- Rule text adapted from the Pragmatic Rust Guidelines, Copyright (c) Microsoft Corporation, MIT license. -->

**This file is the authoritative text of these rules for the Argos project.** It is
the source, not a copy: treat the wording here as normative and edit it directly when
the project's standard changes. Cite rules by id (e.g. `M-BALANCED-MODULES`).

Rules in this file (11): `M-BALANCED-MODULES`, `M-DESIGN-FOR-AI`, `M-DONT-LEAK-TYPES`, `M-FOREIGN-REEXPORTS`, `M-NO-GLOB-REEXPORTS`, `M-NO-PRELUDE`, `M-REGULAR-FN`, `M-RUST-SHAPED`, `M-SHORT-NAMES`, `M-SINGLE-ITEM-PATH`, `M-WEASEL-WORDS`

Decision layer and Argos-specific application: [SKILL.md](SKILL.md). Full rule map: [guidelines index](../../guidelines-index.md).

---

## Design with AI use in mind (M-DESIGN-FOR-AI) { #M-DESIGN-FOR-AI }

<why>maximum utility from agents working in your codebase.</why>

As a general rule, making APIs easier to use for humans also makes them easier to use by AI.
If you follow the guidelines in this book, you should be in good shape.

Rust's strong type system is a boon for agents, as their lack of genuine understanding can often be
counterbalanced by comprehensive compiler checks, which Rust provides in abundance.

With that said, there are a few guidelines which are particularly important to help make AI coding in Rust more effective:

* **Create Idiomatic Rust API Patterns**. The more your APIs, whether public or internal, look and feel like the majority of
Rust code in the world, the better it is for AI. Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)
along with the guidelines from the type and API design rules.

* **Provide Thorough Docs**. Agents love good detailed docs. Include docs for all of your modules and public items in your crate.
Assume the reader has a solid, but not expert, level of understanding of Rust, and that the reader understands the standard library.
Follow
[C-CRATE-DOC](https://rust-lang.github.io/api-guidelines/checklist.html#c-crate-doc),
[C-FAILURE](https://rust-lang.github.io/api-guidelines/checklist.html#c-failure),
[C-LINK](https://rust-lang.github.io/api-guidelines/checklist.html#c-link), and
[M-MODULE-DOCS](../rust-docs/reference.md#M-MODULE-DOCS)
[M-CANONICAL-DOCS](../rust-docs/reference.md#M-CANONICAL-DOCS).

* **Provide Thorough Examples**. Your documentation should have directly usable examples, the repository should include more elaborate ones.
Follow
[C-EXAMPLE](https://rust-lang.github.io/api-guidelines/checklist.html#c-example)
[C-QUESTION-MARK](https://rust-lang.github.io/api-guidelines/checklist.html#c-question-mark).

* **Use Strong Types**. Avoid [primitive obsession](https://refactoring.guru/smells/primitive-obsession) by using strong types with strict well-documented semantics.
Follow
[C-NEWTYPE](https://rust-lang.github.io/api-guidelines/checklist.html#c-newtype).

* **Make Your APIs Testable**. Design APIs which allow your customers to test their use of your API in unit tests. This might involve introducing some mocks, fakes,
or cargo features. AI agents need to be able to iterate quickly to prove that the code they are writing that calls your API is working
correctly.

* **Ensure Test Coverage**. Your own code should have good test coverage over observable behavior.
This enables agents to work in a mostly hands-off mode when refactoring.

## Rust code solves Rust problems (M-RUST-SHAPED) { #M-RUST-SHAPED }

<why>idiomatic code.</why>

When (automatically) porting C#, Java, C++, or similar code to Rust, technical constructs must not be copied 1-on-1.

It is prudent to separate domain aspects from language aspects. Domain aspects address business problems. An algorithm to compute prime numbers or logic for processing a customer table can (and should) work the same when translating between languages.

However, many patterns exist to solve problems particular to the ecosystem they stem from. The Rust ecosystem has its own problems, and these need to be addressed by idioms that work for Rust. These include

- error handling,
- management of tasks and threads,
- component abstractions and their lifetimes,
- ownership of parameters,
- the absence of 'object-oriented' programming,
- structural differences between interfaces and traits,
- and many others.

While some language constructs simply don't translate at all (e.g., compared to C#, Rust does not have any meaningful reflection), others are deceptively similar and might only bite months down the line (e.g., statics, compare [M-AVOID-STATICS](../rust-concurrency/reference.md#M-AVOID-STATICS)).

As a rule of thumb, structs and their methods can have vaguely similar names, flows, inputs and outputs, as far as their business functionality is concerned. However, any striking technical similarity between Rust and { C#, Java, Python, ... } implementations is indicative of deeper architectural problems; a `throw_if_null()` never makes sense.

## Items are only visible through one path (M-SINGLE-ITEM-PATH) { #M-SINGLE-ITEM-PATH }

<why>a single, clutter-free path to each type.</why>

Public items within a crate should be reachable only through one path. For example some `crate::db::Connection` should not also be visible as `crate::Connection`:

```rust,ignore
// Not OK
pub mod db {
    pub struct Connection;
}

pub use db::Connection;
```

This rule is often violated by agents creating or refactoring large code bases over several iterations. In an attempt to _simplify_ their task, they re-export items under multiple paths, often previous ones from before some change, instead of cleanly redesigning structures where it makes sense.

Note this only targets the duplication of user-facing items. Within a crate it is acceptable (and often unavoidable) to see the same item multiple times as export trees are constructed:

```rust,ignore
// OK
pub(crate) mod db {
    pub struct Connection;
}

pub use db::Connection;
```

Similarly, re-exports of foreign items are not covered by this rule, although they should follow [M-FOREIGN-REEXPORTS](#M-FOREIGN-REEXPORTS).

Likewise, this rule also does not apply to public-but-hidden `_private` modules needed by macros, compare [M-MACRO-HELPERS](../rust-macros/reference.md#M-MACRO-HELPERS).

## Prefer regular over associated functions (M-REGULAR-FN) { #M-REGULAR-FN }

<why>readability.</why>

Associated functions should primarily be used for instance creation, not general purpose computation.

In contrast to some OO languages, regular functions are first-class citizens in Rust and need no module or _class_ to host them. Functionality that
does not clearly belong to a receiver should therefore not reside in a type's `impl` block:

```rust, ignore
struct Database {}

impl Database {
    // Ok, associated function creates an instance
    fn new() -> Self {}

    // Ok, regular method with `&self` as receiver
    fn query(&self) {}

    // Not ok, this function is not directly related to `Database`,
    // it should therefore not live under `Database` as an associated
    // function.
    fn check_parameters(p: &str) {}
}

// As a regular function this is fine
fn check_parameters(p: &str) {}
```

Regular functions are more idiomatic, and reduce unnecessary noise on the caller side. Associated trait functions are perfectly idiomatic though:

```rust
pub trait Default {
    fn default() -> Self;
}

struct Foo;

impl Default for Foo {
    fn default() -> Self { Self }
}
```

## Names of items are short (M-SHORT-NAMES) { #M-SHORT-NAMES }

<why>idiomatic code.</why>

The Rust convention that item identifiers are short should be followed:

- identifiers should not compound more than 2 short words (`AppConfig` over `GlobalApplicationConfig`),
- module or crate information shouldn't be baked into prefixes (`foo::Id` over `foo::FooId`), in particular when the direct 'super' item is sufficiently descriptive - in these cases users are expected to disambiguate items locally via qualifiers where needed (`fn convert(foo::Id) -> bar::Id`).
- abbreviations are preferred (`CallbackFn` over `CallbackFunction`),

Any of these rules can be broken where it makes local sense, but on a per-crate bases these exceptions should be _exceptional_ and well motivated.

## Names are free of weasel words (M-WEASEL-WORDS) { #M-WEASEL-WORDS }

<why>readability.</why>

Symbol names, especially type and trait names, should be free of weasel words that do not meaningfully
add information. Common offenders include `Service`, `Manager`, and `Factory`.

While your library may very well contain or communicate with a booking service&mdash;or even hold an `HttpClient`
instance named `booking_service`&mdash;one should rarely encounter a `BookingService` _type_ in code.

An item handling many bookings can just be called `Bookings`. If it does anything more specific, then that quality
should be appended instead. It submits these items elsewhere? Calling it `BookingDispatcher` would be more helpful.

The same is true for `Manager`s. All code manages _something_, so that moniker is rarely useful. With rare
exceptions, life cycle issues should likewise not be made the subject of some manager. Items are created in whatever
way they are needed, their disposal is governed by `Drop`, and only `Drop`.

Regarding factories, at least the term should be avoided. While the concept `FooFactory` has its use, its canonical
Rust name is `Builder` (compare [M-INIT-BUILDER](../rust-type-design/reference.md#M-INIT-BUILDER)). A builder that can produce items repeatedly is still a builder.

In addition, accepting factories (builders) as parameters is an unidiomatic import of OO concepts into Rust. If
repeatable instantiation is required, functions should ask for an `impl Fn() -> Foo` over a `FooBuilder` or
similar. In contrast, standalone builders have their use, but primarily to reduce parametric permutation complexity
around optional values (again, [M-INIT-BUILDER](../rust-type-design/reference.md#M-INIT-BUILDER)).

## Don't leak external types (M-DONT-LEAK-TYPES) { #M-DONT-LEAK-TYPES }

<why>stable APIs and low long-term maintenance cost.</why>

Where possible, you should prefer `std`<sup>1</sup> types in public APIs over types coming from external crates. Exceptions should be carefully considered.

Any type in any public API will become part of that API's contract. Since `std` and constituents are the only crates
shipped by default, and since they come with a permanent stability guarantee, their types are the only ones that come without an interoperability risk.

A crate that exposes another crate's type is said to _leak_ that type.

For maximal long term stability your crate should, theoretically, not leak any types. Practically, some leakage
is unavoidable, sometimes even beneficial. We recommend you follow this heuristic:

- [ ] if you can avoid it, do not leak third-party types
- [ ] if you are part of an umbrella crate,<sup>2</sup> you may freely leak types from sibling crates.
- [ ] behind a relevant feature flag, types may be leaked (e.g., `serde`)
- [ ] without a feature _only_ if they give a _substantial benefit_. Most commonly that is interoperability with significant
      other parts of the Rust ecosystem based around these types.

<footnotes>

<sup>1</sup> In rare instances, e.g., high performance libraries used from embedded, you might even want to limit yourself to `core` only.

<sup>2</sup> For example, a `runtime` crate might be the umbrella of `runtime_rt`, `runtime_app` and `runtime_clock` As users are
expected to only interact with the umbrella, siblings may leak each others types.

</footnotes>

## Items come from their original crate (M-FOREIGN-REEXPORTS) { #M-FOREIGN-REEXPORTS }

<why>unambiguous type identity.</why>

Crates should generally not re-export items from other crates. For example, if your crate contains a method `foo::download(url: bar::Url)`, you should not do `pub use bar::Url` from inside `foo`. This avoids having possibly dozens of aliases in context, which can get confusing for both users and agents, in particular if these are mixed with genuinely different types of the same name from other crates.

When a crate accepts or returns a type defined in some third-party crate, users are expected to depend on that third-party crate directly and import the type from there. That said, there are a few valid exceptions to this rule:

- Umbrella crates (compare [M-DONT-LEAK-TYPES](#M-DONT-LEAK-TYPES)) by definition re-export other types
- Crates split for technical reasons (e.g., exporting `foo_core::Url` from `foo`)
- Macro use to provide stable paths, e.g., via some hidden `foo::__private::Url`

## Don't glob re-export items (M-NO-GLOB-REEXPORTS) { #M-NO-GLOB-REEXPORTS }

<why>a deliberate public surface.</why>

Don't `pub use foo::*` from other modules, especially not from other crates. You might accidentally export more than you want,
and globs are hard to review in PRs. Re-export items individually instead:

```rust,ignore
pub use foo::{A, B, C};
```

Glob exports are permissible for technical reasons, like doing platform specific re-exports from a set of HAL (hardware abstraction layer) modules:

```rust,ignore
#[cfg(target_os = "windows")]
mod windows { /* ... */ }

#[cfg(target_os = "linux")]
mod linux { /* ... */ }

// Acceptable use of glob re-exports, this is a common pattern
// and it is clear everything is just forwarded from a single 
// platform.

#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(target_os = "linux")]
pub use linux::*;
```

## Modules are balanced in size and scope (M-BALANCED-MODULES) { #M-BALANCED-MODULES }

<why>discoverable functionality and clear API usage.</why>

Your module design should approximately follow established UX practices of menu design: A _reasonable_ number of your most important items should be placed in the crate root, and a comprehensible grouping of the remaining functionality into subordinate modules.

Two violations of that rule are encountered most frequently: flat module roots containing dozens of items without clear ordering, or the excessive use of submodules without items in the crate root. While there are crates where this makes sense (e.g., automatically generated `-sys` crates defining 100s of C items, or umbrella crates like `std` and `tokio`), the majority of library crates are not among them.

When designing your module layout, consider these factors:

- Essential items users must find in order to use a crate should go into its root. For example, a `foo_client` crate should probably have its main `Client` struct inside the root.
- Other items should be grouped semantically by use case. Modules named `traits` and `errors` don't help anyone, but `account`, `network` and `status` do.
- Also take into account that modules are the perfect place for module-level documentation that further explains the respective subsystem.

## Don't define preludes (M-NO-PRELUDE) { #M-NO-PRELUDE }

<why>a clean namespace and reliable downstream builds.</why>

Crates must not define a `prelude` or any namespace intended to be imported as `use foo::*`.

While the Rust Standard Library successfully uses [preludes](https://doc.rust-lang.org/std/prelude/index.html) to define edition items, preludes in crates cause more harm than good. Given today's IDE support they are not needed, and once multiple preludes are used from different crates there is potential for conflicts:

```rust,ignore
use foo::prelude::*;
use bar::prelude::*;
use baz::prelude::*;

_ = Client::new();

// error[E0659]: `Client` is ambiguous
//   --> src/lib.rs:17:13
//    |
// 17 |     _ = Client; 
//    |         ^^^^^^ ambiguous name
//    |
//    = note: ambiguous because of multiple glob imports of a name in the same module
```

Preludes in particular do not resolve bad module design. If it looks like a prelude would make the crate easier to use or understand, this is almost always an indication that the existing module system needs restructuring, see [M-BALANCED-MODULES](#M-BALANCED-MODULES).

