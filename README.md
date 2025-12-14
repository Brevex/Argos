# Argos - Image Recovery Tool

A powerful file recovery tool specialized in recovering deleted images from storage devices, even after formatting.

## Features

- **File Carving**: Recovers images by scanning for file signatures (magic bytes)
- **Multi-format Support**: JPEG, PNG, GIF, BMP, WebP, TIFF
- **Filesystem Parsing**: ext4, Btrfs, and NTFS (with metadata-based recovery)
- **PNG Conversion**: Automatically converts recovered images to PNG format
- **Progress Reporting**: Real-time progress with speed and ETA
- **Clean Architecture**: Modular design for easy extension

## Installation

```bash
# Clone and build in release mode
cargo build --release

# The binary will be at ./target/release/argos
```

## Usage

### Scan a device for recoverable files

```bash
# Scan a disk image
sudo ./target/release/argos scan --device /path/to/disk.img --output ./scan_results

# Scan a physical device (requires root)
sudo ./target/release/argos scan --device /dev/sda --output ./scan_results

# Scan for specific file types only
sudo ./target/release/argos scan --device /dev/sda -t jpeg,png
```

### Recover files

```bash
# Recover all images to a directory
sudo ./target/release/argos recover --device /path/to/disk.img --output ./recovered

# Recover without PNG conversion
sudo ./target/release/argos recover --device /dev/sda --output ./recovered --convert-png false
```

### List supported signatures

```bash
./target/release/argos list-signatures
```

### Show device information

```bash
sudo ./target/release/argos info --device /dev/sda
```

## Supported Filesystems

| Filesystem | Status | Notes |
|------------|--------|-------|
| ext4       | ✅ Superblock parsing | Inode table parsing planned |
| Btrfs      | ✅ Superblock parsing | B-tree recovery planned |
| NTFS       | ✅ Boot sector + MFT location | MFT parsing planned |
| Raw        | ✅ Full support | Pure file carving |

## Testing

Tests are organized in the `tests/` directory using [rstest](https://crates.io/crates/rstest):

```bash
# Run all tests
cargo test

# Run specific test file
cargo test --test domain_tests
cargo test --test infrastructure_tests
cargo test --test integration_tests
```

## Development

```bash
# Check compilation
cargo check

# Run with logging
RUST_LOG=debug cargo run -- scan --device /path/to/image.img

# Format code
cargo fmt

# Lint
cargo clippy
```
