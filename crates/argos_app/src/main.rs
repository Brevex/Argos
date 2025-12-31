//! Argos - Forensic Image Recovery Tool
//!
//! A high-performance CLI tool for recovering images from disks on Linux.

mod device_discovery;
mod engine;
mod recovery;

use anyhow::{Context, Result};
use clap::Parser;
use dialoguer::{theme::ColorfulTheme, Select};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use device_discovery::{discover_disks, DiskInfo};
use argos_core::BlockSource;
use argos_io::DiskReader;

#[derive(Parser, Debug)]
#[command(name = "argos")]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    device: Option<String>,

    #[arg(short, long, default_value = "./recovered")]
    output: String,

    #[arg(short, long, default_value_t = false)]
    verbose: bool,

    #[arg(short, long, default_value_t = false)]
    scan: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl+C handler")?;

    let device_path = if let Some(path) = args.device {
        println!("Dispositivo selecionado: {}", path);
        path
    } else {
        let selected_disk = interactive_device_selection()?;
        println!(
            "\n✅ Dispositivo selecionado: {} ({})",
            selected_disk.path,
            selected_disk.human_size()
        );
        selected_disk.path
    };

    if args.scan {
        let output_path = std::path::Path::new(&args.output);
        std::fs::create_dir_all(output_path)?;
        engine::run_scan(&device_path, output_path, running)?;
    } else {
        run_smoke_test(&device_path)?;
    }

    Ok(())
}

fn interactive_device_selection() -> Result<DiskInfo> {
    println!("🔍 Descobrindo dispositivos de bloco...\n");

    let disks = discover_disks().context("Failed to discover disk devices")?;

    if disks.is_empty() {
        anyhow::bail!(
            "Nenhum dispositivo de bloco encontrado.\n\
             Verifique se você tem permissões para ler /sys/block."
        );
    }

    println!("📀 Dispositivos encontrados:\n");

    println!("{:<12} {:<15} {:>12} CAMINHO", "NOME", "TIPO", "TAMANHO");
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
        .with_prompt("Selecione o dispositivo para análise")
        .items(&items)
        .default(0)
        .interact()
        .context("Failed to show interactive selection")?;

    Ok(disks[selection].clone())
}

fn run_smoke_test(device_path: &str) -> Result<()> {
    println!("\n🧪 Smoke Test: Leitura dos primeiros 512 bytes...\n");

    let mut reader = DiskReader::new(device_path)
        .with_context(|| format!("Failed to open device: {}", device_path))?;

    println!("📊 Tamanho total do dispositivo: {} bytes\n", reader.size());

    let mut buffer = vec![0u8; 512];
    let bytes_read = reader
        .read_chunk(0, &mut buffer)
        .context("Failed to read first 512 bytes")?;

    println!("Bytes lidos: {}\n", bytes_read);

    hex_dump(&buffer[..bytes_read]);

    Ok(())
}

/// Displays a buffer in hexadecimal format (16 bytes per line).
/// Format: `OFFSET | HEX BYTES | ASCII`
fn hex_dump(data: &[u8]) {
    println!("Offset   | 00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F | ASCII");
    println!("{}", "-".repeat(75));

    for (i, chunk) in data.chunks(16).enumerate() {
        let offset = i * 16;

        print!("{:08x} | ", offset);
        for (j, byte) in chunk.iter().enumerate() {
            print!("{:02x} ", byte);
            if j == 7 {
                print!(" ");
            }
        }

        for j in chunk.len()..16 {
            print!("   ");
            if j == 7 {
                print!(" ");
            }
        }

        print!("| ");
        for byte in chunk {
            let ch = if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            };
            print!("{}", ch);
        }

        println!();
    }
    println!();
}
