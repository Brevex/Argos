//! CLI commands using clap

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Argos - Image Recovery Tool
///
/// A powerful file recovery tool specialized in recovering deleted images
/// from storage devices, even after formatting.
#[derive(Parser)]
#[command(name = "argos")]
#[command(author = "Breno")]
#[command(version = "0.1.0")]
#[command(about = "Recover deleted images from storage devices", long_about = None)]
pub struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Enable debug output
    #[arg(short, long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Scan a device for recoverable files
    Scan {
        /// Path to device or image file (e.g., /dev/sda, disk.img)
        #[arg(short, long)]
        device: String,

        /// Output directory for scan results
        #[arg(short, long, default_value = "./scan_results")]
        output: PathBuf,

        /// File types to scan for (jpeg, png, gif, bmp, webp, tiff)
        #[arg(short = 't', long, value_delimiter = ',')]
        types: Option<Vec<String>>,

        /// Chunk size in MB for reading
        #[arg(short, long, default_value = "4")]
        chunk_size: usize,
    },

    /// Recover files from a device using scan results
    Recover {
        /// Path to device or image file
        #[arg(short, long)]
        device: String,

        /// Output directory for recovered files
        #[arg(short, long, default_value = "./recovered")]
        output: PathBuf,

        /// File types to recover (jpeg, png, gif, bmp, webp, tiff)
        #[arg(short = 't', long, value_delimiter = ',')]
        types: Option<Vec<String>>,

        /// Convert all images to PNG format
        #[arg(long, default_value = "true")]
        convert_png: bool,

        /// Overwrite existing files
        #[arg(long)]
        overwrite: bool,

        /// Organize files by type in subdirectories
        #[arg(long, default_value = "true")]
        organize: bool,
    },

    /// List supported file signatures
    ListSignatures,

    /// Show device information
    Info {
        /// Path to device or image file
        #[arg(short, long)]
        device: String,
    },
}

/// Parses file type strings to FileType enum
pub fn parse_file_types(types: Option<Vec<String>>) -> Vec<crate::domain::entities::FileType> {
    use crate::domain::entities::FileType;

    match types {
        None => vec![], // All types
        Some(type_strs) => type_strs
            .iter()
            .filter_map(|s| match s.to_lowercase().as_str() {
                "jpeg" | "jpg" => Some(FileType::Jpeg),
                "png" => Some(FileType::Png),
                "gif" => Some(FileType::Gif),
                "bmp" => Some(FileType::Bmp),
                "webp" => Some(FileType::WebP),
                "tiff" | "tif" => Some(FileType::Tiff),
                _ => {
                    eprintln!("Warning: Unknown file type '{}'", s);
                    None
                }
            })
            .collect(),
    }
}
