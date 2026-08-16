//! How much of a medium is image data that no signature can reach.
//!
//! Carving finds a file because it finds the file's signature. A photograph
//! whose first block was reused is invisible to it, however intact the rest of
//! it is — and on a medium that has been reformatted and refilled, that is the
//! expected state of the oldest data rather than an exception.
//!
//! This measures the size of that blind spot before anything is built to see
//! into it. For every 4 KiB block it asks the classifier the engine already
//! uses what the block is, and the detector the engine already uses whether a
//! header stands close enough in front of it for any carve to have reached it.
//! A block the classifier calls a JPEG entropy scan, with no `SOI` within a
//! photograph's length before it, belongs to a file no signature-driven pass
//! can produce.
//!
//! Reads only, through the same read-only device path a scan uses.
//!
//! ```text
//! cargo run --release --example orphan_census -- /dev/sdX --from 0 --to 1000204886016
//! ```

use std::io::{Read, Seek, SeekFrom, Write as _};

use argos_carve::classify::{self, BlockClass};
use argos_carve::{Candidate, Detector};
use argos_core::Format;
use argos_core::geometry::ByteOffset;
use argos_device::{BlockReader, Device};

/// Bytes examined per sample, unless `--window` says otherwise.
const WINDOW_BYTES: u64 = 64 * 1024 * 1024;

/// Distance between the starts of two samples, unless `--stride` says
/// otherwise. At eight times the window, one byte in eight is read — enough to
/// put a proportion within a point or so, and a fraction of the hours a whole
/// surface costs. Setting it equal to the window reads everything.
const STRIDE_BYTES: u64 = 512 * 1024 * 1024;

/// How far back a header may stand and still account for a block.
///
/// A single consumer photograph is under this by a wide margin — the largest
/// this disk yielded was 4.9 MB — so a block with no signature within this
/// distance before it cannot be part of a file any signature names. Kept well
/// above the real maximum on purpose: it makes the orphan count a floor rather
/// than an estimate.
const HEADER_REACH_BYTES: u64 = 32 * 1024 * 1024;

/// Read in pieces this size, so one unreadable span costs a piece.
const READ_BYTES: usize = 4 * 1024 * 1024;

/// Signature hits one window may collect.
const HIT_CAP: usize = 1 << 20;

#[derive(Default)]
struct Tally {
    windows: u64,
    read: u64,
    unreadable: u64,
    by_class: [u64; 5],
    jpeg_hits: u64,
    /// JPEG entropy blocks with a signature within reach before them.
    reachable: u64,
    /// JPEG entropy blocks with none.
    orphaned: u64,
    /// Reachable blocks by how far the header stands before them: under 1 MiB,
    /// 1–4, 4–16, 16–32. An orphan's distance is not measurable here — the
    /// window only looks back as far as the reach — so it has no bucket.
    distance: [u64; 4],
}

impl Tally {
    fn class_slot(class: BlockClass) -> usize {
        match class {
            BlockClass::LowEntropy => 0,
            BlockClass::TextOrSparse => 1,
            BlockClass::HighEntropy => 2,
            BlockClass::Deflate => 3,
            BlockClass::JpegStream => 4,
        }
    }

    fn add(&mut self, other: &Self) {
        self.windows += other.windows;
        self.read += other.read;
        self.unreadable += other.unreadable;
        for (mine, theirs) in self.by_class.iter_mut().zip(other.by_class) {
            *mine += theirs;
        }
        self.jpeg_hits += other.jpeg_hits;
        self.reachable += other.reachable;
        self.orphaned += other.orphaned;
        for (mine, theirs) in self.distance.iter_mut().zip(other.distance) {
            *mine += theirs;
        }
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "byte counts printed as mebibytes and percentages; f64 carries a medium's \
                  byte count exactly up to 4 PiB, far past anything this reads"
    )]
    fn report(&self, name: &str) {
        let mib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
        let pct = |part: u64| {
            if self.read == 0 {
                0.0
            } else {
                100.0 * part as f64 / self.read as f64
            }
        };
        println!("\n=== {name} ===");
        println!(
            "  amostrado    {:.1} MiB em {} janelas   ilegível {:.1} MiB",
            mib(self.read),
            self.windows,
            mib(self.unreadable)
        );
        for (slot, label) in [
            (0, "low-entropy   "),
            (1, "text-or-sparse"),
            (2, "high-entropy  "),
            (3, "deflate       "),
            (4, "jpeg-stream   "),
        ] {
            println!(
                "  {label} {:>10.1} MiB  {:>5.1}%",
                mib(self.by_class[slot]),
                pct(self.by_class[slot])
            );
        }
        let jpeg = self.by_class[4];
        println!("  assinaturas JPEG encontradas: {}", self.jpeg_hits);
        if jpeg > 0 {
            println!(
                "  dados de entropia JPEG alcançáveis por assinatura: {:.1} MiB ({:.1}% deles)",
                mib(self.reachable),
                100.0 * self.reachable as f64 / jpeg as f64
            );
            println!(
                "  ÓRFÃOS — sem cabeçalho a até {} MiB antes:          {:.1} MiB ({:.1}% deles)",
                HEADER_REACH_BYTES / (1024 * 1024),
                mib(self.orphaned),
                100.0 * self.orphaned as f64 / jpeg as f64
            );
            let labels = ["<1", "1-4", "4-16", "16-32"];
            print!("  distância do bloco alcançável ao seu cabeçalho (MiB):");
            for (slot, label) in labels.iter().enumerate() {
                print!("  {label}={:.1}MiB", mib(self.distance[slot]));
            }
            println!();
        }
    }
}

/// A medium read through whichever path suits it, and its length.
///
/// A block device goes through the device HAL, which is where the read-only
/// guarantee lives; a raw image is an ordinary file opened for reading. Both so
/// the same census can be run against an acquired image, which is where it
/// should be run when the medium is worth reading once.
enum Medium {
    Device(Box<BlockReader<Device>>),
    Image(std::fs::File),
}

impl Read for Medium {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Device(reader) => reader.read(buf),
            Self::Image(file) => file.read(buf),
        }
    }
}

impl Seek for Medium {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Device(reader) => reader.seek(pos),
            Self::Image(file) => file.seek(pos),
        }
    }
}

fn open_read_only(path: &std::path::Path) -> anyhow::Result<(Medium, u64)> {
    use anyhow::Context as _;

    // Named, because a device node is not a stable name for a medium: this
    // disk answered to `sdc` yesterday and to `sdd` today. An error that does
    // not say what it looked for leaves the reader guessing at which.
    let metadata =
        std::fs::metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;
    if metadata.is_file() {
        let file = std::fs::File::open(path)
            .with_context(|| format!("cannot open {} for reading", path.display()))?;
        return Ok((Medium::Image(file), metadata.len()));
    }
    let device = Device::open(path)
        .with_context(|| format!("cannot open device {} read-only", path.display()))?;
    let reader = BlockReader::new(device);
    let len = reader
        .geometry()
        .capacity_bytes()
        .ok_or_else(|| anyhow::anyhow!("the device reports a geometry that overflows u64"))?;
    Ok((Medium::Device(Box::new(reader)), len))
}

/// Which bucket a reachable block's distance to its header falls in.
fn bucket(distance: u64) -> usize {
    match distance / (1024 * 1024) {
        0 => 0,
        1..4 => 1,
        4..16 => 2,
        _ => 3,
    }
}

/// Reads `len` bytes at `at`, leaving unreadable pieces zeroed and counting
/// them. Zeroes classify as low-entropy, so they never inflate an image count.
fn read_window<R: Read + Seek>(src: &mut R, at: u64, len: usize, buf: &mut Vec<u8>) -> u64 {
    buf.clear();
    buf.resize(len, 0);
    let mut filled = 0_usize;
    let mut lost = 0_u64;
    while filled < len {
        let take = READ_BYTES.min(len - filled);
        let piece = at.saturating_add(filled as u64);
        let ok = src
            .seek(SeekFrom::Start(piece))
            .and_then(|_| src.read_exact(&mut buf[filled..filled + take]))
            .is_ok();
        if !ok {
            buf[filled..filled + take].fill(0);
            lost += take as u64;
        }
        filled += take;
    }
    lost
}

/// Measures one window, whose bytes start at `origin`. `lookback` is how much
/// of the front of `buf` is context read only to find headers standing before
/// the window proper.
fn measure(buf: &[u8], origin: u64, lookback: usize, detector: &Detector, tally: &mut Tally) {
    let mut hits: Vec<Candidate> = Vec::new();
    detector.hits_in(buf, ByteOffset::new(origin), HIT_CAP, &mut hits);
    let jpeg: Vec<u64> = hits
        .iter()
        .filter(|hit| hit.format == Format::Jpeg)
        .map(|hit| hit.offset.get())
        .collect();
    tally.jpeg_hits += jpeg
        .iter()
        .filter(|at| **at >= origin.saturating_add(lookback as u64))
        .count() as u64;

    let block = classify::BLOCK_BYTES;
    let mut at = lookback;
    while at + block <= buf.len() {
        let profile = classify::classify(&buf[at..at + block]);
        tally.by_class[Tally::class_slot(profile.class)] += block as u64;
        if profile.class == BlockClass::JpegStream {
            let position = origin.saturating_add(at as u64);
            // The nearest signature at or before this block.
            let nearest = jpeg.partition_point(|hit| *hit <= position);
            let distance = nearest
                .checked_sub(1)
                .map(|index| position.saturating_sub(jpeg[index]));
            match distance {
                Some(gap) if gap <= HEADER_REACH_BYTES => {
                    tally.reachable += block as u64;
                    tally.distance[bucket(gap)] += block as u64;
                }
                _ => tally.orphaned += block as u64,
            }
        }
        at += block;
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the percentage on the progress line; exact to well past any real medium"
)]
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!(
            "usage: orphan_census <medium> [--from B] [--to B] [--split B] [--window B] [--stride B]"
        );
        std::process::exit(2);
    });
    let (mut from, mut to, mut split) = (0_u64, u64::MAX, None);
    let (mut window, mut stride) = (WINDOW_BYTES, STRIDE_BYTES);
    while let Some(flag) = args.next() {
        let value = args.next().unwrap_or_default().parse::<u64>().ok();
        match (flag.as_str(), value) {
            ("--from", Some(at)) => from = at,
            ("--to", Some(at)) => to = at,
            ("--split", Some(at)) => split = Some(at),
            ("--window", Some(at)) => window = at.max(classify::BLOCK_BYTES as u64),
            ("--stride", Some(at)) => stride = at.max(1),
            _ => {}
        }
    }
    let stride = stride.max(window);

    let (mut reader, len) = open_read_only(std::path::Path::new(&path))?;
    let to = to.min(len);
    println!(
        "medium {path}: {len} bytes; amostrando {} MiB a cada {} MiB entre {from} e {to}",
        window / (1024 * 1024),
        stride / (1024 * 1024)
    );
    if let Some(at) = split {
        println!("dividindo o relatório em {at}");
    }

    let detector = Detector::new();
    let mut buf = Vec::new();
    let (mut low, mut high, mut all) = (Tally::default(), Tally::default(), Tally::default());

    let mut at = from;
    while at < to {
        let lookback = HEADER_REACH_BYTES.min(at.saturating_sub(from));
        let span = window.min(to - at);
        let total = lookback.saturating_add(span);
        let Ok(total) = usize::try_from(total) else {
            break;
        };
        let start = at - lookback;
        let mut window = Tally {
            windows: 1,
            read: span,
            ..Tally::default()
        };
        window.unreadable = read_window(&mut reader, start, total, &mut buf);
        let Ok(lookback) = usize::try_from(lookback) else {
            break;
        };
        measure(&buf, start, lookback, &detector, &mut window);

        all.add(&window);
        match split {
            Some(boundary) if at >= boundary => high.add(&window),
            Some(_) => low.add(&window),
            None => {}
        }
        print!(
            "\r  {:.1}% ",
            100.0 * (at - from) as f64 / (to - from) as f64
        );
        let _ = std::io::stdout().flush();
        at = at.saturating_add(stride);
    }
    println!();

    if let Some(boundary) = split {
        low.report(&format!("antes de {boundary}"));
        high.report(&format!("a partir de {boundary}"));
    }
    all.report("medium inteiro");
    Ok(())
}
