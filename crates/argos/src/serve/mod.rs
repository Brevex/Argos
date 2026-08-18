//! `argos --serve`: the engine as a process, speaking JSON-RPC over stdio.
//!
//! Raw device access needs root or Administrator, and a graphical application
//! should not run elevated. So a client spawns this — elevated when it must
//! be — and talks to it over a pipe. The client stays unprivileged and holds
//! no recovery logic, because it does not link the engine at all.
//!
//! Everything this mode can do, `argos` can do from a shell. That is not a
//! coincidence: both drive [`crate::scan::run`], so a capability cannot exist
//! here and be missing from the command line (`A-CLI-FIRST`).
//!
//! **Stdout is the protocol.** Nothing in this module or anything it calls may
//! print; the scan driver was separated from the console output for exactly
//! this reason. Diagnostics, if any, go to stderr.

mod pace;
mod trace;
mod translate;
mod wire;

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use argos_core::progress::{ProgressSink, ScanEvent};
use argos_ipc::wire::{Call, Done, ErrorCode, Notification, Reply, Request, Response};
use argos_ipc::{SCHEMA_VERSION, dto};
use argos_report::Manifest;

use crate::scan;
use wire::Wire;

/// Just enough of a request to answer it when the rest does not parse.
#[derive(serde::Deserialize)]
struct Envelope {
    id: Option<u64>,
}

/// Reads requests from stdin and answers them until the client goes away.
///
/// Never fails: a malformed line, an unknown method or a scan that could not
/// start is an error *response*, because the connection outlives any one call.
/// The loop ends when stdin reaches end of input, which is what a client
/// closing the pipe or exiting looks like.
pub fn run() {
    let engine = Engine::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            // The pipe broke mid-read. There is no one left to tell.
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        engine.dispatch(&line);
    }
    // A client that disappears mid-scan leaves a scan running. Stop it and
    // *wait for it*: cancelling still writes the manifest, and exiting before
    // it does would leave artifacts on disk with nothing describing them —
    // bytes no one can attribute to a sector (A-PROVENANCE). Carrying the scan
    // on to completion instead would burn a disk's worth of I/O for nobody.
    engine.cancel();
    engine.finish();
    if let Some(trace) = engine.trace.as_ref() {
        trace.summary();
    }
}

/// One connection's state.
struct Engine {
    out: Arc<Wire>,
    /// Where the session spent its time, when asked for. Absent is the normal
    /// case and costs one `Option` check per event.
    trace: Option<Arc<trace::Trace>>,
    /// The running scan, when there is one. A session is cheap to clone and
    /// shares the run, so this is how `scan.pause` and `scan.cancel` reach a
    /// scan that is blocking a worker thread (`M-SERVICES-CLONE`).
    session: Arc<Mutex<Option<argos_engine::ScanSession>>>,
    /// Whether the handshake has happened. Every other call is refused until
    /// it has, so a version mismatch is found before anything reads a medium.
    ready: Mutex<bool>,
    /// The thread running the scan, kept so shutdown can wait for it.
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Whether an acquisition is under way. An acquisition holds the medium
    /// exactly as a scan does, and this process reads one medium at a time.
    acquiring: Arc<Mutex<bool>>,
    /// Raised to stop the running acquisition. An acquisition has no session to
    /// cancel through, so the flag is the whole mechanism.
    stop_acquire: Arc<std::sync::atomic::AtomicBool>,
}

impl Engine {
    fn new() -> Self {
        Self {
            out: Arc::new(Wire::new()),
            trace: trace::enabled().then(|| Arc::new(trace::Trace::new())),
            session: Arc::new(Mutex::new(None)),
            ready: Mutex::new(false),
            worker: Mutex::new(None),
            acquiring: Arc::new(Mutex::new(false)),
            stop_acquire: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Parses one line and answers it.
    ///
    /// The id is read in a pass of its own, before the call is. A client
    /// blocks on the id it sent, so a request this engine cannot understand —
    /// a method from a newer version, a parameter of the wrong shape — has to
    /// come back carrying it. An uncorrelated error would leave that client
    /// waiting forever for an answer it already received.
    fn dispatch(&self, line: &str) {
        let id = serde_json::from_str::<Envelope>(line)
            .ok()
            .and_then(|envelope| envelope.id);

        let request: Request = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(err) => {
                let code = if id.is_some() {
                    // The envelope was fine, so this is a request this version
                    // does not have rather than a broken line.
                    ErrorCode::InvalidRequest
                } else {
                    ErrorCode::Parse
                };
                self.out.send(&Response::failed(id, code, err.to_string()));
                return;
            }
        };

        if !matches!(request.call, Call::Handshake { .. }) && !self.is_ready() {
            self.out.send(&Response::failed(
                id,
                ErrorCode::NotReady,
                "handshake first: the wire format has to be agreed before anything reads a medium",
            ));
            return;
        }

        match self.answer(request.call) {
            Ok(reply) => self.out.send(&Response::ok(id, reply)),
            Err((code, message)) => self.out.send(&Response::failed(id, code, message)),
        }
    }

    /// Runs one call.
    fn answer(&self, call: Call) -> Result<Reply, (ErrorCode, String)> {
        match call {
            Call::Handshake { schema } => self.handshake(schema),
            Call::DevicesList => Ok(Reply::Inventory(Box::new(translate::inventory()))),
            Call::ScanStart(request) => self.scan_start(&request).map(Reply::Started),
            Call::ScanPause => {
                self.with_session(argos_engine::ScanSession::pause);
                Ok(Reply::Done(Done::new()))
            }
            Call::ScanResume => {
                self.with_session(argos_engine::ScanSession::resume);
                Ok(Reply::Done(Done::new()))
            }
            Call::ScanCancel => {
                // One call for both jobs, because this process reads one medium
                // at a time and a client that has to know which kind of read it
                // is stopping is a client keeping track of the engine's state
                // for it.
                self.stop_acquire
                    .store(true, std::sync::atomic::Ordering::Release);
                self.cancel();
                Ok(Reply::Done(Done::new()))
            }
            Call::ScanResults { session } => Manifest::read(&session)
                .map(|manifest| Reply::Results(Box::new(translate::results(&manifest))))
                .map_err(|err| (ErrorCode::InvalidParams, err.to_string())),
            Call::ScanGallery {
                session,
                offset,
                limit,
                standing,
                include_unwritten,
            } => {
                let floor = Self::parse_standing(standing.as_deref())?;
                Manifest::read(&session)
                    .map(|manifest| {
                        Reply::Page(Box::new(translate::gallery(
                            &manifest,
                            offset,
                            limit,
                            floor,
                            include_unwritten,
                        )))
                    })
                    .map_err(|err| (ErrorCode::InvalidParams, err.to_string()))
            }
            Call::AcquireStart { source, to } => {
                self.acquire_start(&source, &to).map(Reply::Acquiring)
            }
            Call::ExportCopy {
                session,
                to,
                hashes,
                standing,
            } => {
                let standing = Self::parse_standing(standing.as_deref())?;
                crate::export::run(
                    session.as_ref(),
                    to.as_ref(),
                    // The remaining criteria — a pixel floor, a camera, a date
                    // range — stay on the command line: they are queries a
                    // person writes, and no client asks for them yet.
                    &crate::export::Filter {
                        hashes,
                        standing,
                        ..crate::export::Filter::default()
                    },
                )
                .map(|exported| Reply::Exported(translate::exported(&exported)))
                .map_err(|err| (ErrorCode::InvalidParams, format!("{err:#}")))
            }
        }
    }

    /// Copies a medium into a raw image, reporting each pass as it goes.
    ///
    /// The reply waits for the first progress report, because that is the first
    /// moment the medium's size is known and a client needs a denominator
    /// before it can draw anything. A failure before then — the image already
    /// exists, the destination is a device, the source will not open — is the
    /// reply instead, so the client is told by the call it made rather than by
    /// a notification it has to correlate.
    fn acquire_start(
        &self,
        source: &str,
        to: &str,
    ) -> Result<dto::AcquireStarted, (ErrorCode, String)> {
        if self.lock_session().is_some() || self.is_acquiring() {
            return Err((
                ErrorCode::Busy,
                "this engine process is already reading a medium".to_owned(),
            ));
        }
        *self
            .acquiring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        // Cleared here rather than at the end of the last run, so a stop that
        // arrived after one finished cannot cancel the next one before it
        // starts.
        self.stop_acquire
            .store(false, std::sync::atomic::Ordering::Release);

        let source = PathBuf::from(source);
        let to = PathBuf::from(to);
        // Taken before the paths move into the worker; both are what the caller
        // named, and the reply repeats them back.
        let described = source_line(&source);
        let written_to = to.display().to_string();
        let (sized_tx, sized_rx) = mpsc::sync_channel::<Result<u64, String>>(1);
        let failed_tx = sized_tx.clone();

        let wire = Arc::clone(&self.out);
        let image = written_to.clone();
        let acquiring = Arc::clone(&self.acquiring);
        let stop = Arc::clone(&self.stop_acquire);
        let worker = std::thread::spawn(move || {
            let notice = Acquisition {
                out: Arc::clone(&wire),
                image,
                sized: Mutex::new(Some(sized_tx)),
            };
            let outcome = crate::acquire::run(&source, &to, &notice, &|| {
                stop.load(std::sync::atomic::Ordering::Acquire)
            });
            *acquiring
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
            if let Err(err) = outcome {
                let message = format!("{err:#}");
                if failed_tx.try_send(Err(message.clone())).is_err() {
                    wire.send(&Notification::Warning(dto::Warning { text: message }));
                    wire.send(&Notification::State(dto::State {
                        state: "failed".to_owned(),
                    }));
                }
            }
        });
        *self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(worker);

        match sized_rx.recv() {
            Ok(Ok(sectors)) => Ok(dto::AcquireStarted {
                source: described,
                to: written_to,
                sectors,
            }),
            Ok(Err(message)) => Err((ErrorCode::ScanFailed, message)),
            Err(_) => Err((
                ErrorCode::ScanFailed,
                "the acquisition stopped before it started".to_owned(),
            )),
        }
    }

    /// Whether an acquisition is running in this process.
    fn is_acquiring(&self) -> bool {
        *self
            .acquiring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Turns `options` into the ones a resumed search runs under.
    ///
    /// The sweep and the filesystem pass are what those points cost to find,
    /// and skipping them is the whole point; triage is off for the same reason
    /// `argos reassemble` leaves it off. Everything else the client asked for —
    /// the budget above all — is kept, because trying a longer one is why a
    /// search is run again.
    ///
    /// # Errors
    ///
    /// Fails when the session cannot be read, or when it records no
    /// fragmentation points: there would be nothing to search, and running a
    /// scan that found nothing would look like an answer about the medium.
    fn resume_options(
        options: &scan::Options,
        session: &str,
    ) -> Result<scan::Options, (ErrorCode, String)> {
        let manifest = Manifest::read(session).map_err(|err| {
            (
                ErrorCode::InvalidParams,
                format!("cannot read the session to resume from: {err}"),
            )
        })?;
        let broken = scan::fragmentation_points(&manifest);
        if broken.is_empty() {
            return Err((
                ErrorCode::InvalidParams,
                "that session records no fragmentation points; it was written by a scan that \
                 found none, or by a version of this tool that did not record them"
                    .to_owned(),
            ));
        }
        Ok(scan::Options {
            reference: None,
            jobs: options.jobs,
            stages: argos_engine::Stages {
                filesystem: false,
                carving: true,
                reassembly: true,
            },
            triage: false,
            min_long_side: options.min_long_side,
            reassembly_budget: options.reassembly_budget,
            previews: options.previews,
            // The points carry their own offsets; a range would only cut them.
            range: None,
            resume_from: Some(broken),
        })
    }

    /// One standing name from the wire, or `None` when the client sent none.
    ///
    /// One parser for both the gallery and the export, so the set a client is
    /// shown and the set it exports cannot be admitted on different terms.
    ///
    /// # Errors
    ///
    /// Fails when the name is not one the engine knows, naming the ones it does
    /// — a client one version ahead must be told, not silently given everything.
    fn parse_standing(
        standing: Option<&str>,
    ) -> Result<Option<argos_classify::rank::Standing>, (ErrorCode, String)> {
        standing
            .map(str::parse::<argos_classify::rank::Standing>)
            .transpose()
            .map_err(|_unknown| {
                (
                    ErrorCode::InvalidParams,
                    "not a standing: expected one of cache-neighbour, unremarkable, \
                     photograph-sized, dated, camera-named"
                        .to_owned(),
                )
            })
    }

    /// Agrees on the wire format, or refuses to speak.
    fn handshake(&self, schema: u32) -> Result<Reply, (ErrorCode, String)> {
        if schema != SCHEMA_VERSION {
            return Err((
                ErrorCode::SchemaMismatch,
                format!("this engine speaks schema {SCHEMA_VERSION}, the client speaks {schema}"),
            ));
        }
        *self.lock_ready() = true;
        Ok(Reply::Hello(dto::Hello {
            schema: SCHEMA_VERSION,
            tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        }))
    }

    /// Starts a scan on a worker thread and answers once it is under way.
    ///
    /// Answering early is deliberate: the dispatch loop has to stay free to
    /// receive `scan.pause` and `scan.cancel` while the scan runs, and a
    /// client has to be able to show something before the first chunk is read.
    /// What it waits for is the source being opened, because that is when the
    /// scan either has a medium or has failed to get one.
    fn scan_start(
        &self,
        request: &dto::ScanRequest,
    ) -> Result<dto::ScanStarted, (ErrorCode, String)> {
        if self.lock_session().is_some() {
            return Err((
                ErrorCode::Busy,
                "a scan is already running in this engine process".to_owned(),
            ));
        }

        let source = PathBuf::from(&request.source);
        let out = PathBuf::from(&request.out);
        let mut options = translate::options(request);
        if let Some(session) = request.resume_from.as_deref() {
            options = Self::resume_options(&options, session)?;
        }
        let (started_tx, started_rx) = mpsc::sync_channel::<Result<String, String>>(1);
        let failed_tx = started_tx.clone();

        let wire = Arc::clone(&self.out);
        let slot = Arc::clone(&self.session);
        let session_dir = out.clone();
        let trace = self.trace.clone();
        let worker = std::thread::spawn(move || {
            let pacer = pace::Pacer::start(Arc::clone(&wire), trace);
            let events = Events {
                pacer: Arc::clone(&pacer),
            };
            let notice = Notices {
                out: Arc::clone(&wire),
                pacer: Arc::clone(&pacer),
                description: Mutex::new(String::new()),
            };
            let outcome = scan::run(&source, &out, &options, &events, &notice, |session| {
                *slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session.clone());
                let _ = started_tx.try_send(Ok(notice.described()));
            });
            *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            // Stop the beat and send the last of what it was holding, so the
            // account below is not overtaken by a progress figure from before
            // it.
            pacer.finish();

            match outcome {
                // The scan ran. Whatever it ended as, the manifest describes
                // it, so the client is handed the same account any other
                // reader of that directory would get.
                Ok(_) => match Manifest::read(&session_dir) {
                    Ok(manifest) => wire.send(&Notification::Finished(Box::new(
                        translate::summary(&manifest, &session_dir),
                    ))),
                    Err(err) => wire.send(&Notification::Warning(dto::Warning {
                        text: err.to_string(),
                    })),
                },
                // It never got as far as a medium. If the client is already
                // waiting on the reply, this is the reply; otherwise it is a
                // warning and a terminal state.
                Err(err) => {
                    let message = format!("{err:#}");
                    if failed_tx.try_send(Err(message.clone())).is_err() {
                        wire.send(&Notification::Warning(dto::Warning { text: message }));
                        wire.send(&Notification::State(dto::State {
                            state: "failed".to_owned(),
                        }));
                    }
                }
            }
        });
        // Replacing the handle of a scan that already ended is fine; a running
        // one cannot be replaced, because `scan.start` refused above.
        *self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(worker);

        match started_rx.recv() {
            Ok(Ok(source)) => Ok(dto::ScanStarted {
                source,
                out: request.out.clone(),
                preview_dir: argos_report::PREVIEW_DIR.to_owned(),
            }),
            Ok(Err(message)) => Err((ErrorCode::ScanFailed, message)),
            // The worker thread died without saying anything, which means it
            // panicked. Report it rather than leaving the client waiting.
            Err(_) => Err((
                ErrorCode::ScanFailed,
                "the scan stopped before it started".to_owned(),
            )),
        }
    }

    /// Stops the running scan, if any.
    fn cancel(&self) {
        self.with_session(argos_engine::ScanSession::cancel);
    }

    /// Waits for the scan thread to finish writing its session directory.
    ///
    /// Called on shutdown, after [`cancel`](Engine::cancel). A scan stops at
    /// the next chunk boundary and then still hashes what it has, hands it to
    /// the sink and writes the manifest — so this waits rather than returning
    /// as soon as the flag is set.
    fn finish(&self) {
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(worker) = worker {
            // A panicked scan thread has already unwound and written whatever
            // it could; there is nothing left to report it to.
            let _ = worker.join();
        }
    }

    fn with_session(&self, act: impl FnOnce(&argos_engine::ScanSession)) {
        if let Some(session) = self.lock_session().as_ref() {
            act(session);
        }
    }

    fn lock_session(&self) -> std::sync::MutexGuard<'_, Option<argos_engine::ScanSession>> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_ready(&self) -> std::sync::MutexGuard<'_, bool> {
        self.ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn is_ready(&self) -> bool {
        *self.lock_ready()
    }
}

/// Turns the engine's progress port into notifications.
struct Events {
    pacer: Arc<pace::Pacer>,
}

impl ProgressSink for Events {
    fn emit(&self, event: ScanEvent) {
        self.pacer.emit(event);
    }
}

/// One line naming the medium an acquisition is reading.
///
/// The path and nothing else: this is a device node or a file the caller
/// already named, never a recovered file name (`A-NO-CONTENT-IN-LOGS`).
fn source_line(source: &std::path::Path) -> String {
    source.display().to_string()
}

/// Turns what an acquisition says into notifications.
///
/// No pacer: `argos_device::acquire` already caps what it reports to a couple
/// of hundred callbacks per pass, so every one of them can go straight out.
struct Acquisition {
    out: Arc<Wire>,
    /// Path of the image being written. The report says what was read, not
    /// where it went, so the caller's own answer is carried here.
    image: String,
    /// Answers the `acquire.start` call, once, with the medium's size.
    ///
    /// Taken on the first progress report: that is the first moment the size is
    /// known, and the client is waiting on it. `None` afterwards, so the reply
    /// is sent once however many reports follow.
    sized: Mutex<Option<mpsc::SyncSender<Result<u64, String>>>>,
}

impl crate::acquire::Notice for Acquisition {
    fn progress(&self, progress: argos_device::acquire::Progress) {
        use argos_device::acquire::Progress;
        let (pass, done, total) = match progress {
            Progress::Swept { done, total } => ("sweep", done, total),
            Progress::Refined { done, total } => ("refine", done, total),
        };
        if let Some(sized) = self
            .sized
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = sized.try_send(Ok(total));
        }
        self.out
            .send(&Notification::AcquireProgress(dto::AcquireProgress {
                pass: pass.to_owned(),
                done,
                total,
            }));
    }

    fn finished(&self, report: &argos_device::acquire::Report) {
        self.out.send(&Notification::Acquired(dto::Acquired {
            image: self.image.clone(),
            sectors: report.sector_count(),
            recovered: report.recovered_sectors(),
            unreadable_regions: report.unreadable().len() as u64,
            not_attempted: report.not_attempted(),
            stopped_early: report.stopped_early(),
            complete: report.is_complete(),
        }));
    }
}

/// Turns what a scan says about itself into notifications, and remembers the
/// source description for the `scan.start` reply.
struct Notices {
    out: Arc<Wire>,
    /// Flushed before every notice, so a warning never arrives ahead of the
    /// progress that was pending when it was raised.
    pacer: Arc<pace::Pacer>,
    description: Mutex<String>,
}

impl Notices {
    fn described(&self) -> String {
        self.description
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl scan::Notice for Notices {
    fn opened(&self, description: &str, _workers: usize) {
        description.clone_into(
            &mut self
                .description
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }

    fn reduced_expectation(&self) {
        self.pacer.flush();
        self.out.send(&Notification::Warning(dto::Warning {
            text: "this medium reports solid-state storage with TRIM; deleted content is often \
                   already gone from the host-visible surface before a scan begins"
                .to_owned(),
        }));
    }

    fn warning(&self, text: &str) {
        self.pacer.flush();
        self.out.send(&Notification::Warning(dto::Warning {
            text: text.to_owned(),
        }));
    }
}
