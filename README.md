# Argos - Image Recovery Tool
**Argos** is a image recovery tool written in Rust. Specialized in recovering JPEG and PNG images from storage devices, even after multiple formats.

## Quick Start

### Interactive Mode (Recommended)
```bash
sudo ./target/release/argos --scan
```
This will open an interactive wizard that:
1. Discovers all available block devices
2. Lets you select the device to be analyzed
3. Prompts for the output directory
4. Confirms the operation before starting

### Command Line Mode
```bash
sudo ./target/release/argos --device /dev/sda --output ./recovered
```

## Installation

### Prerequisites
- Rust 1.70+ (`rustup install stable`)
- Linux
- Root/administrator access (required to read block devices)

### Build
```bash
git clone https://github.com/your-username/argos.git
```
```bash
cd argos
```
```bash
cargo build --release
```
The binary will be at `target/release/argos`

## Tests
```bash
# Run all tests
cargo test
```
```bash
# Run tests with verbose output
cargo test -- --nocapture
```

## Warnings
1. **Run as root**: Required to access block devices
2. **Read-only operation**: Argos NEVER modifies the source device
3. **Output space**: Make sure you have enough space for the recovered files