//! The client half of the connection to `argos --serve`.
//!
//! One child process per shell, spawned on demand, spoken to in JSON-RPC over
//! its pipes. A reader thread owns the child's stdout: responses go back to
//! whichever call is waiting on their id, and notifications become Tauri
//! events. Nothing polls (`A-EVENTS-NOT-POLLING`).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use argos_ipc::wire::{Call, Outcome, Reply, Request, Response};
use argos_ipc::{SCHEMA_VERSION, dto};
use tauri::{AppHandle, Emitter};

use crate::elevate;

/// Environment variable naming the engine binary, for development.
///
/// In a packaged application the engine sits beside the shell; running from a
/// checkout it does not, and hardcoding a `target/debug` path into shipped
/// code would be worse than reading a variable.
const ENGINE_PATH_VAR: &str = "ARGOS_ENGINE";

/// File name of the engine binary next to the shell.
#[cfg(windows)]
const ENGINE_NAME: &str = "argos.exe";
#[cfg(not(windows))]
const ENGINE_NAME: &str = "argos";

/// Tauri event carrying one engine notification to the frontend.
///
/// One channel for every kind, with the notification's own tag inside, so the
/// frontend subscribes once and switches on the payload rather than
/// registering a listener per message type.
pub const EVENT: &str = "argos://engine";

/// A connected engine process.
#[derive(Debug)]
pub struct Engine {
    child: Mutex<Child>,
    /// The engine's input, held only until the connection is dropped.
    ///
    /// `Option` so it can be *closed* rather than merely dropped with the rest
    /// of the struct: end of input is the engine's shutdown signal, and it has
    /// to reach the engine before anything kills it.
    input: Mutex<Option<ChildStdin>>,
    /// Calls waiting for their answer, by request id.
    waiting: Arc<Mutex<HashMap<u64, SyncSender<Response>>>>,
    next_id: AtomicU64,
}

impl Engine {
    /// Spawns the engine and completes the handshake.
    ///
    /// The child inherits this process' privileges, which are the ones a raw
    /// device needs: `crate::elevate` established them before any window was
    /// drawn. There is no unprivileged path to choose between.
    ///
    /// # Errors
    ///
    /// Fails when the binary cannot be found or spawned, when its pipes cannot
    /// be taken, or when the handshake does not agree on
    /// [`SCHEMA_VERSION`] — which is a hard stop, not a warning: two processes
    /// that disagree about the wire format cannot safely exchange anything.
    pub fn connect(app: &AppHandle) -> Result<Self, String> {
        let binary = locate()?;
        let mut command = elevate::engine(&binary);

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| format!("cannot start the recovery engine ({binary}): {err}"))?;
        let input = child.stdin.take().ok_or("the engine has no input pipe")?;
        let output = child.stdout.take().ok_or("the engine has no output pipe")?;

        let waiting: Arc<Mutex<HashMap<u64, SyncSender<Response>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let engine = Self {
            child: Mutex::new(child),
            input: Mutex::new(Some(input)),
            waiting: Arc::clone(&waiting),
            next_id: AtomicU64::new(0),
        };

        let app = app.clone();
        std::thread::spawn(move || read_loop(&app, BufReader::new(output), &waiting));

        match engine.call(Call::Handshake {
            schema: SCHEMA_VERSION,
        })? {
            Reply::Hello(hello) if hello.schema == SCHEMA_VERSION => Ok(engine),
            Reply::Hello(hello) => Err(format!(
                "this window speaks wire format {SCHEMA_VERSION} and the recovery engine speaks \
                 {}; they are different builds",
                hello.schema
            )),
            _ => Err("the engine answered a handshake with something else".to_owned()),
        }
    }

    /// Sends `call` and blocks until its answer arrives.
    ///
    /// # Errors
    ///
    /// Fails when the request cannot be written, when the engine goes away
    /// before answering, or when the engine answers with a JSON-RPC error —
    /// whose message is passed through unchanged, because the engine is the
    /// one that knows what went wrong.
    pub fn call(&self, call: Call) -> Result<Reply, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx): (SyncSender<Response>, Receiver<Response>) = mpsc::sync_channel(1);
        lock(&self.waiting).insert(id, tx);

        let sent = argos_ipc::wire::line(&Request::new(id, call))
            .map_err(|err| format!("cannot encode the request: {err}"))
            .and_then(|text| {
                let mut input = lock(&self.input);
                let input = input
                    .as_mut()
                    .ok_or_else(|| "the connection is closing".to_owned())?;
                input
                    .write_all(text.as_bytes())
                    .and_then(|()| input.flush())
                    .map_err(|err| format!("cannot reach the recovery engine: {err}"))
            });
        if let Err(err) = sent {
            lock(&self.waiting).remove(&id);
            return Err(err);
        }

        let response = rx
            .recv()
            .map_err(|_| "the recovery engine stopped before answering".to_owned())?;
        match response.outcome {
            Outcome::Result(reply) => Ok(*reply),
            Outcome::Error(failure) => Err(failure.message),
        }
    }
}

/// How long a closing connection waits for the engine to finish on its own.
///
/// A window closed mid-scan must not cost the manifest: artifacts already
/// written with nothing describing them are bytes no one can attribute to a
/// sector, which is the situation provenance exists to prevent
/// (`A-PROVENANCE`). Closing stdin makes the engine's dispatch loop end and
/// cancel the scan, and cancelling still writes the manifest — but that means
/// finishing the artifact already in flight and writing the file, so the wait
/// is generous rather than instant.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

/// How often the closing connection checks whether the engine has exited.
const SHUTDOWN_POLL: std::time::Duration = std::time::Duration::from_millis(25);

impl Drop for Engine {
    fn drop(&mut self) {
        // Close stdin first. That is the engine's shutdown signal: its dispatch
        // loop ends at end of input and cancels whatever is running, which
        // still leaves a complete session directory behind.
        drop(lock(&self.input).take());

        let mut child = lock(&self.child);
        let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
        while std::time::Instant::now() < deadline {
            match child.try_wait() {
                // Exited on its own, having written its manifest.
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(SHUTDOWN_POLL),
                // Cannot be waited on; killing is all that is left.
                Err(_) => break,
            }
        }
        // It did not stop. Killing is worse than letting it finish, and better
        // than leaving an elevated process reading a disk for a window that no
        // longer exists.
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Reads the engine's output until it ends.
///
/// Responses are handed to whoever is waiting on their id; notifications are
/// emitted to the frontend. A line that is neither is dropped: it came from a
/// version this one does not know, and inventing a meaning for it would be
/// worse than ignoring it.
fn read_loop(
    app: &AppHandle,
    output: BufReader<impl std::io::Read>,
    waiting: &Mutex<HashMap<u64, SyncSender<Response>>>,
) {
    let mut trace = crate::trace::Reader::new();
    for line in output.lines() {
        let Ok(line) = line else { break };
        if let Ok(response) = serde_json::from_str::<Response>(&line) {
            if let Some(id) = response.id
                && let Some(tx) = lock(waiting).remove(&id)
            {
                let _ = tx.send(response);
            }
            continue;
        }
        if let Ok(notification) = serde_json::from_str::<serde_json::Value>(&line)
            && notification.get("method").is_some()
        {
            let at = std::time::Instant::now();
            let _ = app.emit(EVENT, notification);
            trace.delivered(at.elapsed());
        }
    }
    trace.summary();
    // The engine is gone. Nothing else will answer these, and a call blocked
    // forever is a window that never responds again.
    lock(waiting).clear();
}

/// Where the engine binary is.
fn locate() -> Result<String, String> {
    if let Some(path) = std::env::var_os(ENGINE_PATH_VAR) {
        return Ok(path.to_string_lossy().into_owned());
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(ENGINE_NAME)))
        .filter(|path| path.is_file());
    beside
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| {
            format!(
                "the recovery engine ({ENGINE_NAME}) was not found next to this application; set \
             {ENGINE_PATH_VAR} to its path"
            )
        })
}

/// Locks a mutex, ignoring poisoning.
///
/// A panic in one call must not make the engine connection unusable for every
/// later one; the data behind these locks is a map and a pipe, neither of
/// which a panic can leave in a state the next caller would misread.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A reply the caller expected to be a specific shape.
///
/// The engine answers each method with one variant; anything else means the
/// two builds disagree, which is worth a message rather than a silent default.
pub fn expect<T>(reply: Reply, extract: impl FnOnce(Reply) -> Option<T>) -> Result<T, String> {
    extract(reply).ok_or_else(|| "the engine answered with an unexpected shape".to_owned())
}

/// Narrows a reply to an inventory.
#[must_use]
pub fn inventory(reply: Reply) -> Option<dto::Inventory> {
    match reply {
        Reply::Inventory(inventory) => Some(*inventory),
        _ => None,
    }
}

/// Narrows a reply to a started scan.
#[must_use]
pub fn started(reply: Reply) -> Option<dto::ScanStarted> {
    match reply {
        Reply::Started(started) => Some(started),
        _ => None,
    }
}

/// Narrows a reply to a started acquisition.
#[must_use]
pub fn acquiring(reply: Reply) -> Option<dto::AcquireStarted> {
    match reply {
        Reply::Acquiring(started) => Some(started),
        _ => None,
    }
}

/// Narrows a reply to a bare acknowledgement.
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "one signature for the whole family of narrowing functions, which \
              `Shell::call` takes as `fn(Reply) -> Option<T>`; this is the one \
              member with nothing to move out"
)]
pub fn done(reply: Reply) -> Option<()> {
    match reply {
        Reply::Done(_) => Some(()),
        _ => None,
    }
}
