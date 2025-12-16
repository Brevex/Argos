use crate::recovery::carver::ScanProgress;
use crate::recovery::signatures::FileType;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "argos")]
#[command(author = "Breno")]
#[command(version = "0.2.0")]
#[command(about = "Recover deleted images from storage devices", long_about = None)]
pub struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Enable debug output
    #[arg(short, long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    Scan {
        #[arg(short, long)]
        device: String,

        #[arg(short, long, default_value = "./scan_results")]
        output: PathBuf,

        #[arg(short = 't', long, value_delimiter = ',')]
        types: Option<Vec<String>>,

        #[arg(short, long, default_value = "4")]
        chunk_size: usize,
    },

    Recover {
        #[arg(short, long)]
        device: String,

        #[arg(short, long, default_value = "./recovered")]
        output: PathBuf,

        #[arg(short = 't', long, value_delimiter = ',')]
        types: Option<Vec<String>>,

        #[arg(long, default_value = "true")]
        convert_png: bool,

        #[arg(long)]
        overwrite: bool,

        #[arg(long, default_value = "true")]
        organize: bool,
    },

    ListSignatures,

    Info {
        #[arg(short, long)]
        device: String,
    },
}

pub fn parse_file_types(types: Option<Vec<String>>) -> Vec<FileType> {
    match types {
        None => vec![],
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

pub struct ProgressReporter {
    bar: Arc<ProgressBar>,
}

impl ProgressReporter {
    pub fn new(total: u64, message: &str) -> Self {
        let bar = ProgressBar::new(total);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        bar.set_message(message.to_string());
        Self { bar: Arc::new(bar) }
    }

    pub fn for_scan(device_size: u64) -> Self {
        Self::new(device_size, "Scanning device for recoverable files...")
    }

    pub fn for_recovery(total_files: u64) -> Self {
        let bar = ProgressBar::new(total_files);
        bar.set_style(
             ProgressStyle::default_bar()
                .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} files ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        bar.set_message("Recovering files...".to_string());
        Self { bar: Arc::new(bar) }
    }

    pub fn finish(&self, message: &str) {
        self.bar.finish_with_message(message.to_string());
    }

    pub fn scan_callback(&self) -> Box<dyn Fn(&ScanProgress) + Send + Sync> {
        let bar = Arc::clone(&self.bar);
        Box::new(move |progress: &ScanProgress| {
            bar.set_position(progress.scanned_bytes);
            bar.set_message(format!(
                "Found {} potential files | Speed: {} MB/s",
                progress.matches_found,
                progress.speed_bps / (1024 * 1024)
            ));
        })
    }

    pub fn recovery_inc(&self) {
        self.bar.inc(1);
    }
}
