use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

use argos::carving::RecoveryStats;
use argos::devices::{device_selection_options, discover_block_devices};
use argos::io::DiskScanner;
use argos::types::FragmentMap;
use argos::{analysis, carving, extraction, io};

const PROGRESS_UPDATE_INTERVAL: u64 = 100 * 1024 * 1024;

#[derive(Parser)]
#[command(name = "argos")]
#[command(version = "0.1.0")]
#[command(about = "Professional forensic image recovery tool")]
#[command(author = "Argos Project")]
struct Cli {
    #[arg(long)]
    scan: bool,

    #[arg(short, long)]
    device: Option<PathBuf>,

    #[arg(short, long)]
    output: Option<PathBuf>,

    #[arg(short = 'y', long)]
    yes: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.scan {
        run_interactive_wizard()?;
    } else if let (Some(device), Some(output)) = (cli.device, cli.output) {
        run_scan(&device, &output)?;
    } else {
        run_interactive_wizard()?;
    }
    Ok(())
}

fn run_interactive_wizard() -> Result<()> {
    print_banner();

    println!("\n{}", style("Discovering block devices...").cyan());

    let devices = discover_block_devices();

    if devices.is_empty() {
        println!(
            "\n[!] {}",
            style("No block devices found. Are you running as root?").yellow()
        );
        println!("Try: sudo ./argos --scan");
        return Ok(());
    }

    println!("\n{}", style("Found Devices:").green().bold());
    println!();
    println!(
        "{:<12} {:<15} {:>12} {}",
        style("NAME").bold(),
        style("TYPE").bold(),
        style("SIZE").bold(),
        style("PATH").bold()
    );
    println!("{}", "-".repeat(55));

    for device in &devices {
        println!(
            "{:<12} {:<15} {:>12} {}",
            device.name,
            format!("{}", device.device_type),
            device.size_human(),
            device.path
        );
    }

    println!();

    let theme = ColorfulTheme::default();
    let options = device_selection_options(&devices);

    let selection = Select::with_theme(&theme)
        .with_prompt("Select device for analysis")
        .items(&options)
        .default(0)
        .interact()
        .context("Failed to select device")?;

    let selected_device = &devices[selection];

    let output_dir: String = Input::with_theme(&theme)
        .with_prompt("Where do you want to save the recovered files?")
        .default("./recovered".to_string())
        .interact_text()
        .context("Failed to get output directory")?;

    let output_path = PathBuf::from(&output_dir);

    println!();
    println!("{}", style("Operation Summary:").cyan().bold());
    println!(
        "   - Target:  {} ({})",
        selected_device.path,
        selected_device.size_human()
    );
    println!("   - Output:  {}", output_dir);
    println!("   - Modes:   JPEG, PNG");
    println!();

    let confirmed = Confirm::with_theme(&theme)
        .with_prompt("Confirm and start scan?")
        .default(true)
        .interact()
        .context("Failed to confirm")?;

    if !confirmed {
        println!("\nOperation cancelled.");
        return Ok(());
    }

    run_scan(&PathBuf::from(&selected_device.path), &output_path)?;

    Ok(())
}

fn run_scan(device_path: &PathBuf, output_path: &PathBuf) -> Result<()> {
    println!();

    let reader = io::DiskReader::open(device_path)
        .context(format!("Failed to open device: {:?}", device_path))?;

    let disk_size = reader.size();
    let mut scanner = DiskScanner::new(reader);

    let pb = ProgressBar::new(disk_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40.cyan/blue}] {bytes}/{total_bytes} {msg}")?
            .progress_chars("=>-"),
    );

    let mut map = FragmentMap::new();
    let mut blocks_processed = 0u64;

    while let Some((offset, data)) = scanner.next_block()? {
        analysis::scan_block(offset, data, &mut map);

        blocks_processed += data.len() as u64;
        pb.set_position(blocks_processed);

        if blocks_processed.is_multiple_of(PROGRESS_UPDATE_INTERVAL) {
            pb.set_message(format!("Found {} fragments", map.len()));
        }
    }

    pb.finish_with_message(format!(
        "Scan complete! Found {} fragments",
        style(map.len()).green().bold()
    ));

    let bad_sectors = scanner.bad_sectors();
    if !bad_sectors.is_empty() {
        println!(
            "\n[!] {} bad sectors skipped",
            style(bad_sectors.len()).yellow()
        );
    }

    println!("\nAnalyzing fragments...");

    let jpeg_headers = map.jpeg_headers().count();
    let jpeg_footers = map.jpeg_footers().count();
    let png_headers = map.png_headers().count();
    let png_footers = map.png_footers().count();

    println!("   Fragment breakdown:");
    println!("   - JPEG headers: {}", jpeg_headers);
    println!("   - JPEG footers: {}", jpeg_footers);
    println!("   - PNG headers:  {}", png_headers);
    println!("   - PNG footers:  {}", png_footers);

    map.sort_by_offset();
    map.dedup();
    let mut recovered = carving::linear_carve(&map);

    if map.len() > recovered.len() * 2 {
        println!("Attempting bifragment recovery...");
        let mut reader = io::DiskReader::open(device_path)?;
        let bifrag = carving::bifragment_carve(&map, &mut reader);
        recovered.extend(bifrag);
    }

    let stats = RecoveryStats::from_recovered(&recovered);

    println!(
        "\nFound {} recoverable images:",
        style(stats.total_files()).green().bold()
    );
    println!("   - JPEG (linear):     {}", stats.jpeg_linear);
    println!("   - JPEG (bifragment): {}", stats.jpeg_bifragment);
    println!("   - PNG (linear):      {}", stats.png_linear);
    println!("   - PNG (bifragment):  {}", stats.png_bifragment);

    if recovered.is_empty() {
        println!("\n[!] No recoverable images found.");
        return Ok(());
    }

    println!(
        "\nExtracting and validating {} candidates to {:?}...",
        recovered.len(),
        output_path
    );

    let pb = ProgressBar::new(recovered.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40.green/white}] {pos}/{len} ({percent}%)")?
            .progress_chars("=>-"),
    );

    let progress_cb = |current: usize, _total: usize| {
        pb.set_position(current as u64);
    };

    let extracted =
        extraction::extract_all(&recovered, device_path, output_path, Some(&progress_cb))?;

    pb.finish_with_message("done");

    println!();
    println!("{}", "=".repeat(55));
    println!("{}", style("Recovery Complete!").green().bold());
    println!("{}", "=".repeat(55));
    println!();
    println!("   Candidates processed: {}", recovered.len());
    println!(
        "   Valid files recovered: {}",
        style(extracted.len()).green()
    );
    println!("   Output folder:   {:?}", output_path);
    println!();

    Ok(())
}

fn print_banner() {
    println!();
    println!("{}", style("Argos - Image Recovery Tool").cyan().bold());
}
