# Argos - Image Recovery Tool

A powerful file recovery tool specialized in recovering deleted images from storage devices, even after formatting.

## Features

- **File Carving**: Recovers images by scanning for file signatures (magic bytes)
- **Multi-format Support**: JPEG, PNG, GIF, BMP, WebP, TIFF
- **Filesystem Parsing**: ext4 and Btrfs (Superblock parsing)
- **PNG Conversion**: Automatically converts recovered images to PNG format
- **Progress Reporting**: Real-time progress with speed and ETA
- **Vertical Slice Architecture**: Organized by domain (ext4, btrfs, recovery) for better cohesion

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
| Raw        | ✅ Full support | Pure file carving |

## Testing

Tests are organized to ensure reliability:

```bash
# Run all tests
cargo test
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
