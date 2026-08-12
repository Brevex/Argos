# Argos

Forensic recovery of permanently deleted images from block devices, written in Rust. The medium is
opened read-only and never written to; every artifact is recorded with the extents it came from and
the hash of what was written.

## Install

The [releases page](../../releases) carries one installer per platform. Each release also lists
`SHA256SUMS.txt`; on a tool that reads evidence, checking the download against it is worth the ten
seconds.

| Platform | File | Install |
| --- | --- | --- |
| Debian, Ubuntu, Mint | `Argos_<version>_amd64.deb` | `sudo apt install ./Argos_<version>_amd64.deb` |
| Fedora, RHEL, openSUSE | `Argos-<version>-1.x86_64.rpm` | `sudo dnf install ./Argos-<version>-1.x86_64.rpm` |
| Windows 10/11 | `Argos_<version>_x64-setup.exe` | run it |
| macOS 11+ | `Argos_<version>_universal.dmg` | open it, drag Argos to Applications |

The macOS disk image holds one application for both Apple Silicon and Intel.

### The installers are not signed

There is no code-signing certificate behind this project yet, so both platforms that check for one
will object the first time:

- **Windows** shows "Windows protected your PC". Choose *More info* → *Run anyway*.
- **macOS** says the application cannot be opened. Right-click Argos in Applications, choose *Open*,
  and confirm. This is needed once.

### Privileges

Reading a raw disk needs administrator rights, so **Argos asks for them when it is opened, on every
platform, before the window appears** — UAC on Windows, the authentication panel on macOS, the
desktop's usual prompt on Linux. Declining means Argos does not start: there is no reduced mode,
because a scan that ran without privileges would report a disk it could not read as an empty one.

The scan therefore runs as the administrator, and what it recovers is handed back to the account
that asked, so the files are yours to open, move and delete. Where the destination cannot represent
ownership — an exFAT stick, a mounted Windows volume — Argos says so before the scan begins rather
than leaving you to find out at the end.

Everything the window does is also a subcommand of the `argos` binary the packages install, so a
scan can be driven from a terminal instead: `argos devices`, `sudo argos scan`, `argos report`,
`argos export`.

## Build from source

Needs a stable Rust toolchain and Node 24. On Linux also `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`librsvg2-dev` and `patchelf`.

```bash
# the engine, from the repository root
cargo test --workspace
cargo build --release -p argos --target x86_64-unknown-linux-gnu

# stage it where the bundler looks for it
mkdir -p crates/argos_ui/binaries
cp target/x86_64-unknown-linux-gnu/release/argos \
   crates/argos_ui/binaries/argos-x86_64-unknown-linux-gnu

# the shell
npm --prefix crates/argos_ui/ui ci
cd crates/argos_ui
TAURI_FRONTEND_PATH=ui ui/node_modules/.bin/tauri build \
  --target x86_64-unknown-linux-gnu --bundles deb
```

The engine is a separate binary that the shell finds beside itself, which is why it is staged under
`binaries/` before anything else. That step is not optional even for a plain `cargo check` of the
shell: the binary is declared as an external one, and Tauri's build script fails when it is
missing. `TAURI_FRONTEND_PATH` is given explicitly because the frontend sits beside the Tauri
configuration rather than above it.

## Contributing

The standard this codebase is written to, and the reviewers that enforce it, are described in
[CLAUDE.md](CLAUDE.md). The architecture and build order are in
[docs/DEVELOPMENT-PLAN.md](docs/DEVELOPMENT-PLAN.md).
