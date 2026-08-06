# Concurrency, async and shared state — guideline reference

<!-- Rule text adapted from the Pragmatic Rust Guidelines, Copyright (c) Microsoft Corporation, MIT license. -->

**This file is the authoritative text of these rules for the Argos project.** It is
the source, not a copy: treat the wording here as normative and edit it directly when
the project's standard changes. Cite rules by id (e.g. `M-ASYNC-FN`).

Rules in this file (7): `M-ASYNC-FN`, `M-ASYNC-STACK-SIZE`, `M-AVOID-STATICS`, `M-SERVICES-CLONE`, `M-THROUGHPUT`, `M-TYPES-SEND`, `M-YIELD-POINTS`

Decision layer and Argos-specific application: [SKILL.md](SKILL.md). Full rule map: [guidelines index](../../guidelines-index.md).

---

## Hot `async` functions reduce stack size (M-ASYNC-STACK-SIZE) { #M-ASYNC-STACK-SIZE }

<why>small async stack sizes and low memcpy overhead.</why>

Functions marked `async` in the hot path should track their future sizes, and take one or more of the following steps to reduce their impact:

- reduction of parameter and rval type size,
- reduction of type size of items held across `.await` points,
- returning `impl Future` and extracting setup logic from `async {}` capture.

> ### <tip></tip> Future 'Stack' Sizes
>
> In Futures, what would naively be considered _their stack_, is actually part of a significantly more complicated machinery under their  hood.
>
> Regular locals, that only live momentarily between two `.await` points, still remain part of the runtime thread's regular stack. However, any locals that live across `.await` points, or parameters passed during construction, become part of that Future's state machine type, and the layout of this type is currently not as optimized as it could be.
>
> This not only can cause stack-to-heap memcpy operations when creating or boxing Futures, it can also force large upfront stack sizes of the hypothetical most deeply nested cross-async call stack of the involved async function (which, on a side note, is why they can't simply recurse).
>
> ```rust,ignore
> async fn foo(_large: Large) {
>     let within_future = [0_u8; 1024]; // Crosses .await below, embedded in `foo` type
>     let on_stack = [0_u8; 1024]; // Does not cross .await points, lives on stack
>     let sneaky = Droppable::with_size(1024); // Secretly crosses .await point!
>     dbg!(&on_stack, &sneaky);
>     bar(&within_future).await;
>     dbg!(&within_future);
>     // <- `sneaky` dropped here, despite otherwise not being used!
> }
> 
> let future = foo(Large::new()); // `Large` becomes embedded in `foo` type, 
>                                 // blowing up its size, despite it not even
>                                 // being used.
> 
> // Here, despite `foo` not running yet, we might consume up to `Large` + 
> // 2kb of this thread's stack memory. Once we spawn this is memcpy'ed 
> // to runtime Task structure:
> rt.spawn(future);
>```

For many async functions this isn't an issue, as their associated `Future`-cost is negligible. However, functions used along the hot path, that are either called or instantiated frequently (e.g., 1000's of calls per second or concurrent tasks) might benefit from monitoring and optimizations.

Hot futures should be tracked via `size_of_val`:

```rust,ignore
async fn hot() { ... }

#[test]
fn has_reasonable_size() {
    let f = hot();
    assert!(size_of_val(&f) < ...); // Determine value / limit at first run.
}
```

Then consider a combination of the following:

```rust,ignore
// 1) Return an `impl Future` instead, this prevents large arguments 
//    from infecting the future size, among others.
fn hot(args: Args) -> impl Future<Output = Result<T>> { 
    // 2) Process arguments outside async context if processing does
    //    not require async functionality.
    let args = args.do_something(); 

    if args.invalid() {
        // 3) Use `Either` to return a single `impl Future` type, as
        //    otherwise you'd have to invent a new type. 
        async { Err(InvalidArgs) }.left_future() 
    } else {
        // 4) Chain future invocations via future helpers, which again 
        //    prevents heavy locals from being passed through the state 
        //    machine.
        read(args).then(|x| foo(x)).right_future() 
    }
}
```

## Optimize for throughput, avoid empty cycles (M-THROUGHPUT) { #M-THROUGHPUT }

<why>COGS savings at scale.</why>

You should optimize your library for throughput, and one of your key metrics should be _items per CPU cycle_.

This does not mean to neglect latency&mdash;after all you can scale for throughput, but not for latency. However,
in most cases you should not pay for latency with _empty cycles_ that come with single-item processing, contended locks and frequent task switching.

Ideally, you should

- partition reasonable chunks of work ahead of time,
- let individual threads and tasks deal with their slice of work independently,
- sleep or yield when no work is present,
- design your own APIs for batched operations,
- perform work via batched APIs where available,
- yield within long individual items, or between chunks of batches (see [M-YIELD-POINTS]),
- exploit CPU caches, temporal and spatial locality.

You should not:

- hot spin to receive individual items faster,
- perform work on individual items if batching is possible,
- do work stealing or similar to balance individual items.

Shared state should only be used if the cost of sharing is less than the cost of re-computation.

[M-YIELD-POINTS]: #M-YIELD-POINTS

## Long-running tasks should have yield points (M-YIELD-POINTS) { #M-YIELD-POINTS }

<why>fair CPU time for all tasks.</why>

If you perform long running computations, they should contain `yield_now().await` points.

Your future might be executed in a runtime that cannot work around blocking or long-running tasks. Even then, such tasks are
considered bad design and cause runtime overhead. If your complex task performs I/O regularly it will simply utilize these await points to preempt itself:

```rust, ignore
async fn process_items(items: &[items]) {
    // Keep processing items, the runtime will preempt you automatically.
    for i in items {
        read_item(i).await;
    }
}
```

If your task performs long-running CPU operations without intermixed I/O, it should instead cooperatively yield at regular intervals, to not starve concurrent operations:

```rust, ignore
async fn process_items(zip_file: File) {
    let items = zip_file.read().async;
    for i in items {
        decompress(i);
        yield_now().await;
    }
}
```

If the number and duration of your individual operations are unpredictable you should use APIs such as `has_budget_remaining()` and
related APIs to query your hosting runtime.

> ### <tip></tip> Yield how often?
>
> In a thread-per-core model the overhead of task switching must be balanced against the systemic effects of starving unrelated tasks.
>
> Under the assumption that runtime task switching takes 100's of ns, in addition to the overhead of lost CPU caches,
> continuous execution in between should be long enough that the switching cost becomes negligible (<1%).
>
> Thus, performing 10 - 100μs of CPU-bound work between yield points would be a good starting point.

## Types are Send (M-TYPES-SEND) { #M-TYPES-SEND }

<why>use in Tokio and behind runtime abstractions.</why>

Public types should be `Send` for compatibility reasons:

- All futures produced (explicitly or implicitly) must be `Send`
- Most other types should be `Send`, but there might be exceptions

### Futures

When declaring a future explicitly you should ensure it is, and remains, `Send`.

```rust
# use std::future::Future;
# use std::pin::Pin;
# use std::task::{Context, Poll};
#
struct Foo {}

impl Future for Foo {
    // Explicit implementation of `Future` for your type
    # type Output = ();
    #
    # fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<<Self as Future>::Output> { todo!() }
}

// You should assert your type is `Send`
const fn assert_send<T: Send>() {}
const _: () = assert_send::<Foo>();
```

When returning futures implicitly through `async` method calls, you should make sure these are `Send` too.
You do not have to test every single method, but you should at least validate your main entry points.

```rust,edition2021
async fn foo() { }

// TODO: We want this as a macro as well
fn assert_send<T: Send>(_: T) {}
_ = assert_send(foo());
```

### Regular Types

Most regular types should be `Send`, as they otherwise infect futures turning them `!Send` if held across `.await` points.

```rust,edition2021
# use std::rc::Rc;
# async fn read_file(x: &str) {}
#
async fn foo() {
    let rc = Rc::new(123);      // <-- Holding this across an .await point prevents
    read_file("foo.txt").await; //     the future from being `Send`.
    dbg!(rc);
}
```

That said, if the default use of your type is _instantaneous_, and there is no reason for it to be otherwise held across `.await` boundaries, it may be `!Send`.

```rust,edition2021
# use std::rc::Rc;
# struct Telemetry; impl Telemetry { fn ping(&self, _: u32) {} }
# fn telemetry() -> Telemetry  { Telemetry }
# async fn read_file(x: &str) {}
#
async fn foo() {
    // Here a hypothetical instance Telemetry is summoned
    // and used ad-hoc. It may be ok for Telemetry to be !Send.
    telemetry().ping(0);
    read_file("foo.txt").await;
    telemetry().ping(1);
}
```

> ### <tip></tip> The Cost of Send
>
> Ideally, there would be abstractions that are `Send` in work-stealing runtimes, and `!Send` in thread-per-core models based on non-atomic
> types like `Rc` and `RefCell` instead.
>
> Practically these abstractions don't exist, preventing Tokio compatibility in the non-atomic case. That in turn means you would have to
> "reinvent the world" to get anything done in a thread-per-core universe.
>
> The good news is, in most cases atomics and uncontended locks only have a measurable impact if accessed more frequently than every 64 words or so.
>
> <div style="background-color:white;">
>
> *(illustrative screenshot from the original book, not reproduced here)*
>
> </div>
>
> Working with a large `Vec<AtomicUsize>` in a hot loop is a bad idea, but doing the occasional uncontended atomic operation from otherwise thread-per-core
> async code has no performance impact, but gives you widespread ecosystem compatibility.

## Avoid statics (M-AVOID-STATICS) { #M-AVOID-STATICS }

<why>consistency and correctness across crate versions.</why>

Libraries should avoid `static` and thread-local items, if a consistent view of the item is relevant for correctness.
Essentially, any code that would be incorrect if the static _magically_ had another value must not use them. Statics
only used for performance optimizations are ok.

The fundamental issue with statics in Rust is the secret duplication of state.

Consider a crate `core` with the following function:

```rust
# use std::sync::atomic::AtomicUsize;
# use std::sync::atomic::Ordering;
static GLOBAL_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn increase_counter() -> usize {
    GLOBAL_COUNTER.fetch_add(1, Ordering::Relaxed)
}
```

Now assume you have a crate `main`, calling two libraries `library_a` and `library_b`, each invoking that counter:

```rust,ignore
// Increase global static counter 2 times
library_a::count_up();
library_a::count_up();

// Increase global static counter 3 more times
library_b::count_up();
library_b::count_up();
library_b::count_up();
```

They eventually report their result:

```rust,ignore
library_a::print_counter();
library_b::print_counter();
main::print_counter();
```

At this point, what is _the_ value of said counter; `0`, `2`, `3` or `5`?

The answer is, possibly any  (even multiple!) of the above, depending on the crate's version resolution!

Under the hood Rust may link to multiple versions of the same crate, independently instantiated, to satisfy declared
dependencies. This is especially observable during a crate's `0.x` version timeline, where each `x` constitutes a separate _major_ version.

If `main`,  `library_a` and `library_b` all declared the same version of `core`, e.g. `0.5`, then the reported result will be `5`, since all
crates actually _see_ the same version of `GLOBAL_COUNTER`.

However, if `library_a` declared `0.4` instead, then it would be linked against a separate version of `core`; thus `main` and `library_b` would
agree on a value of `3`, while `library_a` reported `2`.

Although `static` items can be useful, they are particularly dangerous before a library's stabilization, and for any state where _secret duplication_ would
cause consistency issues when static and non-static variable use interacts. In addition, statics interfere with unit testing, and are a contention point in
thread-per-core designs.

## Functions are `async` over returning a Future (M-ASYNC-FN) { #M-ASYNC-FN }

<why>simpler code and easier-to-understand APIs.</why>

Functions should be declared `async fn foo()` over `fn foo() -> impl Future` when both are viable.

Functions marked `async` are more idiomatic and easier to read. An explicit `Future`-returning signature should only be used when required, for example inside traits or for _hot 'n heavy_ async functions, compare [M-ASYNC-STACK-SIZE](#M-ASYNC-STACK-SIZE).

```rust,ignore
impl Foo {
    // Bad, signature is noisier and the body needs an extra `async` block
    fn foo() -> impl Future<Output = Result<T, E>> { async { Ok(t) } }

    // Good, method and implementation reads normally
    async fn foo() -> Result<T, E> { Ok(t) }
}
```

## Services are Clone (M-SERVICES-CLONE) { #M-SERVICES-CLONE }

<why>composable sharing of common services.</why>

Heavyweight _service_ types and 'thread singletons' should implement shared-ownership `Clone` semantics, including any type you expect to be used from your `Application::init`.

Per thread, users should essentially be able to create a single resource handler instance, and have it reused by other handlers on the same thread:

```rust,ignore
impl ThreadLocal for MyThreadState {
    fn init(...) -> Self {

        // Create common service instance possibly used by many.
        let common = ServiceCommon::new();

        // Users can freely pass `common` here multiple times
        let service_1 = ServiceA::new(&common);
        let service_2 = ServiceA::new(&common);

        Self { ... }
    }
}
```

Services then simply clone their dependency and store a new _handle_, as if `ServiceCommon` were a shared-ownership smart pointer:

```rust,ignore
impl ServiceA {
    pub fn new(common: &ServiceCommon) -> Self {
        // If we only need to access `common` from `new` we don't have
        // to store it. Otherwise, make a clone we store in `Self`.
        let common = common.clone();
    }
}
```

Under the hood this `Clone` should **not** create a fat copy of the entire service. Instead, it should follow the `Arc<Inner>` pattern:

```rust, ignore
// Actual service containing core logic and data.
struct ServiceCommonInner {}

#[derive(Clone)]
pub ServiceCommon {
    inner: Arc<ServiceCommonInner>
}

impl ServiceCommon {
    pub fn new() {
        Self { inner: Arc::new(ServiceCommonInner::new()) }
    }

    // Method forwards ...
    pub fn foo(&self) { self.inner.foo() }
    pub fn bar(&self) { self.inner.bar() }
}
```

