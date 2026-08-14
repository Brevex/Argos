//! The data every Argos client and the engine agree on.
//!
//! Each type here is a deliberate, flattened restatement of something the
//! engine knows — never a re-export of it. That costs a translation in
//! `argos`, and buys two things: the wire format changes only when someone
//! edits this file, and a client cannot reach engine behaviour through a type
//! it was handed (`M-DONT-LEAK-TYPES`).
//!
//! Nothing here carries recovered bytes. Artifacts and previews are files in
//! the session directory; what travels is their path, their size and their
//! hash (`A-DTO-VERSIONED`).
//!
//! # 64-bit values
//!
//! Byte offsets and lengths are `u64` here and JSON numbers on the wire, which
//! a JavaScript client reads as a double. Values above 2^53 lose precision —
//! that is a medium of nine petabytes, past anything this tool opens, and the
//! generated TypeScript says `number` because that is what `JSON.parse`
//! actually produces. A client that needs exact addressing at that scale
//! should read the manifest, which is written in full precision and is the
//! record of provenance regardless (`A-PROVENANCE`).

use serde::{Deserialize, Serialize};

/// Derives the TypeScript definition of a DTO when the `bindings` feature is
/// on, and nothing otherwise.
macro_rules! dto {
    ($(#[$meta:meta])* $vis:vis struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
        #[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
        #[cfg_attr(
            feature = "bindings",
            ts(export, export_to = "../../argos_ui/ui/src/lib/dto.ts", rename_all = "camelCase")
        )]
        #[serde(rename_all = "camelCase")]
        $vis struct $name { $($body)* }
    };
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
        #[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
        #[cfg_attr(
            feature = "bindings",
            ts(export, export_to = "../../argos_ui/ui/src/lib/dto.ts", rename_all = "camelCase")
        )]
        #[serde(rename_all = "camelCase")]
        $vis enum $name { $($body)* }
    };
}

dto! {
    /// What the engine answers a handshake with.
    ///
    /// A client that does not recognise [`Hello::schema`] must stop rather
    /// than guess: the two processes are versioned separately and a field
    /// silently missing is the worst way to find that out.
    pub struct Hello {
        /// Wire-format version, [`crate::SCHEMA_VERSION`].
        pub schema: u32,
        /// Version of the `argos` binary answering.
        pub tool_version: String,
    }
}

dto! {
    /// One medium a scan can be pointed at.
    pub struct Device {
        /// Path to open it by.
        pub path: String,
        /// `disk` or `partition`.
        pub kind: String,
        /// Capacity in bytes, when the platform reported one without opening
        /// the device.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub capacity_bytes: Option<u64>,
        /// What kind of medium the platform says it is.
        pub class: String,
        /// Whether the medium reports TRIM enabled: `enabled`, `disabled` or
        /// `unknown`.
        pub trim: String,
        /// Model or product string, when the platform offers one.
        pub model: Option<String>,
        /// Where the operating system currently has it mounted, if anywhere,
        /// already rendered for display.
        pub mounts: Vec<String>,
        /// Whether any mount of this medium is writable, so the bytes can
        /// change underneath a scan.
        pub writable_mount: bool,
    }
}

dto! {
    /// A shadow copy this machine holds.
    pub struct ShadowCopy {
        /// Device path to open it by.
        pub path: String,
        /// The snapshot's index in the object namespace.
        pub index: u32,
    }
}

dto! {
    /// What `devices.list` answers.
    pub struct Inventory {
        /// Media, whole disks first.
        pub devices: Vec<Device>,
        /// Shadow copies, empty on platforms without them.
        pub shadow_copies: Vec<ShadowCopy>,
    }
}

dto! {
    /// Which recovery stages a scan runs, and what it annotates.
    pub struct ScanRequest {
        /// Raw image file or block device to scan.
        pub source: String,
        /// Directory receiving recovered files and the manifest.
        pub out: String,
        /// Worker threads; the machine's available parallelism when absent.
        pub jobs: Option<u32>,
        /// Recover from filesystem metadata.
        pub filesystem: bool,
        /// Carve the full surface.
        pub carving: bool,
        /// Reassemble images the medium stored in pieces.
        pub reassembly: bool,
        /// Label artifacts photograph vs synthetic asset.
        pub triage: bool,
        /// Smallest long side, in pixels, an image is written to disk for.
        /// Absent takes the engine's default; zero writes everything.
        /// Whatever is not written is still examined, hashed and recorded
        /// with its extents and its dimensions.
        #[serde(default)]
        pub min_long_side: Option<u32>,
        /// Render a preview of every artifact that decodes.
        pub previews: bool,
        /// How long reassembly may search, in seconds. Absent takes the
        /// engine's own budget; zero searches every candidate however long it
        /// takes.
        ///
        /// Reassembly is the one stage that reaches a ceiling and stops without
        /// finishing, and the report says so rather than implying the medium
        /// held nothing more. A client that cannot set this cannot ask for the
        /// longer search that ceiling exists to bound.
        #[serde(default)]
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub reassembly_budget_seconds: Option<u64>,
        /// Session directory whose fragmentation points to search again,
        /// instead of sweeping the medium for them.
        ///
        /// A scan of a large disk spends its hours on the sweep and the
        /// validation pass, and both establish the same fragmentation points
        /// every time — the manifest records them. Trying a longer budget from
        /// those costs minutes rather than another overnight run. The medium is
        /// still read: every extent reported is fetched back and hashed exactly
        /// as a scan's is.
        #[serde(default)]
        pub resume_from: Option<String>,
    }
}

impl Default for ScanRequest {
    /// The defaults of `argos scan`: every stage, triage on, previews off.
    fn default() -> Self {
        Self {
            source: String::new(),
            out: String::new(),
            jobs: None,
            filesystem: true,
            carving: true,
            reassembly: true,
            triage: true,
            min_long_side: None,
            previews: false,
            reassembly_budget_seconds: None,
            resume_from: None,
        }
    }
}

dto! {
    /// What a started scan tells its client before it produces anything.
    pub struct ScanStarted {
        /// One line naming the source, as recorded in the manifest.
        pub source: String,
        /// Directory the session is being written into.
        pub out: String,
        /// Subdirectory of `out` holding previews. A viewer is given access to
        /// this and nothing else of the session.
        pub preview_dir: String,
    }
}

dto! {
    /// What a started acquisition tells its client before it copies anything.
    pub struct AcquireStarted {
        /// One line naming the medium being copied.
        pub source: String,
        /// Path of the raw image being written.
        pub to: String,
        /// Sectors the medium holds, so a client has a denominator from the
        /// first frame.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub sectors: u64,
    }
}

dto! {
    /// How far an acquisition has got.
    ///
    /// Counted in sectors rather than bytes because that is the unit a medium
    /// fails in: the sweep skips a failing region and the refinement revisits
    /// it one sector at a time, and both are reported against the same
    /// denominator.
    pub struct AcquireProgress {
        /// `sweep` for the sequential pass, `refine` for the sector-by-sector
        /// revisit of what the sweep skipped.
        pub pass: String,
        /// Sectors this pass has covered.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub done: u64,
        /// Sectors this pass has to cover.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub total: u64,
    }
}

dto! {
    /// What an acquisition produced.
    ///
    /// `recovered` is never presented as the whole medium when it is not:
    /// whatever stayed unreadable is zero-filled in the image and counted here,
    /// because those zeroes are placeholders and a client that showed them as
    /// data would be reporting bytes that were never read
    /// (`A-CONFIDENCE-HONEST`).
    pub struct Acquired {
        /// Path of the image that was written.
        pub image: String,
        /// Sectors the medium holds.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub sectors: u64,
        /// Sectors actually read off the medium and into the image.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub recovered: u64,
        /// Runs of sectors that stayed unreadable after both passes.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub unreadable_regions: u64,
        /// Sectors the run never tried, because it was stopped before reaching
        /// them.
        ///
        /// Never folded into `unreadable_regions`: that is what the *medium*
        /// refused, and a run its operator stopped says nothing about the
        /// medium. Merging the two would turn a cancelled copy into a report of
        /// a damaged disk (`A-CONFIDENCE-HONEST`).
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub not_attempted: u64,
        /// Whether the run was stopped before it covered the medium.
        pub stopped_early: bool,
        /// Whether every sector was read.
        pub complete: bool,
    }
}

dto! {
    /// A byte range, absolute in the medium.
    pub struct Extent {
        /// Offset of the range's first byte.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub offset: u64,
        /// Range length in bytes.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub length: u64,
    }
}

dto! {
    /// One recovered artifact, as a results view needs it.
    ///
    /// Every field is copied from the manifest. A client that recomputes any
    /// of them is doing recovery work in a presentation layer
    /// (`A-SHELL-NO-DOMAIN`).
    pub struct Artifact {
        /// File name inside the session directory, absent when the run was
        /// told to leave this artifact unwritten. Its extents and digest are
        /// recorded either way.
        pub name: Option<String>,
        /// SHA-256 of the artifact bytes, lowercase hex.
        pub sha256: String,
        /// Recovery stage that produced it.
        pub stage: String,
        /// Image format it validated as.
        pub format: String,
        /// Evidence tier, as its canonical display name.
        pub confidence: String,
        /// What was actually recovered and stored, in bytes.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub length: u64,
        /// Length the source metadata claimed, when it said one.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub expected_length: Option<u64>,
        /// Bytes the metadata expected that were not recovered.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub missing_bytes: Option<u64>,
        /// Every source extent it was assembled from, in file order.
        pub extents: Vec<Extent>,
        /// Creation time from metadata, seconds since the Unix epoch.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub created_unix: Option<i64>,
        /// Modification time from metadata, seconds since the Unix epoch.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub modified_unix: Option<i64>,
        /// Name recovered from filesystem metadata, when one survived.
        pub recovered_name: Option<String>,
        /// Where the artifact stands in a list, by the evidence it carries:
        /// `camera-named`, `dated`, `photograph-sized`, `unremarkable` or
        /// `cache-neighbour`. A sort key the engine derived; nothing is hidden
        /// by it and a client must not recompute it (`A-SHELL-NO-DOMAIN`).
        pub standing: Option<String>,
        /// Decoded pixel width, when the artifact decoded.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub width: Option<u32>,
        /// Decoded pixel height, when the artifact decoded.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub height: Option<u32>,
        /// Camera that took the picture, as recorded: make and model joined.
        pub camera: Option<String>,
        /// When the picture was taken, as EXIF stores it.
        pub taken: Option<String>,
        /// How many same-sized neighbours it was found among, when it sat in a
        /// run of them — the layout a thumbnail cache has.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        pub same_size_neighbours: Option<u32>,
        /// Triage label, when it was scored.
        pub triage_label: Option<String>,
        /// The property that settled the label, when there is one.
        pub triage_decided_by: Option<String>,
        /// SHA-256 of the artifact this one is a near-duplicate of. Both stay.
        pub near_duplicate_of: Option<String>,
        /// Preview path relative to the session directory, when one exists.
        pub preview: Option<String>,
    }
}

dto! {
    /// How triage ran over a session.
    pub struct Triage {
        /// `scored` when a model ran, `disabled` when it did not.
        pub status: String,
        /// Why triage did not run, when it did not.
        pub disabled_reason: Option<String>,
        /// Version of the model that scored the session.
        pub model_version: Option<String>,

        /// Artifacts that received a score.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub scored: u64,
        /// Artifacts triage saw but could not score.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub unscored: u64,
        /// Whether the classifier failed mid-run.
        pub degraded: bool,
    }
}

dto! {
    /// Everything a session directory holds, read back from its manifest.
    pub struct Results {
        /// Version of the tool that produced the session.
        pub tool_version: String,
        /// Description of the scanned source.
        pub source: String,
        /// How the run ended: `finished`, `cancelled` or `failed`.
        pub state: String,
        /// Signature hits that failed validation and were not recovered.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub rejected_candidates: u64,
        /// Byte ranges the medium could not read.
        pub unreadable: Vec<Extent>,
        /// How triage ran, when the session says.
        pub triage: Option<Triage>,
        /// One entry per recovered artifact, in manifest order.
        pub artifacts: Vec<Artifact>,
    }
}

dto! {
    /// One page of a session's artifacts, strongest evidence first.
    pub struct Gallery {
        /// Artifacts on this page, already ordered by the engine.
        pub artifacts: Vec<Artifact>,
        /// Artifacts the filter admits across the whole session, so a client
        /// can page without asking twice.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub total: u32,
        /// Artifacts the session recorded in all, whatever the filter.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub recorded: u32,
        /// Subdirectory holding previews, relative to the session.
        pub preview_dir: String,
    }
}

dto! {
    /// The account of a finished run, in the figures a display shows.
    ///
    /// Deliberately not [`Results`]. A run over a system disk recovers tens of
    /// thousands of artifacts, and sending every record so a client can count
    /// them is megabytes of JSON to produce three numbers — serialized here,
    /// parsed there, and re-materialized as objects on whatever thread draws
    /// the window. That is a client stopped for seconds at the exact moment
    /// the scan succeeds.
    ///
    /// A client that wants the records asks for them with `scan.results`, once
    /// and on purpose. The engine counts; the client is told.
    pub struct Summary {
        /// How the run ended: `finished`, `cancelled` or `failed`.
        pub state: String,
        /// Description of the scanned source.
        pub source: String,
        /// Artifacts written to the output directory.
        ///
        /// Never the size of the manifest: a run asked to leave synthetic
        /// assets unwritten records every one of them, and counting those here
        /// would report as recovered what is not in the directory
        /// (`A-CONFIDENCE-HONEST`). They are counted by
        /// [`Summary::omitted`] instead.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub artifacts: u64,
        /// Their total length in bytes.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub bytes: u64,
        /// Artifacts recorded in the manifest but deliberately not written.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub omitted: u64,
        /// Signature hits that failed validation and were not recovered.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub rejected_candidates: u64,
        /// Regions the medium could not read, and their total length.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub unreadable_regions: u64,
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub unreadable_bytes: u64,
        /// The session directory, which is where the records are.
        pub session: String,
        /// How triage ran, when the session says.
        pub triage: Option<Triage>,
    }
}

dto! {
    /// What an export produced.
    pub struct Exported {
        /// Artifacts copied and verified.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub copied: u64,
        /// Preview files copied alongside them.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub previews: u64,
        /// Artifacts whose stored bytes no longer reproduce the manifest's
        /// digest, and were therefore not copied.
        pub tampered: Vec<String>,
        /// Artifacts recorded in the manifest whose file is missing.
        pub missing: Vec<String>,
    }
}

dto! {
    /// What a scan is doing, emitted per chunk of work rather than per
    /// candidate (`A-EVENTS-NOT-POLLING`).
    ///
    /// `done` and `total` are counted in `unit`. A stage that reads the medium
    /// counts bytes; one that examines candidates or labels artifacts counts
    /// those, and saying which is what stops a candidate count being read as a
    /// byte count.
    pub struct Progress {
        /// Stage reporting progress.
        pub stage: String,
        /// `bytes` or `items`.
        pub unit: String,
        /// Work processed so far.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub done: u64,
        /// Work the stage expects to cover, zero when not known ahead.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub total: u64,
    }
}

dto! {
    /// A stage began.
    ///
    /// Sent for every stage, including one that will report no progress at
    /// all, because a client that hears nothing between two stages cannot tell
    /// work from a stall.
    pub struct StageBegan {
        /// Stage that began.
        pub stage: String,
        /// `bytes` or `items`.
        pub unit: String,
        /// Work the stage expects to cover, zero when not known ahead.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub total: u64,
    }
}

dto! {
    /// What a scan has actually recovered so far, as of one stored artifact.
    ///
    /// Both figures describe artifacts **stored** — never candidates seen. A
    /// signature hit that has not passed its format's state machine is not a
    /// recovery, and a display that counted one would overstate the result
    /// (`A-CONFIDENCE-HONEST`).
    pub struct Stored {
        /// Artifacts handed to the sink so far.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub artifacts: u64,
        /// Sum of their lengths in bytes.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub bytes: u64,
    }
}

dto! {
    /// How much of the medium refused to be read, as a running total.
    ///
    /// A count rather than one message per region: a failing disk produces
    /// them faster than any client can draw them, and what a client needs is
    /// how much was lost. The regions themselves are in the manifest, which is
    /// where a record of damage belongs.
    pub struct Unreadable {
        /// Regions the medium refused so far.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub regions: u64,
        /// Their total length in bytes.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub bytes: u64,
    }
}

dto! {
    /// A stage ended, having produced this many results.
    pub struct StageDone {
        /// Stage that ended.
        pub stage: String,
        /// Findings the stage contributed.
        #[cfg_attr(feature = "bindings", ts(type = "number"))]
        pub findings: u64,
    }
}

dto! {
    /// A run changed lifecycle state.
    pub struct State {
        /// `running`, `paused`, `cancelled` or `finished`.
        pub state: String,
    }
}

dto! {
    /// Something the user should know that is not progress: a mounted medium,
    /// a partition-only scan, a disabled classifier.
    pub struct Warning {
        /// The message, already written for a person to read.
        pub text: String,
    }
}
