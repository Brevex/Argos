---
name: rust-unsafe-ffi
description: Use BEFORE writing or reviewing any `unsafe` block or fn, raw pointer, `transmute`, `libc`/ioctl/syscall call, `MaybeUninit`, `-sys` or `-ffi` crate, `extern "C"` item, `unsafe impl Send/Sync`, or a type wrapping a native OS handle (raw device file descriptors, `O_DIRECT` reads, disk geometry ioctls, `HANDLE`). Covers M-UNSAFE, M-UNSAFE-IMPLIES-UB, M-UNSOUND, M-FFI-NAMING, M-FFI-TRANSLATES, M-ISOLATE-DLL-STATE, M-ESCAPE-HATCHES.
---

# Unsafe, soundness and FFI

The most consequential skill in Argos: raw block-device access is the one place where the compiler
stops helping. Read [reference.md](reference.md) for the full rule text before writing the code —
the checklists there are normative, this file is the decision layer.

## The one absolute rule

`M-UNSOUND` is the single guideline in the whole book marked **"No Exceptions"**: safe-looking code
that can cause UB from safe callers is never acceptable, not even as a "remote, theoretical
possibility". If you cannot encapsulate something safely, expose an `unsafe fn` and document the
contract instead of hiding it.

## Before writing `unsafe`

Only three reasons are valid (`M-UNSAFE`):

1. novel abstractions (new smart pointer / allocator),
2. performance — **only after benchmarking** proved the safe version too slow,
3. FFI and platform calls (kernel, C libraries).

Never as an ad-hoc shortcut: not to "simplify" enum casts via `transmute`, not to bypass `Send`
bounds with `unsafe impl`, not to dodge a lifetime. If your justification is "it's faster to write",
stop.

Checklist for every `unsafe` you keep:

- [ ] Plain-text reasoning above the block explaining why it is sound (`M-UNSAFE`; clippy's
      `undocumented_unsafe_blocks` is on and will fail the build otherwise)
- [ ] `unsafe fn` bodies wrap their unsafe ops explicitly (`unsafe_op_in_unsafe_fn` is `warn`)
- [ ] `# Safety` doc section listing every condition the caller must uphold (see `rust-docs`)
- [ ] Passes Miri, including adversarial cases (`cargo +nightly miri test`)
- [ ] Minimal and testable — the unsafe core is small enough to audit in one sitting
- [ ] For closures/traits: assumes `Deref`, `Clone`, `Drop` may misbehave and that a user closure
      may panic (poison the abstraction if it does)

Soundness boundaries are **module boundaries** — inside one module a safe fn may rely on invariants
established by its siblings. Across modules it may not.

## `unsafe` is only for UB

`M-UNSAFE-IMPLIES-UB`: mark a fn `unsafe` if and only if misuse risks undefined behavior. A function
that is merely *dangerous* stays safe.

```rust,ignore
unsafe fn read_raw_sector(fd: RawFd, buf: *mut u8, len: usize) {}  // correct: raw pointer
unsafe fn wipe_device(dev: &Device) {}                             // wrong: destructive, not UB
```

For Argos this matters: a device-wiping or write-back path is *not* `unsafe` — guard it with a type
or an explicit confirmation flag, not with the keyword.

## FFI layering

- Naming (`M-FFI-NAMING`): `-sys` = we call into an existing C library; `-ffi` = others call into us.
- Separation (`M-FFI-TRANSLATES`): business logic lives in the core crate as idiomatic safe Rust;
  the FFI crate only translates. Never let `#[repr(C)]`, raw pointers or manual capacity fields leak
  back into the core types just to save a conversion.
- DLL state (`M-ISOLATE-DLL-STATE`): only "portable" `#[repr(C)]` data crosses a dynamic-library
  boundary — no `String`/`Vec`/`Box`, no `TypeId`, no types touching statics or thread-locals.

## Native handles get escape hatches

Any type wrapping an OS handle (`M-ESCAPE-HATCHES`) should offer:

```rust,ignore
impl DeviceHandle {
    pub fn open(path: &Path) -> Result<Self, DeviceError> { /* safe construction */ }
    pub unsafe fn from_native(fd: RawFd) -> Self { /* documents its safety contract */ }
    pub fn to_native(&self) -> RawFd { /* borrowed */ }
    pub fn into_native(self) -> RawFd { /* owned, caller must close */ }
}
```

## Argos application

- **Raw device I/O** (`open("/dev/sdX")`, `pread`, `O_DIRECT` alignment, `BLKGETSIZE64`/
  `BLKSSZGET` ioctls) is FFI: it belongs in a dedicated platform crate, wrapped in a safe API. The
  carvers and filesystem parsers must never contain `unsafe`.
- **`O_DIRECT` buffers** need alignment invariants — encode them in a type whose constructor
  guarantees alignment, so the `unsafe` read has one auditable precondition instead of many.
- **Zero-copy parsing** of on-disk structures (MFT records, superblocks, EXIF) is the classic place
  where a `transmute` to `#[repr(C)]` looks tempting. It is unsound on unaligned or attacker-shaped
  input: read fields explicitly with `u32::from_le_bytes`, or use a vetted crate. Disk images are
  **untrusted input**.
- **Read-only by construction:** forensic soundness argues for opening devices `O_RDONLY` at the
  lowest layer, so no write path exists to be reviewed.
- Platform differences (Linux/Windows/macOS) go behind a HAL module with per-target `mod`s, not
  `cfg!` scattered through logic.

## Full rule text

[reference.md](reference.md) — `M-UNSAFE`, `M-UNSAFE-IMPLIES-UB`, `M-UNSOUND`, `M-FFI-NAMING`,
`M-FFI-TRANSLATES`, `M-ISOLATE-DLL-STATE`, `M-ESCAPE-HATCHES`.
