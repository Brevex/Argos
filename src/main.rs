use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use argos::core::FragmentMap;
use argos::device::{device_selection_options, discover_block_devices};
use argos::io::{DiskScanner, PollResult};
use argos::recovery::carving::RecoveryStats;
use argos::{extraction, fs, io, recovery, scan};

const PROGRESS_MSG_INTERVAL: u64 = 50 * 1024 * 1024;

#[derive(Parser)]
#[command(name = "argos")]
#[command(version = env!("CARGO_PKG_VERSION"))]
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
        run_interactive_wizard(cli.yes)?;
    } else if let (Some(device), Some(output)) = (cli.device, cli.output) {
        run_scan(&device, &output)?;
    } else {
        run_interactive_wizard(cli.yes)?;
    }
    Ok(())
}

fn run_interactive_wizard(skip_confirm: bool) -> Result<()> {
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
        "{:<10} {:<8} {:>10} {}",
        style("NAME").bold(),
        style("TYPE").bold(),
        style("SIZE").bold(),
        style("PATH").bold()
    );
    println!("{}", "-".repeat(45));

    for device in &devices {
        println!(
            "{:<10} {:<8} {:>10} {}",
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
        "Target: {} ({})",
        selected_device.path,
        selected_device.size_human()
    );
    println!("Output: {}", output_dir);
    println!("Modes:  JPEG, PNG");
    println!();

    if !skip_confirm {
        let confirmed = Confirm::with_theme(&theme)
            .with_prompt("Confirm and start scan?")
            .default(true)
            .interact()
            .context("Failed to confirm")?;

        if !confirmed {
            println!("\nOperation cancelled.");
            return Ok(());
        }
    }

    run_scan(&PathBuf::from(&selected_device.path), &output_path)?;

    Ok(())
}

fn run_scan(device_path: &Path, output_path: &Path) -> Result<()> {
    println!();

    let reader = io::DiskReader::open(device_path)
        .with_context(|| format!("Failed to open device: {:?}", device_path))?;

    let disk_size = reader.size();
    let mut scanner = DiskScanner::new(reader);
    let pb = ProgressBar::new(disk_size);

    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40.cyan/blue}] {bytes}/{total_bytes} {msg}")?
            .progress_chars("=>-"),
    );

    let mut map = FragmentMap::with_disk_estimate(disk_size);
    let mut last_msg_pos = 0u64;
    let max_in_flight = rayon::current_num_threads().min(5);
    let (result_tx, result_rx) = crossbeam_channel::bounded(max_in_flight * 2);
    let mut in_flight = 0usize;
    let mut scanner_done = false;

    loop {
        while let Ok((fragments, buffer)) = result_rx.try_recv() {
            for f in fragments {
                map.push(f);
            }

            scanner.recycle_buffer(buffer);
            in_flight -= 1;
        }

        let disk_pos = scanner.disk_position();
        pb.set_position(disk_pos);
        if disk_pos >= last_msg_pos + PROGRESS_MSG_INTERVAL {
            pb.set_message(format!("Found {} fragments", map.len()));
            last_msg_pos = disk_pos;
        }

        if scanner_done && in_flight == 0 {
            break;
        }

        if !scanner_done && in_flight < max_in_flight {
            match scanner.poll_block()? {
                PollResult::Block(block) => {
                    let tx = result_tx.clone();

                    rayon::spawn(move || {
                        let mut local = Vec::new();
                        scan::scan_block(block.offset, block.data(), &mut local);
                        let _ = tx.send((local, block.buffer));
                    });

                    in_flight += 1;
                    continue;
                }
                PollResult::Pending => {}
                PollResult::Done => {
                    scanner_done = true;
                }
            }
        }

        if in_flight > 0 {
            match result_rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok((fragments, buffer)) => {
                    for f in fragments {
                        map.push(f);
                    }

                    scanner.recycle_buffer(buffer);
                    in_flight -= 1;
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        } else if !scanner_done {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    drop(result_tx);
    drop(result_rx);

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

    println!();
    println!();
    println!("Analyzing fragments...");

    let counts = map.count_by_kind();

    println!("JPEG headers: {}", counts.jpeg_headers);
    println!("JPEG footers: {}", counts.jpeg_footers);
    println!("PNG headers:  {}", counts.png_headers);
    println!("PNG footers:  {}", counts.png_footers);
    println!();

    map.sort_by_offset();
    map.dedup();

    let reader = scanner.into_reader();

    let linear_total = map.jpeg_headers().len() + map.png_headers().len();
    let pb_linear = ProgressBar::new(linear_total as u64);

    pb_linear.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40.yellow/white}] {pos}/{len} Linear carving ({percent}%)")?
            .progress_chars("=>-"),
    );

    let linear_cb = |current: usize, _total: usize| {
        pb_linear.set_position(current as u64);
    };

    let mut recovered = recovery::linear_carve(&map, &reader, Some(&linear_cb));
    pb_linear.finish_with_message(format!(
        "Linear carving complete — {} images found",
        style(recovered.len()).green().bold()
    ));

    let recovered_offsets: HashSet<u64> = recovered.iter().map(|r| r.header_offset()).collect();

    println!("Scanning for filesystem metadata...");
    let fs_hints = fs::collect_hints(&reader);
    if !fs_hints.is_empty() {
        println!(
            "Found {} filesystem hints for deleted files",
            style(fs_hints.len()).cyan()
        );
    }

    if map.len() > recovered.len() * 2 {
        let reasm_total = map.jpeg_headers().len() + map.png_headers().len();
        let pb_reasm = ProgressBar::new(reasm_total as u64);

        pb_reasm.set_style(
            ProgressStyle::default_bar()
                .template("[{bar:40.magenta/white}] {pos}/{len} Fragment reassembly ({percent}%)")?
                .progress_chars("=>-"),
        );

        let reasm_cb = |current: usize, _total: usize| {
            pb_reasm.set_position(current as u64);
        };

        let hints_ref = if fs_hints.is_empty() {
            None
        } else {
            Some(&fs_hints)
        };

        let reassembled = recovery::reassemble(
            &map,
            &reader,
            &recovered_offsets,
            hints_ref,
            Some(&reasm_cb),
        );
        pb_reasm.finish_with_message(format!(
            "Fragment reassembly complete — {} images found",
            style(reassembled.len()).green().bold()
        ));

        recovered.extend(reassembled);
    }

    let stats = RecoveryStats::from_recovered(&recovered);

    println!();
    println!();
    println!(
        "Found {} recoverable images:",
        style(stats.total_files()).green().bold()
    );
    println!("JPEG (linear):      {}", stats.jpeg_linear);
    println!("JPEG (reassembled): {}", stats.jpeg_reassembled);
    println!("PNG (linear):       {}", stats.png_linear);
    println!("PNG (reassembled):  {}", stats.png_reassembled);

    if recovered.is_empty() {
        println!("\n[!] No recoverable images found.");
        return Ok(());
    }

    println!();
    println!(
        "Extracting {} validated images to {:?}...",
        recovered.len(),
        output_path
    );
    println!();

    let pb = ProgressBar::new(recovered.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40.green/white}] {pos}/{len} ({percent}%)")?
            .progress_chars("=>-"),
    );

    let progress_cb = |current: usize, _total: usize| {
        pb.set_position(current as u64);
    };

    let report = extraction::extract_all(&recovered, &reader, output_path, Some(&progress_cb))?;

    pb.finish_with_message("done");

    println!();
    println!();
    println!("{}", style("Recovery Complete!").green().bold());
    println!();
    println!(
        "Images extracted: {}",
        style(report.extracted.len()).green()
    );
    if report.high_confidence > 0 {
        println!(
            "High confidence:    {}",
            style(report.high_confidence).green()
        );
    }
    if report.partial_confidence > 0 {
        println!(
            "Partial confidence: {}",
            style(report.partial_confidence).yellow()
        );
    }
    if report.low_confidence > 0 {
        println!("Low confidence:     {}", style(report.low_confidence).dim());
    }
    if report.dedup_skipped > 0 {
        println!("Duplicates removed: {}", style(report.dedup_skipped).cyan());
    }
    if report.failed > 0 {
        println!("Failed:             {}", style(report.failed).yellow());
        if report.tail_check_failed > 0 {
            println!(
                "No valid footer:    {}",
                style(report.tail_check_failed).dim()
            );
        }
        if report.head_validation_failed > 0 {
            println!(
                "Invalid header:     {}",
                style(report.head_validation_failed).dim()
            );
        }
        if report.decode_failed > 0 {
            println!(
                "Decode errors:      {} (kept as partial)",
                style(report.decode_failed).dim()
            );
        }
    }
    if report.corrupt_discarded > 0 {
        println!(
            "Corrupt/dropped:    {}",
            style(report.corrupt_discarded).yellow()
        );
    }
    println!();
    println!("Output folder:      {:?}", output_path);
    println!();

    Ok(())
}

fn print_banner() {
    println!();
    println!("{}", style("Argos - Image Recovery Tool").cyan().bold());
}
