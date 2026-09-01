# Argos

Deleted image recovery tool for HDD, SSD and NVMe. The medium is opened read-only and never written to; every image recovered is recorded with the extents it came from and the hash of what was written.

Carves JPEG and PNG from the raw surface, recovers from NTFS, ext2/3/4, FAT32, exFAT, APFS and
btrfs metadata, and reassembles photographs the medium stored in pieces

![Argos](docs/images/argos.png)

## Dependencies

**Every platform**

| Lang | Version |
| --- | --- |
| Rust | stable, 1.89 or newer |
| Node.js | 24 |

**Debian, Ubuntu, Mint**

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libappindicator3-dev librsvg2-dev libsoup-3.0-dev patchelf
```

**Fedora, RHEL**
```bash
sudo dnf install webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel libsoup3-devel patchelf
```

**Windows** — MSVC build tools. WebView2 ships with Windows 10 and 11.

**macOS** — Xcode Command Line Tools.

## Build from source

```bash
npm --prefix crates/argos_ui/ui ci

cargo build --release -p argos

triple=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p crates/argos_ui/binaries
cp target/release/argos "crates/argos_ui/binaries/argos-$triple"

(cd crates/argos_ui/ui && TAURI_APP_PATH="$PWD/.." TAURI_FRONTEND_PATH="$PWD" npx tauri build)

mkdir -p installers
find crates/argos_ui/target -path '*/release/bundle/*' -type f \
  \( -name '*.deb' -o -name '*.rpm' -o -name '*-setup.exe' -o -name '*.dmg' \) \
  -exec cp {} installers/ \;
```

The installers land in `installers/` at the root of the project.

That command builds for the machine it runs on and no other: a Tauri application
links the platform's own web view (WebKitGTK, WebView2, WKWebView) and the
shell asks for privileges in code that only its own target compiles. The release
workflow builds all four on their own runners and publishes nothing unless it
was started by a tag:

```bash
gh workflow run release.yml
```

To run the engine alone, without building the window: `cargo build --release -p argos` and use
`target/release/argos`.
