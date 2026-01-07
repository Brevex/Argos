# Argos

**High-performance forensic image recovery tool for Linux**

Argos scans raw block devices to recover deleted images (JPEG, PNG) using signature-based file carving. Built with Rust for maximum performance, featuring zero-copy I/O and parallel processing.

## Features

- **Signature scanning** — Detects file headers/footers using SIMD-accelerated pattern matching
- **High throughput** — Achieves 150-200+ MB/s on NVMe drives
- **Parallel extraction** — Multi-threaded file recovery
- **Progress tracking** — Real-time progress bar with ETA
- **Safe** — Read-only access to source device

## Requirements

- **OS**: Linux 
- **Rust**: 1.75+ 
- **Permissions**: Root access required for raw device access

## Installation

### From source

```bash
# Clone the repository
git clone https://github.com/Brevex/Argos.git
cd argos

# Build in release mode (required for performance)
cargo build --release

# The binary will be at ./target/release/argos
```

## Usage

### Interactive Mode (Recommended)

```bash
sudo ./target/release/argos --scan
```

This launches an interactive wizard that:
1. Lists available block devices
2. Prompts for output directory
3. Shows operation summary and asks for confirmation
4. Displays real-time progress during scan

### Example Session

```
🔮 Argos - Image Recovery Wizard

🔍 Discovering block devices...

📀 Found Devices:

NAME         TYPE                    SIZE PATH
-------------------------------------------------------
sda          HDD               931.51 GB /dev/sda
nvme0n1      NVMe              476.94 GB /dev/nvme0n1

✔ Select device for analysis · /dev/sda (HDD) - 931.51 GB

✔ Where do you want to save the recovered files? · ./recovered

📋 Operation Summary:
   • Target:  /dev/sda (931.51 GB)
   • Output:  ./recovered
   • Modes:   JPEG, PNG

✔ Confirm and start scan? · yes

[00:05:32] [##########--------------------] 312 GiB/931 GiB (10m)
```
