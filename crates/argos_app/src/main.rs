mod device_discovery;
mod engine;
mod recovery;
mod signature_index;

use anyhow::{Context, Result};
use clap::Parser;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use device_discovery::{discover_disks, DiskInfo};

#[derive(Parser, Debug)]
#[command(name = "argos")]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = false)]
    scan: bool,

    #[arg(short, long, default_value_t = false)]
    multipass: bool,

    #[arg(short, long, default_value_t = false)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if !args.scan {
        use clap::CommandFactory;
        let mut cmd = Args::command();
        cmd.print_help()?;
        println!("\n\n❌ Usage: sudo ./argos --scan");
        return Ok(());
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl+C handler")?;

    println!("\n🔮 Argos - Image Recovery Wizard\n");

    let selected_disk = interactive_device_selection()?;
    println!(
        "\n✅ Selected Device: {} ({})",
        selected_disk.path,
        selected_disk.human_size()
    );

    println!();
    let output_path_str: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Where do you want to save the recovered files?")
        .default("./recovered".to_string())
        .interact_text()?;

    let output_path = std::path::Path::new(&output_path_str);

    println!("\n📋 Operation Summary:");
    println!(
        "   • Target:  {} ({})",
        selected_disk.path,
        selected_disk.human_size()
    );
    println!("   • Output:  {}", output_path.display());
    println!("   • Modes:   JPEG, PNG (Zero-Allocation Engine)");

    println!();
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Confirm and start scan?")
        .default(true)
        .interact()?
    {
        println!("\n❌ Operation cancelled.");
        return Ok(());
    }

    std::fs::create_dir_all(output_path)?;
    println!();

    if args.multipass {
        println!("🔬 Using multi-pass scan engine\n");
        engine::run_multipass_scan(&selected_disk.path, output_path, running)?;
    } else {
        engine::run_scan(&selected_disk.path, output_path, running)?;
    }

    Ok(())
}

fn interactive_device_selection() -> Result<DiskInfo> {
    println!("🔍 Discovering block devices...\n");

    let disks = discover_disks().context("Failed to discover disk devices")?;

    if disks.is_empty() {
        anyhow::bail!(
            "No block devices found.\n\
             Check if you have permissions to read /sys/block."
        );
    }

    println!("📀 Found Devices:\n");

    println!("{:<12} {:<15} {:>12} PATH", "NAME", "TYPE", "SIZE");
    println!("{}", "-".repeat(55));
    for disk in &disks {
        println!(
            "{:<12} {:<15} {:>12} {}",
            disk.name,
            disk.device_type,
            disk.human_size(),
            disk.path
        );
    }
    println!();

    let items: Vec<String> = disks.iter().map(|d| d.display()).collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select device for analysis")
        .items(&items)
        .default(0)
        .interact()
        .context("Failed to show interactive selection")?;

    Ok(disks[selection].clone())
}
