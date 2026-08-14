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
}

impl Engine {
    fn new() -> Self {
        Self {
            out: Arc::new(Wire::new()),
            trace: trace::enabled().then(|| Arc::new(trace::Trace::new())),
            session: Arc::new(Mutex::new(None)),
            ready: Mutex::new(false),
            worker: Mutex::new(None),
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
                let floor = standing
                    .as_deref()
                    .map(str::parse::<argos_classify::rank::Standing>)
                    .transpose()
                    .map_err(|_unknown| {
                        (
                            ErrorCode::InvalidParams,
                            "not a standing: expected one of cache-neighbour, unremarkable, \
                             photograph-sized, dated, camera-named"
                                .to_owned(),
                        )
                    })?;
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
            Call::ExportCopy {
                session,
                to,
                hashes,
            } => crate::export::run(
                session.as_ref(),
                to.as_ref(),
                // The wire carries hash selection only; the photograph
                // criteria are a schema change (`A-DTO-VERSIONED`).
                &crate::export::Filter {
                    hashes,
                    ..crate::export::Filter::default()
                },
            )
            .map(|exported| Reply::Exported(translate::exported(&exported)))
            .map_err(|err| (ErrorCode::InvalidParams, format!("{err:#}"))),
        }
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
        let options = translate::options(request);
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
