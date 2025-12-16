use anyhow::{Context, Result};
use argos::cli::{parse_file_types, Cli, Commands, ProgressReporter};
use argos::core::device::LinuxBlockDevice;
use argos::core::io::BlockDeviceReader;
use argos::recovery::carver::{Carver, RecoveredFile, ScanOptions};
use argos::recovery::signatures::{FileType, SignatureRegistry};
use clap::Parser;
use image::ImageFormat;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = if cli.debug {
        "debug"
    } else if cli.verbose {
        "info"
    } else {
        "warn"
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    match cli.command {
        Commands::Scan {
            device,
            output,
            types,
            chunk_size,
        } => {
            run_scan(&device, &output, types, chunk_size)?;
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

fn run_scan(
    device_path: &str,
    output_path: &Path,
    types: Option<Vec<String>>,
    chunk_size_mb: usize,
) -> Result<()> {
    println!("\n🔍 Argos Image Recovery Tool\nScanning: {}", device_path);

    let device = LinuxBlockDevice::open(device_path).context("Failed to open device")?;
    let info = device.device_info()?;
    println!("Device size: {} bytes", info.size);

    let options = ScanOptions {
        chunk_size: chunk_size_mb * 1024 * 1024,
        file_types: parse_file_types(types),
    };

    let progress = ProgressReporter::for_scan(info.size);
    let carver = Carver::new();
    let result = carver.scan(&device, &options, Some(progress.scan_callback()))?;
    progress.finish("Scan complete!");

    println!("\n{}", result.summary());

    fs::create_dir_all(output_path)?;
    let summary_path = output_path.join("scan_summary.txt");
    fs::write(&summary_path, result.summary())?;
    println!("Results saved to: {}", summary_path.display());
    Ok(())
}

fn run_recover(
    device_path: &str,
    output_path: &Path,
    types: Option<Vec<String>>,
    convert_png: bool,
    overwrite: bool,
    organize: bool,
) -> Result<()> {
    println!(
        "\n🔄 Argos Image Recovery Tool\nDevice: {}\nOutput: {}",
        device_path,
        output_path.display()
    );

    let device = LinuxBlockDevice::open(device_path).context("Failed to open device")?;
    let info = device.device_info()?;
    println!("Device size: {} bytes\nPhase 1: Scanning...", info.size);

    let options = ScanOptions {
        chunk_size: 4 * 1024 * 1024,
        file_types: parse_file_types(types),
    };

    let carver = Carver::new();
    let progress = ProgressReporter::for_scan(info.size);
    let scan_result = carver.scan(&device, &options, Some(progress.scan_callback()))?;

    progress.finish(&format!(
        "Found {} potential files",
        scan_result.total_matches()
    ));

    if scan_result.total_matches() == 0 {
        println!("\n⚠️ No recoverable files found.");
        return Ok(());
    }

    println!("\nPhase 2: Recovering files...\n");
    let recovery_progress = ProgressReporter::for_recovery(scan_result.total_matches() as u64);

    let _chunk_size = 1024 * 1024;

    let mut files_recovered = 0;

    let mut matches = scan_result.matches.clone();
    matches.sort_by_key(|m| m.start_offset());

    for (id, m) in matches.iter().enumerate() {
        let max_size = m.estimated_size() as usize;
        let read_size = max_size.min(200 * 1024 * 1024);

        match device.read_at(m.start_offset(), read_size) {
            Ok(data) => {
                if let Ok(file) = carver.recover_file(&data, m, id as u64) {
                    save_recovered_file(&file, output_path, convert_png, overwrite, organize)?;
                    files_recovered += 1;
                }
            }
            Err(e) => eprintln!("Error reading at {}: {}", m.start_offset(), e),
        }
        recovery_progress.recovery_inc();
    }

    recovery_progress.finish("Recovery complete!");
    println!(
        "Recovered {} files to {}",
        files_recovered,
        output_path.display()
    );
    Ok(())
}

fn save_recovered_file(
    file: &RecoveredFile,
    output_dir: &Path,
    convert_png: bool,
    overwrite: bool,
    organize: bool,
) -> Result<()> {
    let mut path = output_dir.to_path_buf();
    if organize {
        path.push(file.file_type.extension());
    }
    fs::create_dir_all(&path)?;

    let should_convert =
        convert_png && file.file_type != FileType::Png && file.file_type != FileType::Unknown;
    let extension = if should_convert {
        "png"
    } else {
        file.file_type.extension()
    };
    let filename = format!("recovered_{:06}.{}", file.id, extension);
    path.push(filename);

    if path.exists() && !overwrite {
        return Ok(());
    }

    let data_to_write = if should_convert {
        match convert_to_png(&file.data, file.file_type) {
            Ok(d) => d,
            Err(_) => file.data.clone(), // Fallback
        }
    } else {
        file.data.clone()
    };

    File::create(path)?.write_all(&data_to_write)?;
    Ok(())
}

fn convert_to_png(data: &[u8], file_type: FileType) -> Result<Vec<u8>> {
    let format = match file_type {
        FileType::Jpeg => ImageFormat::Jpeg,
        FileType::Gif => ImageFormat::Gif,
        FileType::Bmp => ImageFormat::Bmp,
        FileType::WebP => ImageFormat::WebP,
        FileType::Tiff => ImageFormat::Tiff,
        _ => return Ok(data.to_vec()),
    };

    let img = image::load_from_memory_with_format(data, format)?;
    let mut png_data = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png_data), ImageFormat::Png)?;
    Ok(png_data)
}

fn list_signatures() {
    println!("\n📋 Supported File Signatures\n");
    let registry = SignatureRegistry::default_images();
    for file_type in registry.enabled_types() {
        println!("  {} ({})", file_type.name(), file_type.extension());
        for sig in registry.get_signatures(*file_type) {
            let header_hex: Vec<String> =
                sig.header().iter().map(|b| format!("{:02X}", b)).collect();
            println!("    Header: {}", header_hex.join(" "));
        }
        println!();
    }
}

fn show_device_info(device_path: &str) -> Result<()> {
    println!("\n📊 Device Information\n");
    let device = LinuxBlockDevice::open(device_path).context("Failed to open device")?;
    let info = device.device_info()?;
    println!("  Path:       {}", info.path);
    println!("  Size:       {} bytes", info.size);
    println!("  Block Size: {} bytes", info.block_size);
    Ok(())
}
