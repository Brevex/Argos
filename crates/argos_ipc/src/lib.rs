//! The wire format between Argos' engine process and its clients.
//!
//! Raw device access needs root or Administrator, and a graphical application
//! should not run elevated. So the engine is a *process*: `argos --serve`
//! speaks JSON-RPC over a pipe, and anything that wants a recovery — the Tauri
//! shell, a script, a future client nobody has written — is a client of that
//! one interface.
//!
//! That arrangement is what makes "the shell is only a shell" structural
//! rather than a rule someone has to keep remembering. The shell cannot
//! contain recovery logic, because it does not link the engine; it links this
//! crate, and this crate depends on nothing in the workspace
//! (`A-SHELL-NO-DOMAIN`).
//!
//! # The three things this format guarantees
//!
//! - **Versioned.** [`SCHEMA_VERSION`] travels in the handshake, which must be
//!   the first call on a connection. A mismatch fails loudly at connect time
//!   rather than as a field that quietly turns up missing later
//!   (`A-DTO-VERSIONED`).
//! - **Opaque-free.** No type here re-exports an engine type, and none can:
//!   this crate cannot name one. What a client receives is a flat record of
//!   strings and numbers it can display and nothing else.
//! - **No content.** No message carries recovered bytes. Artifacts and
//!   previews stay files in the session directory, reached through a scope the
//!   client is granted — never base64 through the pipe.
//!
//! # Shape of a session
//!
//! ```text
//! client → {"jsonrpc":"2.0","id":1,"method":"handshake","params":{"schema":1}}
//! engine → {"jsonrpc":"2.0","id":1,"result":{"schema":1,"toolVersion":"0.1.0"}}
//! client → {"jsonrpc":"2.0","id":2,"method":"scan.start","params":{…}}
//! engine → {"jsonrpc":"2.0","id":2,"result":{"source":"…","out":"…",…}}
//! engine → {"method":"progress","params":{"stage":"carve","bytesDone":…}}
//! engine → {"method":"finished","params":{…}}
//! ```
//!
//! Progress is pushed. No method fetches it (`A-EVENTS-NOT-POLLING`).

pub mod dto;
pub mod wire;

/// Version of this wire format.
///
/// Bumped whenever a client that understood the previous version could
/// misread this one: a field removed or renamed, a meaning changed. Adding an
/// optional field does not bump it, because a client that ignores the field
/// still reads everything else correctly.
pub const SCHEMA_VERSION: u32 = 10;

#[cfg(test)]
mod tests {
    use super::SCHEMA_VERSION;
    use crate::dto;
    use crate::wire::{Call, Request};

    #[test]
    fn a_scan_request_defaults_to_what_the_command_line_defaults_to() {
        // The UI and `argos scan` must recover the same thing from the same
        // medium. That starts with the two agreeing on what "no options
        // given" means (`A-CLI-FIRST`).
        let request = dto::ScanRequest::default();
        assert!(request.filesystem, "every stage runs by default");
        assert!(request.carving);
        assert!(request.reassembly);
        assert!(request.triage, "triage is on unless asked otherwise");
        assert!(
            !request.previews,
            "previews are derived files and stay opt-in"
        );
    }

    #[test]
    fn the_handshake_carries_the_version_this_crate_defines() {
        let text = serde_json::to_string(&Request::new(
            1,
            Call::Handshake {
                schema: SCHEMA_VERSION,
            },
        ))
        .expect("serialize");
        assert!(
            text.contains(&format!(r#""schema":{SCHEMA_VERSION}"#)),
            "{text}"
        );
    }
}
