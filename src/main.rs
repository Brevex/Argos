//! Argos - Image Recovery Tool
//!
//! A powerful file recovery tool specialized in recovering deleted images
//! from storage devices, even after formatting.

use anyhow::{Context, Result};
use argos::application::dto::ScanOptions;
use argos::application::{RecoverFilesUseCase, ScanDeviceUseCase};
use argos::domain::repositories::{BlockDeviceReader, RecoveredFileWriter, WriteOptions};
use argos::domain::services::SignatureRegistry;
use argos::infrastructure::block_device::LinuxBlockDevice;
use argos::infrastructure::carvers::ImageCarver;
use argos::infrastructure::persistence::LocalFileWriter;
use argos::presentation::cli::{parse_file_types, Cli, Commands, ProgressReporter};
use argos::utils::format_bytes;
use clap::Parser;

fn main() -> Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.debug {
        "debug"
    } else if cli.verbose {
        "info"
    } else {
        "warn"
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    // Execute command
    match cli.command {
        Commands::Scan {
            device,
            output,
            types,
            chunk_size,
        } => {
            run_scan(&device, &output.to_string_lossy(), types, chunk_size)?;
        }

        Commands::Recover {
            device,
            output,
            types,
            convert_png,
            overwrite,
            organize,
        } => {
            run_recover(&device, &output, types, convert_png, overwrite, organize)?;
        }

        Commands::ListSignatures => {
            list_signatures();
        }

        Commands::Info { device } => {
            show_device_info(&device)?;
        }
    }

    Ok(())
}

/// Runs the scan command
fn run_scan(
    device_path: &str,
    output_path: &str,
    types: Option<Vec<String>>,
    chunk_size_mb: usize,
) -> Result<()> {
    println!("\n🔍 Argos Image Recovery Tool\n");
    println!("Scanning: {}", device_path);

    // Open device
    let device = LinuxBlockDevice::open(device_path)
        .context("Failed to open device. Make sure you have read permissions (try sudo).")?;

    let device_info = device.device_info()?;
    println!(
        "Device size: {} ({} bytes)",
        format_bytes(device_info.size),
        device_info.size
    );
    println!("Block size: {} bytes\n", device_info.block_size);

    // Create scan options
    let file_types = parse_file_types(types);
    let options = ScanOptions::new(device_path)
        .with_chunk_size(chunk_size_mb * 1024 * 1024)
        .with_types(file_types);

    // Create progress reporter
    let progress = ProgressReporter::for_scan(device_info.size);

    // Create and execute use case
    let scan_use_case = ScanDeviceUseCase::with_default_signatures();
    let result = scan_use_case.execute(&device, &options, Some(progress.scan_callback()))?;

    progress.finish("Scan complete!");

    // Display results
    println!("\n{}", result.summary());

    // Save results summary to file
    let summary_path = format!("{}/scan_summary.txt", output_path);
    std::fs::create_dir_all(output_path)?;
    std::fs::write(&summary_path, result.summary())?;
    println!("Results saved to: {}", summary_path);

    Ok(())
}

/// Runs the recover command
fn run_recover(
    device_path: &str,
    output_path: &std::path::Path,
    types: Option<Vec<String>>,
    convert_png: bool,
    overwrite: bool,
    organize: bool,
) -> Result<()> {
    println!("\n🔄 Argos Image Recovery Tool\n");
    println!("Device: {}", device_path);
    println!("Output: {}\n", output_path.display());

    // Open device
    let device = LinuxBlockDevice::open(device_path)
        .context("Failed to open device. Make sure you have read permissions (try sudo).")?;

    let device_info = device.device_info()?;
    println!(
        "Device size: {} ({} bytes)\n",
        format_bytes(device_info.size),
        device_info.size
    );

    // First, scan for files
    println!("Phase 1: Scanning for recoverable files...\n");

    let file_types = parse_file_types(types);
    let scan_options = ScanOptions::new(device_path).with_types(file_types);

    let progress = ProgressReporter::for_scan(device_info.size);
    let scan_use_case = ScanDeviceUseCase::with_default_signatures();
    let scan_result =
        scan_use_case.execute(&device, &scan_options, Some(progress.scan_callback()))?;

    progress.finish(&format!(
        "Found {} potential files",
        scan_result.total_matches()
    ));

    if scan_result.total_matches() == 0 {
        println!("\n⚠️  No recoverable files found.");
        return Ok(());
    }

    // Now recover files
    println!("\nPhase 2: Recovering files...\n");

    let write_options = WriteOptions {
        overwrite,
        convert_to_png: convert_png,
        organize_by_type: organize,
        filename_prefix: "recovered".to_string(),
    };

    let carver = ImageCarver::new();
    let writer = LocalFileWriter::new(output_path)?;

    let recovery_progress = ProgressReporter::for_recovery(scan_result.total_matches() as u64);
    let recover_use_case = RecoverFilesUseCase::new(carver, writer);
    let recovery_result = recover_use_case.execute(
        &device,
        &scan_result,
        &write_options,
        Some(recovery_progress.recovery_callback()),
    )?;

    recovery_progress.finish("Recovery complete!");

    // Display results
    println!("\n{}", recovery_result.summary());
    println!("Files saved to: {}", output_path.display());

    Ok(())
}

/// Lists all supported file signatures
fn list_signatures() {
    println!("\n📋 Supported File Signatures\n");

    let registry = SignatureRegistry::default_images();

    for file_type in registry.enabled_types() {
        let sigs = registry.get_signatures(*file_type);
        println!("  {} ({}):", file_type.name(), file_type.extension());

        for sig in sigs {
            let header_hex: Vec<String> =
                sig.header().iter().map(|b| format!("{:02X}", b)).collect();
            print!("    Header: {}", header_hex.join(" "));

            if let Some(footer) = sig.footer() {
                let footer_hex: Vec<String> = footer.iter().map(|b| format!("{:02X}", b)).collect();
                print!(" | Footer: {}", footer_hex.join(" "));
            }

            println!(" | Max: {}", format_bytes(sig.max_size()));
        }
        println!();
    }
}

/// Shows device information
fn show_device_info(device_path: &str) -> Result<()> {
    println!("\n📊 Device Information\n");

    let device = LinuxBlockDevice::open(device_path)
        .context("Failed to open device. Make sure you have read permissions (try sudo).")?;

    let info = device.device_info()?;

    println!("  Path:       {}", info.path);
    println!(
        "  Size:       {} ({} bytes)",
        format_bytes(info.size),
        info.size
    );
    println!("  Block Size: {} bytes", info.block_size);
    println!("  Blocks:     {}", info.block_count());
    println!(
        "  Read-Only:  {}",
        if info.read_only { "Yes" } else { "No" }
    );

    if let Some(model) = &info.model {
        println!("  Model:      {}", model);
    }
    if let Some(serial) = &info.serial {
        println!("  Serial:     {}", serial);
    }

    println!();

    Ok(())
}
