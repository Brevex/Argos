use anyhow::{Context, Result};
use argos::cli::{parse_file_types, Cli, Commands, ProgressReporter};
use argos::core::device::LinuxBlockDevice;
use argos::core::io::BlockDeviceReader;
use argos::recovery::carver::{Carver, RecoveredFile, ScanOptions};
use argos::recovery::signatures::{FileType, SignatureRegistry};
use argos::recovery::validator::{ImageValidator, ValidationConfig, ValidationStats};
use clap::Parser;
use image::ImageFormat;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use inquire::{Confirm, InquireError, Select, Text};

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
        Some(Commands::Scan {
            device,
            output,
            types,
            chunk_size,
        }) => {
            run_scan(&device, &output, types, chunk_size)?;
        }
        Some(Commands::Recover {
            device,
            output,
            types,
            convert_png,
            overwrite,
            organize,
            min_size,
            min_width,
            min_height,
            no_validation,
        }) => {
            run_recover(
                &device,
                &output,
                types,
                convert_png,
                overwrite,
                organize,
                min_size,
                min_width,
                min_height,
                no_validation,
            )?;
        }
        Some(Commands::ListSignatures) => {
            list_signatures();
        }
        Some(Commands::Info { device }) => {
            show_device_info(&device)?;
        }
        None => match run_wizard() {
            Ok(_) => {}
            Err(e) => {
                if let Some(inquire_err) = e.downcast_ref::<InquireError>() {
                    match inquire_err {
                        InquireError::OperationCanceled => {
                            println!("👋 Operation cancelled.");
                        }
                        _ => eprintln!("❌ Error: {}", e),
                    }
                } else {
                    eprintln!("❌ Error: {}", e);
                }
            }
        },
    }
    Ok(())
}

struct PhysicalDisk {
    name: String,
    model: String,
    size: u64,
    path: String,
}

fn get_physical_disks() -> Result<Vec<PhysicalDisk>> {
    let mut disks = Vec::new();
    let sys_block = Path::new("/sys/class/block");

    if sys_block.exists() {
        for entry in fs::read_dir(sys_block)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if file_name.starts_with("loop")
                || file_name.starts_with("ram")
                || file_name.starts_with("zram")
            {
                continue;
            }

            if path.join("partition").exists() {
                continue;
            }

            let size_path = path.join("size");
            if !size_path.exists() {
                continue;
            }
            let sectors_str = fs::read_to_string(&size_path)?.trim().to_string();
            let sectors: u64 = sectors_str.parse().unwrap_or(0);
            if sectors == 0 {
                continue;
            }

            let size_bytes = sectors * 512;

            let model_path = path.join("device/model");
            let model = if model_path.exists() {
                fs::read_to_string(model_path)?.trim().to_string()
            } else {
                "Unknown Model".to_string()
            };

            disks.push(PhysicalDisk {
                name: file_name.clone(),
                model,
                size: size_bytes,
                path: format!("/dev/{}", file_name),
            });
        }
    }

    disks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(disks)
}

fn run_wizard() -> Result<()> {
    println!("🧙 Argos Recovery Wizard");

    let disks = get_physical_disks()?;
    let mut options: Vec<String> = disks
        .iter()
        .map(|d| {
            format!(
                "{} - {} ({})",
                d.path,
                d.model,
                humansize::format_size(d.size, humansize::BINARY)
            )
        })
        .collect();

    options.push("Manual Entry".to_string());

    let selection = Select::new("Which storage device do you want to analyze?", options)
        .with_page_size(10)
        .prompt()?;

    let device_path = if selection == "Manual Entry" {
        Text::new("Enter the device path (e.g., /dev/sdb):")
            .with_validator(|input: &str| {
                if input.trim().is_empty() {
                    Ok(inquire::validator::Validation::Invalid(
                        "Path cannot be empty".into(),
                    ))
                } else {
                    Ok(inquire::validator::Validation::Valid)
                }
            })
            .prompt()?
    } else {
        selection
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string()
    };

    if device_path.is_empty() {
        anyhow::bail!("Invalid device path selection");
    }

    let output_path_str = Text::new("Where do you want to save the recovered images?")
        .with_default("./recovered")
        .with_validator(|input: &str| {
            if input.trim().is_empty() {
                Ok(inquire::validator::Validation::Invalid(
                    "Path cannot be empty".into(),
                ))
            } else {
                Ok(inquire::validator::Validation::Valid)
            }
        })
        .prompt()?;

    if device_path == output_path_str {
        println!("⚠️  Warning: Source and Destination look identical. This is dangerous!");
        if !Confirm::new("Are you sure you want to proceed?")
            .with_default(false)
            .prompt()?
        {
            println!("Operation cancelled.");
            return Ok(());
        }
    }
    println!("\n📊 Configuration:");
    println!("   Target Device: {}", device_path);
    println!("   Output Path:   {}", output_path_str);

    if !Confirm::new("Start recovery scan?")
        .with_default(true)
        .prompt()?
    {
        println!("Operation cancelled.");
        return Ok(());
    }

    run_recover(
        &device_path,
        Path::new(&output_path_str),
        None,
        true,
        false,
        true,
        100,  
        600,  
        600,  
        false, 
    )?;

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
    min_size_kb: usize,
    min_width: u32,
    min_height: u32,
    no_validation: bool,
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

    let validator = if no_validation {
        None
    } else {
        let validation_config = ValidationConfig::new()
            .with_min_size(min_size_kb)
            .with_min_dimensions(min_width, min_height);
        Some(ImageValidator::new(validation_config))
    };
    let validation_stats = ValidationStats::new();

    let mut files_recovered = 0;

    let mut matches = scan_result.matches.clone();
    matches.sort_by_key(|m| m.start_offset());

    for (id, m) in matches.iter().enumerate() {
        let max_size = m.estimated_size() as usize;
        let read_size = max_size.min(200 * 1024 * 1024);

        match device.read_at(m.start_offset(), read_size) {
            Ok(data) => {
                if let Ok(file) = carver.recover_file(&data, m, id as u64, validator.as_ref()) {
                    validation_stats.record(file.validation_result);
                    
                    if save_recovered_file(&file, output_path, convert_png, overwrite, organize)? {
                        files_recovered += 1;
                    }
                }
            }
            Err(e) => eprintln!("Error reading at {}: {}", m.start_offset(), e),
        }
        recovery_progress.recovery_inc();
    }

    recovery_progress.finish("Recovery complete!");
    
    println!("\n{}", validation_stats.summary());
    println!(
        "\n✅ Successfully saved {} files to {}",
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
) -> Result<bool> {
    if !file.validation_result.is_valid() {
        log::debug!(
            "Skipping file {} (offset {}): {}",
            file.id,
            file.offset,
            file.validation_result.reason()
        );
        return Ok(false);
    }

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
        return Ok(false);
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
    Ok(true)
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
