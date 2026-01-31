use anyhow::{Context, Result};
use clap::Parser;
use console::{style, Emoji};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

mod analysis;
mod carving;
mod devices;
mod extraction;
mod formats;
mod io;
mod types;

use carving::RecoveryStats;
use devices::{device_selection_options, discover_block_devices};
use io::DiskScanner;
use types::FragmentMap;

static CRYSTAL_BALL: Emoji<'_, '_> = Emoji("🔮 ", "");
static MAGNIFYING_GLASS: Emoji<'_, '_> = Emoji("🔍 ", "");
static DISK: Emoji<'_, '_> = Emoji("📀 ", "");
static CLIPBOARD: Emoji<'_, '_> = Emoji("📋 ", "");
static CHECK: Emoji<'_, '_> = Emoji("✔ ", "[OK] ");
static SPARKLES: Emoji<'_, '_> = Emoji("✨ ", "");
static FOLDER: Emoji<'_, '_> = Emoji("📁 ", "");
static WARNING: Emoji<'_, '_> = Emoji("⚠️  ", "[!] ");

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

    println!(
        "\n{}{}",
        MAGNIFYING_GLASS,
        style("Discovering block devices...").cyan()
    );

    let devices = discover_block_devices();

    if devices.is_empty() {
        println!(
            "\n{}{}",
            WARNING,
            style("No block devices found. Are you running as root?").yellow()
        );
        println!("Try: sudo ./argos --scan");
        return Ok(());
    }

    println!("\n{}{}", DISK, style("Found Devices:").green().bold());
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
        .with_prompt(format!("{}Select device for analysis", CHECK))
        .items(&options)
        .default(0)
        .interact()
        .context("Failed to select device")?;

    let selected_device = &devices[selection];

    let output_dir: String = Input::with_theme(&theme)
        .with_prompt(format!(
            "{}Where do you want to save the recovered files?",
            CHECK
        ))
        .default("./recovered".to_string())
        .interact_text()
        .context("Failed to get output directory")?;

    let output_path = PathBuf::from(&output_dir);

    println!();
    println!("{}{}", CLIPBOARD, style("Operation Summary:").cyan().bold());
    println!(
        "   • Target:  {} ({})",
        selected_device.path,
        selected_device.size_human()
    );
    println!("   • Output:  {}", output_dir);
    println!("   • Modes:   JPEG, PNG");
    println!();

    let confirmed = Confirm::with_theme(&theme)
        .with_prompt(format!("{}Confirm and start scan?", CHECK))
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
            .progress_chars("█▓▒░  "),
    );

    let mut map = FragmentMap::new();
    let mut blocks_processed = 0u64;

    while let Some((offset, data)) = scanner.next_block()? {
        analysis::scan_block(offset, data, &mut map);

        blocks_processed += data.len() as u64;
        pb.set_position(blocks_processed);

        if blocks_processed % (100 * 1024 * 1024) == 0 {
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
            "\n{}{} bad sectors skipped",
            WARNING,
            style(bad_sectors.len()).yellow()
        );
    }

    println!("\n{}Analyzing fragments...", SPARKLES);

    map.sort_by_offset();
    let mut recovered = carving::linear_carve(&map.fragments);

    if map.len() > recovered.len() * 2 {
        println!("{}Attempting bifragment recovery...", SPARKLES);
        let mut reader = io::DiskReader::open(device_path)?;
        let bifrag = carving::bifragment_carve(&map.fragments, &mut reader);
        recovered.extend(bifrag);
    }

    let stats = RecoveryStats::from_recovered(&recovered);

    println!(
        "\n{}Found {} recoverable images:",
        CHECK,
        style(stats.total_files()).green().bold()
    );
    println!("   • JPEG (linear):     {}", stats.jpeg_linear);
    println!("   • JPEG (bifragment): {}", stats.jpeg_bifragment);
    println!("   • PNG (linear):      {}", stats.png_linear);
    println!("   • PNG (bifragment):  {}", stats.png_bifragment);

    if recovered.is_empty() {
        println!("\n{}No recoverable images found.", WARNING);
        return Ok(());
    }

    println!(
        "\n{}{}Extracting files to {:?}...",
        FOLDER,
        style("").cyan(),
        output_path
    );

    let pb = ProgressBar::new(recovered.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40.green/white}] {pos}/{len} files")?
            .progress_chars("█▓▒░  "),
    );

    let extracted = extraction::extract_all(&recovered, device_path, output_path)?;

    pb.finish();

    println!();
    println!("{}", "=".repeat(55));
    println!("{}{}", SPARKLES, style("Recovery Complete!").green().bold());
    println!("{}", "=".repeat(55));
    println!();
    println!("   Files recovered: {}", style(extracted.len()).green());
    println!("   Total size:      {}", format_bytes(stats.total_bytes));
    println!("   Output folder:   {:?}", output_path);
    println!();

    Ok(())
}

fn print_banner() {
    println!();
    println!(
        "{}{}",
        CRYSTAL_BALL,
        style("Argos - Image Recovery Wizard").cyan().bold()
    );
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
