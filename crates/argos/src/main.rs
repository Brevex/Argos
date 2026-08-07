//! Argos CLI — forensic recovery of deleted images from block devices.
//!
//! Stdout is the user interface of this binary; argument parsing and subcommands
//! land with the carving MVP.

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() {
    println!("argos {}", env!("CARGO_PKG_VERSION"));
}
