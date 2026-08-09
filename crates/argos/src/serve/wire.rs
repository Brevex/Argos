//! Stdout, which in this mode is the protocol.
//!
//! In `--serve` every byte on stdout is a JSON-RPC message, so this is the
//! only thing allowed to write there and `println!` is a bug anywhere in the
//! serve path. The lock exists because a running scan emits progress from the
//! pipeline's threads while the dispatch loop answers calls on its own.

use std::io::{BufWriter, Stdout, Write};
use std::sync::Mutex;

use argos_ipc::wire;
use serde::Serialize;

/// The outbound half of a connection.
#[derive(Debug)]
pub struct Wire {
    out: Mutex<BufWriter<Stdout>>,
}

impl Wire {
    /// Wraps this process' stdout.
    #[must_use]
    pub fn new() -> Self {
        Self {
            out: Mutex::new(BufWriter::new(std::io::stdout())),
        }
    }

    /// Writes one message and flushes it.
    ///
    /// Flushed per message because the client is waiting on it: a progress
    /// event still sitting in a buffer is a user interface that has stopped
    /// moving. Events are emitted per chunk of work, never per candidate, so
    /// this is not a hot path (`M-LOG-OVERHEAD`).
    ///
    /// Failures are dropped on purpose. A broken pipe means the client is
    /// gone, and the dispatch loop learns that from end-of-input on stdin a
    /// moment later; there is nowhere to report a write failure to when the
    /// thing being written to is the report channel.
    pub fn send<T: Serialize>(&self, message: &T) {
        let Ok(text) = wire::line(message) else {
            return;
        };
        let mut out = self
            .out
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

impl Default for Wire {
    fn default() -> Self {
        Self::new()
    }
}
