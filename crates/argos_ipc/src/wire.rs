//! JSON-RPC 2.0 over a byte stream, one value per line.
//!
//! Line framing rather than a length prefix, because the transport is a pipe
//! to a child process and a human debugging it should be able to read the
//! traffic. `serde_json` never emits a bare newline inside a value, so a line
//! is exactly a message.
//!
//! Nothing here panics on input. The stream comes from another process — one
//! that may have been killed mid-write, or may not be `argos` at all — so a
//! malformed line is a [`Request`] that fails to parse and answers an error,
//! never an abort.

use serde::{Deserialize, Serialize};

use crate::dto;

/// The JSON-RPC version every message carries.
const JSONRPC: &str = "2.0";

/// One call from a client to the engine.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Correlates the response. A request without one is a notification and
    /// gets no answer.
    pub id: Option<u64>,
    /// The method, and its parameters.
    #[serde(flatten)]
    pub call: Call,
}

impl Request {
    /// A request for `call`, correlated by `id`.
    #[must_use]
    pub fn new(id: u64, call: Call) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id: Some(id),
            call,
        }
    }
}

/// Every method a client may call, with its parameters.
///
/// One enum rather than a free-form method string: an unknown method is a
/// deserialization failure at the edge, which is where a version mismatch
/// should be found.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum Call {
    /// Agree on the wire format. Must be the first call on a connection.
    Handshake {
        /// The wire format the client speaks.
        schema: u32,
    },
    /// List the media this machine exposes.
    #[serde(rename = "devices.list")]
    DevicesList,
    /// Start a scan. One at a time per engine process.
    #[serde(rename = "scan.start")]
    ScanStart(Box<dto::ScanRequest>),
    /// Suspend the running scan at the next chunk boundary.
    #[serde(rename = "scan.pause")]
    ScanPause,
    /// Resume a paused scan.
    #[serde(rename = "scan.resume")]
    ScanResume,
    /// Stop the running scan, keeping everything recovered so far.
    #[serde(rename = "scan.cancel")]
    ScanCancel,
    /// Read back what a session directory holds.
    ///
    /// The whole record set. A scan of a used disk records hundreds of
    /// thousands of artifacts, so a client that wants to *show* them asks for
    /// [`Call::ScanGallery`] instead and lets the engine do the ordering.
    #[serde(rename = "scan.results")]
    ScanResults {
        /// Session directory a scan wrote.
        session: String,
    },
    /// Read back one page of a session, strongest evidence first.
    ///
    /// The ordering is the engine's, not the client's: which artifact looks
    /// most like a photograph is a recovery question, and a window that
    /// answered it would be doing recovery work in a presentation layer
    /// (`A-SHELL-NO-DOMAIN`). The page is what makes a results view possible
    /// at all — the measured session holds 348,361 records, which is far more
    /// JSON than a window can be handed at once.
    #[serde(rename = "scan.gallery")]
    ScanGallery {
        /// Session directory a scan wrote.
        session: String,
        /// Artifacts to skip, for paging.
        #[serde(default)]
        offset: u32,
        /// Artifacts to return, capped by the engine.
        limit: u32,
        /// Weakest standing to include, by its canonical name. Absent shows
        /// every artifact the session recorded.
        #[serde(default)]
        standing: Option<String>,
        /// Whether to include artifacts the run recorded but did not write.
        /// They have no file and no preview, so a gallery hides them by
        /// default — the manifest still lists them.
        #[serde(default)]
        include_unwritten: bool,
    },
    /// Copy a medium into a raw image, so the scan can read a file instead of
    /// the disk.
    ///
    /// A scan reads the whole surface and every rerun reads it again; on a
    /// failing medium each pass is one it may not survive. One acquisition at a
    /// time per engine process, on the same terms as a scan.
    #[serde(rename = "acquire.start")]
    AcquireStart {
        /// Block device or image file to copy, opened read-only.
        source: String,
        /// Path of the raw image to create. Must not already exist.
        to: String,
    },
    /// Copy artifacts out of a session directory, verifying each hash.
    #[serde(rename = "export.copy")]
    ExportCopy {
        /// Session directory to export from.
        session: String,
        /// Directory to copy into.
        to: String,
        /// Artifact hashes to export; everything the other criteria admit when
        /// empty.
        #[serde(default)]
        hashes: Vec<String>,
        /// Weakest standing to export, by its canonical name. Absent exports
        /// whatever the other criteria admit.
        ///
        /// The same vocabulary [`Call::ScanGallery`] filters by, so a client
        /// can export exactly the set it is showing rather than asking the
        /// person to describe it a second time.
        #[serde(default)]
        standing: Option<String>,
    },
}

/// One answer from the engine to a client.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The request this answers.
    pub id: Option<u64>,
    /// What the call produced.
    #[serde(flatten)]
    pub outcome: Outcome,
}

impl Response {
    /// A successful answer to request `id`.
    #[must_use]
    pub fn ok(id: Option<u64>, reply: Reply) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id,
            outcome: Outcome::Result(Box::new(reply)),
        }
    }

    /// A failed answer to request `id`.
    #[must_use]
    pub fn failed(id: Option<u64>, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC.to_owned(),
            id,
            outcome: Outcome::Error(Failure {
                code: code as i32,
                message: message.into(),
            }),
        }
    }
}

/// A response is one or the other, never both.
///
/// Both variants are newtypes on purpose: this enum is flattened into
/// [`Response`], and a *struct* variant would nest its field under the variant
/// name — `{"error":{"error":{…}}}` rather than the `{"error":{…}}` JSON-RPC
/// specifies.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Outcome {
    /// The call succeeded.
    #[serde(rename = "result")]
    Result(Box<Reply>),
    /// The call failed.
    #[serde(rename = "error")]
    Error(Failure),
}

/// What a successful call produced.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Reply {
    /// Answer to a handshake.
    Hello(dto::Hello),
    /// Answer to `devices.list`.
    Inventory(Box<dto::Inventory>),
    /// Answer to `scan.start`.
    Started(dto::ScanStarted),
    /// Answer to `scan.results`.
    Results(Box<dto::Results>),
    /// Answer to `scan.gallery`.
    Page(Box<dto::Gallery>),
    /// Answer to `acquire.start`.
    Acquiring(dto::AcquireStarted),
    /// Answer to `export.copy`.
    Exported(dto::Exported),
    /// Answer to a call that produces nothing but success.
    Done(Done),
}

/// The reply of a call with no value to return.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct Done {
    /// Always true; present so the reply is an object rather than `null`,
    /// which `untagged` cannot tell from a missing field.
    pub done: bool,
}

impl Done {
    /// The only value this type has.
    #[must_use]
    pub fn new() -> Self {
        Self { done: true }
    }
}

/// A failed call.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Failure {
    /// One of [`ErrorCode`].
    pub code: i32,
    /// What went wrong, for a person to read. Never carries recovered content
    /// or a recovered file name (`A-NO-CONTENT-IN-LOGS`).
    pub message: String,
}

/// Why a call failed.
///
/// The first three are JSON-RPC's own; the rest are Argos'. A client
/// distinguishes "you sent nonsense" from "the medium refused" without parsing
/// the message.
///
/// JSON-RPC's `-32601` is absent because this protocol cannot produce it: a
/// method is a [`Call`] variant, so an unknown one fails to deserialize and is
/// an [`ErrorCode::InvalidRequest`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ErrorCode {
    /// The line was not valid JSON.
    Parse = -32700,
    /// The JSON was not a request this version understands.
    InvalidRequest = -32600,
    /// The parameters were not what the method takes.
    InvalidParams = -32602,
    /// The client's schema version is not this engine's.
    SchemaMismatch = -32000,
    /// A call arrived before the handshake.
    NotReady = -32001,
    /// The scan itself failed — the medium, the output directory, the
    /// settings. The message says which.
    ScanFailed = -32002,
    /// A scan is already running in this engine process.
    Busy = -32003,
}

/// An unsolicited message from the engine to its client.
///
/// Progress is pushed, never polled: the engine already owns a progress port,
/// and a client that asked for progress would couple itself to the pipeline's
/// internals (`A-EVENTS-NOT-POLLING`).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum Notification {
    /// A stage began.
    StageBegan(dto::StageBegan),
    /// Cumulative progress within a stage.
    Progress(dto::Progress),
    /// A stage ended.
    StageDone(dto::StageDone),
    /// An artifact reached the output directory.
    Stored(dto::Stored),
    /// The run changed lifecycle state.
    State(dto::State),
    /// A region of the medium could not be read.
    Unreadable(dto::Unreadable),
    /// Something the user should know before trusting the result.
    Warning(dto::Warning),
    /// An acquisition covered more of the medium.
    AcquireProgress(dto::AcquireProgress),
    /// An acquisition ended; the image is on disk and this says what reached
    /// it.
    Acquired(dto::Acquired),
    /// The scan ended; its results are readable from the session directory.
    Finished(Box<dto::Summary>),
}

/// Serializes `value` as one line, newline included.
///
/// # Errors
///
/// Fails when the value cannot be serialized, which for these types means a
/// non-finite float reached a DTO.
pub fn line<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let mut text = serde_json::to_string(value)?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::{Call, ErrorCode, Reply, Request, Response, line};
    use crate::dto;

    #[test]
    fn a_request_round_trips_through_its_line() {
        let request = Request::new(7, Call::DevicesList);
        let text = line(&request).expect("serialize");
        assert!(text.ends_with('\n'), "a message is exactly one line");
        assert_eq!(text.matches('\n').count(), 1);

        let back: Request = serde_json::from_str(&text).expect("parse");
        assert_eq!(back.id, Some(7));
        assert!(matches!(back.call, Call::DevicesList));
    }

    #[test]
    fn the_method_name_is_the_one_written_in_the_protocol() {
        let text = line(&Request::new(1, Call::ScanPause)).expect("serialize");
        assert!(text.contains(r#""method":"scan.pause""#), "{text}");

        let text = line(&Request::new(2, Call::Handshake { schema: 3 })).expect("serialize");
        assert!(text.contains(r#""method":"handshake""#), "{text}");
        assert!(text.contains(r#""schema":3"#), "{text}");
    }

    #[test]
    fn an_unknown_method_fails_to_parse_rather_than_being_ignored() {
        // A client one version ahead calling something this engine does not
        // have must get an error, not silence.
        let text = r#"{"jsonrpc":"2.0","id":1,"method":"scan.teleport","params":{}}"#;
        serde_json::from_str::<Request>(text)
            .expect_err("an unknown method must not parse into a call");
    }

    #[test]
    fn a_failure_is_shaped_the_way_json_rpc_specifies() {
        // Asserted against the literal JSON rather than by round-tripping:
        // a wrong shape round-trips perfectly through its own wrongness, and
        // the client that has to read it is not this crate.
        let text = line(&Response::failed(
            Some(4),
            ErrorCode::SchemaMismatch,
            "engine speaks 1",
        ))
        .expect("serialize");
        let json: serde_json::Value = serde_json::from_str(&text).expect("parse");
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 4);
        assert_eq!(json["error"]["code"], ErrorCode::SchemaMismatch as i32);
        assert_eq!(json["error"]["message"], "engine speaks 1");
        assert!(
            json["error"]["error"].is_null(),
            "the failure must not be nested inside itself: {text}"
        );
        assert!(json["result"].is_null(), "a response is one or the other");
    }

    #[test]
    fn a_success_is_shaped_the_way_json_rpc_specifies() {
        let text = line(&Response::ok(
            Some(2),
            Reply::Hello(dto::Hello {
                schema: crate::SCHEMA_VERSION,
                tool_version: "0.1.0".to_owned(),
            }),
        ))
        .expect("serialize");
        let json: serde_json::Value = serde_json::from_str(&text).expect("parse");
        assert_eq!(json["result"]["schema"], crate::SCHEMA_VERSION);
        assert_eq!(json["result"]["toolVersion"], "0.1.0");
        assert!(json["error"].is_null());
    }

    #[test]
    fn a_reply_with_no_value_is_still_an_object() {
        // `untagged` resolves by shape. A unit reply serialized as `null`
        // would be indistinguishable from an absent field, and would then
        // parse as whichever variant happened to accept it first.
        let text =
            line(&Response::ok(Some(9), Reply::Done(super::Done::new()))).expect("serialize");
        assert!(text.contains(r#""done":true"#), "{text}");
        let back: Response = serde_json::from_str(&text).expect("parse");
        assert!(matches!(
            back.outcome,
            super::Outcome::Result(reply) if matches!(*reply, Reply::Done(_))
        ));
    }
}
