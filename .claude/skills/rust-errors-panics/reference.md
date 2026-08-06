# Errors and panics — guideline reference

<!-- Rule text adapted from the Pragmatic Rust Guidelines, Copyright (c) Microsoft Corporation, MIT license. -->

**This file is the authoritative text of these rules for the Argos project.** It is
the source, not a copy: treat the wording here as normative and edit it directly when
the project's standard changes. Cite rules by id (e.g. `M-APP-ERROR`).

Rules in this file (7): `M-APP-ERROR`, `M-ERRORS-CANONICAL-STRUCTS`, `M-FROM-ERROR`, `M-PANIC-CONTINUATION`, `M-PANIC-IS-STOP`, `M-PANIC-MESSAGE`, `M-PANIC-ON-BUG`

Decision layer and Argos-specific application: [SKILL.md](SKILL.md). Full rule map: [guidelines index](../../guidelines-index.md).

---

## Applications may use Anyhow or derivatives (M-APP-ERROR) { #M-APP-ERROR }

<why>simple application-level error handling.</why>

> Note, this guideline is primarily a relaxation and clarification of [M-ERRORS-CANONICAL-STRUCTS].

Applications, and crates in your own repository exclusively used from your application, may use [ohno::AppError](https://docs.rs/crate/ohno/latest#structs), [anyhow](https://github.com/dtolnay/anyhow),
[eyre](https://github.com/eyre-rs/eyre) or similar application-level error crates instead of implementing their own types.

For example, in your application crates you may just re-export and use eyre's common `Result` type, which should be able to automatically
handle all third party library errors, in particular the ones following
[M-ERRORS-CANONICAL-STRUCTS].

```rust,ignore
use ohno::AppError;

fn start_application() -> Result<(), AppError> {
    start_server()?;
    Ok(())
}
```

Once you selected your application error crate you should switch all application-level errors to that type, and you should not mix multiple
application-level error types.

Libraries (crates used by more than one crate) should always follow [M-ERRORS-CANONICAL-STRUCTS] instead.

[M-ERRORS-CANONICAL-STRUCTS]: #M-ERRORS-CANONICAL-STRUCTS

## Panic continuation is last resort (M-PANIC-CONTINUATION) { #M-PANIC-CONTINUATION }

<why>state integrity and freedom from subtle bugs.</why>

Panic recovery via `catch_unwind()` is a matter of last resort and must generally be followed by a controlled application restart.

Panics indicate the program has reached an unrecoverable state (compare [M-PANIC-IS-STOP](#M-PANIC-IS-STOP) and [M-PANIC-ON-BUG](#M-PANIC-ON-BUG)). Library code in particular should not attempt to catch a panic and continue execution, as there is a risk of observing otherwise impossible state:

```rust,ignore
thread_local! {
    static ALWAYS_EQUAL: RefCell<(i32, i32)> = RefCell::new((0, 0));
}

fn main() {
    let _ = panic::catch_unwind(|| {
        ALWAYS_EQUAL.with_borrow_mut(|p| {
            p.0 += 1;        
            panic!("Assume some user-provided closure failed here");  
            p.1 += 1;
        });
    });

    ALWAYS_EQUAL.with_borrow(|p| {
        assert_eq!(p.0, p.1);  // Broken!
    });
}
```

Although the example above is slightly contrived, the side effects and interactions of a caught panic can be harder to identify, can have wide blast radius, and be subtle.

Systems where many unrelated tasks are in flight (e.g., server request handlers) can use `catch_unwind` on a per-request basis, but should still promote an application restart after a request handler caused a panic. The purpose of `catch_unwind` here is not to continue execution indefinitely, but to allow all other requests to gracefully finish.

## Panic means 'stop the program' (M-PANIC-IS-STOP) { #M-PANIC-IS-STOP }

<why>soundness and predictability.</why>

Panics are not exceptions. Instead, they suggest immediate program termination.

Although your code must be [_minimally_ panic-safe](https://doc.rust-lang.org/nomicon/exception-safety.html) (i.e., a survived panic may not lead to
undefined state), invoking a panic means _this program should stop now_. It is not valid to:

- use panics to communicate (errors) upstream,
- use panics to handle self-inflicted error conditions,
- assume panics will be caught, even by your own code.

For example, if the application calling you is compiled with a `Cargo.toml` containing

```toml
[profile.release]
panic = "abort"
```

then any invocation of panic will cause an otherwise functioning program to needlessly abort. Valid reasons to panic are:

- when encountering a programming error, e.g., `x.expect("must never happen")`,
- anything invoked from const contexts, e.g., `const { foo.unwrap() }`,
- when user requested, e.g., providing an `unwrap()` method yourself,
- when encountering a poison, e.g., by calling `unwrap()` on a lock result (a poisoned lock signals another thread has panicked already).

Any of those are directly or indirectly linked to programming errors.

## Custom panics have a helpful message (M-PANIC-MESSAGE) { #M-PANIC-MESSAGE }

<why>faster bug diagnosis.</why>

When code panics intentionally (via `panic!`, `assert!`, `unreachable!`, `todo!`, or similar), a message must be present to clearly state what went wrong and, where applicable, include relevant values.

```rust,ignore
// Bad, the panic gives the developer little to act on.
assert!(buffer.len() >= HEADER_SIZE);

// Good, message contains reason and actual values.
assert!(buffer.len() >= HEADER_SIZE, "buffer too small for header: got {} bytes, need {HEADER_SIZE}", buffer.len());
```

Messages related to API misuse should be useful to the end user. Messages indicating bugs should be helpful to you-as-the-author, or whoever maintains the project after you, to quickly identify the underlying cause.

Panic messages in tests are not generally needed.

## Detected programming bugs are panics, not errors (M-PANIC-ON-BUG) { #M-PANIC-ON-BUG }

<why>tractable error handling and runtime consistency.</why>

As an extension of [M-PANIC-IS-STOP] above, when an unrecoverable programming error has been
detected, libraries and applications must panic, i.e., request program termination.

In these cases, no `Error` type should be introduced or returned, as any such error could not be acted upon at runtime.

Contract violations, i.e., the breaking of invariants either within a library or by a caller, are programming errors and must therefore panic.

However, what constitutes a violation is situational. APIs are not expected to go out of their way to detect them, as such
checks can be impossible or expensive. Encountering `must_be_even == 3` during an already existing check clearly warrants
a panic, while a function `parse(&str)` clearly must return a `Result`. If in doubt, we recommend you take inspiration from the standard library.

```rust, ignore
// Generally, a function with bad parameters must either
// - Ignore a parameter and/or return the wrong result
// - Signal an issue via Result or similar
// - Panic
// If in this `divide_by` we see that y == 0, panicking is
// the correct approach.
fn divide_by(x: u32, y: u32) -> u32 { ... }

// However, it can also be permissible to omit such checks
// and return an unspecified (but not an undefined) result.
fn divide_by_fast(x: u32, y: u32) -> u32 { ... }

// Here, passing an invalid URI is not a contract violation.
// Since parsing is inherently fallible, a Result must be returned.
fn parse_uri(s: &str) -> Result<Uri, ParseError> { };

```

> ### <tip></tip> Make it 'Correct by Construction'
>
> While panicking on a detected programming error is the 'least bad option', your panic might still ruin someone's day.
> For any user input or calling sequence that would otherwise panic, you should also explore if you can use the type
> system to avoid panicking code paths altogether.

[M-PANIC-IS-STOP]: #M-PANIC-IS-STOP

## Errors are canonical structs (M-ERRORS-CANONICAL-STRUCTS) { #M-ERRORS-CANONICAL-STRUCTS }

<why>harmonized error types and consistent error handling.</why>

Errors should be a situation-specific `struct` that contain a [`Backtrace`](https://doc.rust-lang.org/stable/std/backtrace/struct.Backtrace.html),
a possible upstream error cause, and helper methods.

Simple crates usually expose a single error type `Error`, complex crates may expose multiple types, for example
`AccessError` and `ConfigurationError`. Error types should provide helper methods for additional information that allows callers to handle the error.

A simple error might look like so:

```rust
# use std::backtrace::Backtrace;
# use std::fmt::Display;
# use std::fmt::Formatter;
pub struct ConfigurationError {
    backtrace: Backtrace,
}

impl ConfigurationError {
    pub(crate) fn new() -> Self {
        Self { backtrace: Backtrace::capture() }
    }
}

// Impl Debug + Display
```

Where appropriate, error types should provide contextual error information, for example:

```rust,ignore
# use std::backtrace::Backtrace;
# #[derive(Debug)]
# pub struct ConfigurationError {
#    backtrace: Backtrace,
# }
impl ConfigurationError {
    pub fn config_file(&self) -> &Path { }
}
```

If your API does mixed operations, or depends on various upstream libraries, store an `ErrorKind`.
Error kinds, and more generally enum-based errors, should not be used to avoid creating separate public error types when there is otherwise no error overlap:

```rust, ignore
// Prefer this
fn download_iso() -> Result<(), DownloadError> {}
fn start_vm() -> Result<(), VmError> {}

// Over that
fn download_iso() -> Result<(), GlobalEverythingErrorEnum> {}
fn start_vm() -> Result<(), GlobalEverythingErrorEnum> {}

// However, not every function warrants a new error type. Errors
// should be general enough to be reused.
fn parse_json() -> Result<(), ParseError> {}
fn parse_toml() -> Result<(), ParseError> {}
```

If you do use an inner `ErrorKind`, that enum should not be exposed directly for future-proofing reasons,
as otherwise you would expose your callers to _all_ possible failure modes, even the ones you consider internal
and unhandleable. Instead, expose various `is_xxx()` methods as shown below:

```rust
# use std::backtrace::Backtrace;
# use std::fmt::Display;
# use std::fmt::Formatter;
#[derive(Debug)]
pub(crate) enum ErrorKind {
    Io(std::io::Error),
    Protocol
}

#[derive(Debug)]
pub struct HttpError {
    kind: ErrorKind,
    backtrace: Backtrace,
}

impl HttpError {
    pub fn is_io(&self) -> bool { matches!(self.kind, ErrorKind::Io(_)) }
    pub fn is_protocol(&self) -> bool { matches!(self.kind, ErrorKind::Protocol) }
}
```

Most upstream errors don't provide a backtrace. You should capture one when creating an `Error` instance, either via one of
your `Error::new()` flavors, or when implementing `From<UpstreamError> for Error {}`.

Error structs must properly implement `Display` that renders as follows:

```rust,ignore
impl Display for MyError {
    // Print a summary sentence what happened.
    // Print `self.backtrace`.
    // Print any additional upstream 'cause' information you might have.
#   fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
#       todo!()
#   }
}
```

Errors must also implement `std::error::Error`:

```rust,ignore
impl std::error::Error for MyError { }
```

Lastly, if you happen to emit lots of errors from your crate, consider creating a private `bail!()` helper macro to simplify error instantiation.

> ### <tip></tip> When You Get Backtraces
>
> Backtraces are an invaluable debug tool in complex or async code, since  errors might _travel_ far through a callstack before being surfaced.
>
> That said, they are a _development_ tool, not a _runtime_ diagnostic, and by default `Backtrace::capture()` will **not** capture
> backtraces, as they have a large overhead, e.g., 4μs per capture on the author's PC.
>
> Instead, Rust evaluates a [set of environment variables](https://doc.rust-lang.org/stable/std/backtrace/index.html#environment-variables), such as
> `RUST_BACKTRACE`, and only walks the call frame when explicitly asked. Otherwise it captures an empty trace, at the cost of only a few CPU instructions.

## Canonical error conversion uses `From`, not `map_err` (M-FROM-ERROR) { #M-FROM-ERROR }

<why>idiomatic error handling.</why>

Where an `Error` type is owned, it should `impl From<Other> for Error {}` instead of handling the conversion throughout the code via `.map_error()`. Calling `.map_error()` is only appropriate when dealing with foreign error types, or if contextual information needs to be preserved.

```rust,ignore
// Bad, repeats the same conversion at every call site and obscures the happy path.
fn load() -> Result<Config, MyError> {
    let bytes = read("config.toml").map_err(|e| MyError::Io(e))?;
    let text = str::from_utf8(&bytes).map_err(|e| MyError::Utf8(e))?;
    let cfg = toml::from_str(text).map_err(|e| MyError::Parse(e))?;
    Ok(cfg)
}

// Good, define the conversion once and let `?` apply it.
impl From<std::io::Error> for MyError { ... }
impl From<std::str::Utf8Error> for MyError { ... }
impl From<toml::de::Error> for MyError { ... }

fn load() -> Result<Config, MyError> {
    let bytes = read("config.toml")?;
    let text = str::from_utf8(&bytes)?;
    let cfg = toml::from_str(text)?;
    Ok(cfg)
}
```

