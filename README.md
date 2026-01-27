# Argos

**High-performance forensic image recovery tool for Linux**

Argos scans raw block devices to recover deleted images (JPEG, PNG) using signature-based file carving. Built with Rust for maximum performance, featuring zero-copy I/O and parallel processing.

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
# Multi-pass mode (DEFAULT - better recovery rates)
sudo ./target/release/argos --scan

# Fast single-pass mode (lower recovery rates for fragmented files)
sudo ./target/release/argos --scan --fast
```

This launches an interactive wizard that:
1. Lists available block devices
2. Prompts for output directory
3. Shows operation summary and asks for confirmation
4. Displays real-time progress during scan

### Scanning Modes

#### Multi-Pass Mode (Default) - RECOMMENDED

```
Pass 1: Collect signatures
├─ Scan disk for all headers (JPEG SOI, PNG signature)
├─ Scan disk for all footers (using entropy boundaries, not naive EOI)
└─ Index: ~1GB disk/s on SSD

Pass 2: Global matching
├─ Build bipartite graph (headers × footers)
├─ Calculate edge weights (structural validation + heuristics)
├─ Solve optimal assignment (Hungarian algorithm)
└─ Validate: structural parsing of matched files

Pass 3: Orphan recovery
├─ Identify orphan headers (no matched footer)
├─ Attempt Bifragment Gap Carving
└─ Recover: ~40% of orphans via BGC
```

#### Single-Pass Mode (--fast) - USE WITH CAUTION

```
Stream: Scan once
├─ Collect header
├─ Immediately try to match footer
├─ Extract on-the-fly
└─ Speed: ~2x faster than multi-pass
```

### Command Line Options

| Option | Description |
|--------|-------------|
| `--scan`, `-s` | Start the recovery wizard |
| `--fast`, `-f` | Use fast single-pass mode (lower recovery rate) |
| `--unsafe-mode` | Bypass validation filters (for debugging) |
| `--debug` | Enable detailed skip reason logging |
| `--verbose`, `-v` | Verbose output |

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

🔬 Using multi-pass scan engine

[Pass 1] [========================================] 931.51 GB/931.51 GB
[Pass 1] Complete in 520.3s - JPEG: 15234 headers / 14891 footers, PNG: 892 headers / 876 footers

[Pass 2] [========================================] 15126/15126 candidates
[Pass 2] Complete in 45.2s - 15126 candidates processed, 1000 skipped

[Pass 3] [========================================] 343/343 orphans
[Pass 3] Complete in 12.1s - 127 recovered via BGC, 216 failed (of 343 orphans)

╔════════════════════════════════════════╗
║     === Multi-Pass Scan Complete ===   ║
╠════════════════════════════════════════╣
║ Total Time:                    577.6s  ║
║ JPEG Headers:                   15234  ║
║ JPEG Footers:                   14891  ║
║ PNG Headers:                      892  ║
║ PNG Footers:                      876  ║
║ Contiguous Files:               15126  ║
║ BGC Recovered:                    127  ║
║ Orphans Failed:                   216  ║
║ Files Recovered:                12847  ║
╚════════════════════════════════════════╝
```
