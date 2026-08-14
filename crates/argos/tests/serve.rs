//! `argos --serve` driven over pipes, the way a client drives it.
//!
//! The test that matters here is the parity one: a scan through the JSON-RPC
//! surface must recover exactly what `argos scan` recovers from the same
//! medium, byte for byte and record for record. That is `A-CLI-FIRST` made
//! checkable — a UI cannot grow a capability the command line lacks, because
//! both go through one driver and this test would notice if they stopped.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use argos_carve::fixture::{Disk, icon_png, photo_jpeg, png};

/// A client of one `argos --serve` process.
struct Engine {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    next_id: u64,
    /// Messages read while waiting for a response, kept in order.
    ///
    /// Responses and notifications share one stream, so waiting for an answer
    /// means reading past the progress a running scan is emitting. Those
    /// messages are the point of the interface — dropping them here would make
    /// the test blind to exactly what a client depends on, and would lose the
    /// `finished` notification a later assertion waits for.
    pending: VecDeque<serde_json::Value>,
}

impl Engine {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_argos"))
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn argos serve");
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            input,
            output,
            next_id: 0,
            pending: VecDeque::new(),
        }
    }

    /// Sends a call and returns the response correlated to it.
    ///
    /// Anything that arrives first — progress from a running scan, a warning —
    /// is buffered rather than dropped, because those messages are the point
    /// of the interface and a later assertion waits on them.
    fn call(&mut self, method: &str, params: &serde_json::Value) -> serde_json::Value {
        self.next_id += 1;
        let id = self.next_id;
        let request = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        });
        self.write(&request.to_string());

        // The answer may already be buffered from an earlier wait.
        if let Some(at) = self
            .pending
            .iter()
            .position(|message| message["id"] == serde_json::json!(id))
        {
            return self
                .pending
                .remove(at)
                .expect("a message at a position just found");
        }
        // Otherwise take lines off the stream, never off the buffer: reading
        // from the buffer here would pop a message, fail to match it, push it
        // back, and spin on it forever.
        loop {
            let message = self.read_line();
            if message["id"] == serde_json::json!(id) {
                return message;
            }
            self.pending.push_back(message);
        }
    }

    /// Writes one line to the engine.
    fn write(&mut self, line: &str) {
        let input = self.input.as_mut().expect("the connection is open");
        writeln!(input, "{line}").expect("write request");
        input.flush().expect("flush");
    }

    /// Closes the engine's input, which is how a client asks it to stop.
    fn close_input(&mut self) {
        self.input = None;
    }

    /// Reads one message, oldest buffered first.
    fn read(&mut self) -> serde_json::Value {
        self.pending.pop_front().unwrap_or_else(|| self.read_line())
    }

    /// Reads one message straight off the stream.
    fn read_line(&mut self) -> serde_json::Value {
        let mut line = String::new();
        let read = self.output.read_line(&mut line).expect("read a message");
        assert!(read > 0, "the engine closed the stream unexpectedly");
        serde_json::from_str(&line).unwrap_or_else(|err| panic!("not a message: {line}: {err}"))
    }

    /// Reads until the notification named `method` arrives, and returns it.
    ///
    /// Everything before it is discarded, which is what a caller waiting on a
    /// terminal message wants: the progress ahead of it is the point of the
    /// channel, not of the assertion.
    fn wait_for(&mut self, method: &str) -> serde_json::Value {
        loop {
            let message = self.read();
            if message["method"].as_str() == Some(method) {
                return message;
            }
        }
    }

    /// Reads messages until the scan reports it finished, collecting the
    /// method names seen on the way.
    fn drain_until_finished(&mut self) -> (Vec<String>, serde_json::Value) {
        let mut seen = Vec::new();
        loop {
            let message = self.read();
            let Some(method) = message["method"].as_str() else {
                continue;
            };
            seen.push(method.to_owned());
            if method == "finished" {
                return (seen, message["params"].clone());
            }
        }
    }

    fn handshake(&mut self) -> serde_json::Value {
        self.call(
            "handshake",
            &serde_json::json!({ "schema": argos_ipc::SCHEMA_VERSION }),
        )
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A fixture disk with one photograph and one PNG.
fn fixture(at: &Path) -> Vec<u8> {
    let disk = Disk::filled(1024 * 1024)
        .with(10_000, &photo_jpeg(256, 192, 0x5E44_0001))
        .with(600_000, &png(48, 32))
        .into_bytes();
    std::fs::write(at, &disk).expect("write fixture disk");
    disk
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn the_handshake_agrees_on_the_schema_before_anything_else_is_allowed() {
    let mut engine = Engine::spawn();

    // A call before the handshake is refused, and refused with a code rather
    // than a message a client would have to parse.
    let early = engine.call("devices.list", &serde_json::json!(null));
    assert_eq!(early["error"]["code"], -32001, "{early}");

    let hello = engine.handshake();
    assert_eq!(hello["result"]["schema"], argos_ipc::SCHEMA_VERSION);
    assert!(hello["result"]["toolVersion"].is_string());

    // And now it works.
    let devices = engine.call("devices.list", &serde_json::json!(null));
    assert!(devices["result"]["devices"].is_array(), "{devices}");
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_client_speaking_another_schema_is_turned_away_at_the_door() {
    let mut engine = Engine::spawn();
    let refused = engine.call("handshake", &serde_json::json!({ "schema": 99 }));
    assert_eq!(refused["error"]["code"], -32000, "{refused}");
    // The engine says what it speaks, so the mismatch is actionable.
    let message = refused["error"]["message"].as_str().expect("a message");
    assert!(
        message.contains(&format!("speaks schema {}", argos_ipc::SCHEMA_VERSION)),
        "{message}"
    );

    // Nothing is unlocked by a failed handshake.
    let after = engine.call("devices.list", &serde_json::json!(null));
    assert_eq!(after["error"]["code"], -32001, "{after}");
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_malformed_line_is_answered_rather_than_killing_the_connection() {
    let mut engine = Engine::spawn();
    engine.handshake();

    engine.write("{not json at all");
    let failure = engine.read();
    assert_eq!(failure["error"]["code"], -32700, "{failure}");

    // The connection survives it: input from another process is untrusted, and
    // a stream that dies on the first bad byte is unusable.
    let devices = engine.call("devices.list", &serde_json::json!(null));
    assert!(devices["result"]["devices"].is_array(), "{devices}");
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn an_unknown_method_is_refused_without_touching_a_medium() {
    let mut engine = Engine::spawn();
    engine.handshake();
    let refused = engine.call("scan.teleport", &serde_json::json!({}));
    // An unknown method does not parse into a call, so it is a bad request.
    assert!(refused["error"].is_object(), "{refused}");
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_scan_over_the_wire_recovers_exactly_what_the_command_line_does() {
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    fixture(&image);

    // The command line, first.
    let cli_out = dir.path().join("cli");
    let status = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&cli_out)
        .arg("--previews")
        // Both legs run the same settings, floor included: the point of this
        // test is that the wire and the command line recover the same thing.
        .args(["--min-long-side", "0"])
        .output()
        .expect("run argos scan");
    assert!(
        status.status.success(),
        "cli scan failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    // Then the same scan through the engine process.
    let rpc_out = dir.path().join("rpc");
    let mut engine = Engine::spawn();
    engine.handshake();
    let started = engine.call(
        "scan.start",
        &serde_json::json!({
            "source": image, "out": rpc_out,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": true, "minLongSide": 0, "previews": true,
        }),
    );
    assert!(started["result"]["source"].is_string(), "{started}");
    assert_eq!(started["result"]["previewDir"], "previews");

    let (methods, summary) = engine.drain_until_finished();
    assert!(
        methods.iter().any(|method| method == "progress"),
        "a client must be able to show progress without asking for it: {methods:?}"
    );
    // Every stage announces itself on the wire, not only the ones that go on
    // to report progress. A client that hears nothing between two stages has
    // no way to tell a long pass from a hung engine.
    assert!(
        methods.iter().any(|method| method == "stageBegan"),
        "no stage announced itself to the client: {methods:?}"
    );

    // What ends a run is a count, not the records. A client that only shows
    // figures is told the figures; one that wants the records asks for them.
    assert_eq!(summary["artifacts"], 2);
    assert!(summary["bytes"].as_u64().expect("a byte total") > 0);
    assert!(
        summary["artifacts"].is_u64(),
        "the account of a finished run is a count, not every record: a client that \
         shows figures would otherwise parse megabytes of them to derive one number: \
         {summary}"
    );

    // The two session directories describe the same recovery.
    let cli_manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(cli_out.join("manifest.json")).expect("manifest"))
            .expect("json");
    let cli_artifacts = cli_manifest["artifacts"].as_array().expect("artifacts");
    let read_back = engine.call("scan.results", &serde_json::json!({ "session": rpc_out }));
    let rpc_artifacts = read_back["result"]["artifacts"]
        .as_array()
        .expect("artifacts")
        .clone();
    let rpc_artifacts = &rpc_artifacts;
    assert_eq!(cli_artifacts.len(), 2);
    assert_eq!(rpc_artifacts.len(), cli_artifacts.len());

    // The manifest is snake_case on disk and the wire format is camelCase, so
    // the pairing is spelled out rather than assumed.
    let same = [
        ("sha256", "sha256"),
        ("length", "length"),
        ("confidence", "confidence"),
        ("stage", "stage"),
        ("extents", "extents"),
        ("triage_label", "triageLabel"),
        ("triage_photograph", "triagePhotograph"),
        ("preview", "preview"),
    ];
    for (cli, rpc) in cli_artifacts.iter().zip(rpc_artifacts) {
        for (on_disk, on_wire) in same {
            assert_eq!(
                cli[on_disk], rpc[on_wire],
                "{on_disk} differs between the command line and the wire"
            );
        }
    }
    // And the labels really were produced, so the comparison above is not two
    // absent fields agreeing with each other.
    assert!(
        cli_artifacts
            .iter()
            .any(|artifact| artifact["triage_label"].is_string()),
        "the fixture photograph should have been scored"
    );

    // And the bytes on disk are identical, which is the claim that actually
    // matters: the wire surface recovers evidence, not a description of it.
    for name in ["000000.jpg", "000001.png"] {
        assert_eq!(
            std::fs::read(cli_out.join(name)).expect("cli artifact"),
            std::fs::read(rpc_out.join(name)).expect("rpc artifact"),
            "{name} differs between the two paths"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn an_acquisition_over_the_wire_copies_the_medium_and_says_what_it_read() {
    // A disk worth recovering from is a disk worth reading exactly once. The
    // copy has to be bit-identical, and the account of it has to be the
    // engine's own — a client that counted sectors itself would be describing a
    // read it did not do.
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    let planted = fixture(&image);
    let copy = dir.path().join("copy.img");

    let mut engine = Engine::spawn();
    engine.handshake();
    let started = engine.call(
        "acquire.start",
        &serde_json::json!({ "source": image, "to": copy }),
    );
    let sectors = started["result"]["sectors"]
        .as_u64()
        .unwrap_or_else(|| panic!("the reply must name the medium's size: {started}"));
    assert_eq!(
        sectors,
        planted.len() as u64 / 512,
        "the size reported must be the medium's"
    );

    let acquired = engine.wait_for("acquired");
    assert_eq!(acquired["params"]["recovered"], sectors);
    assert_eq!(acquired["params"]["unreadableRegions"], 0);
    assert_eq!(acquired["params"]["complete"], true);

    assert_eq!(
        std::fs::read(&copy).expect("the acquired image"),
        planted,
        "an acquisition that is not bit-identical is not evidence"
    );

    // And the image it produced is a medium in its own right: scanning it must
    // reach the same artifacts as scanning the original.
    let session = dir.path().join("session");
    engine.call(
        "scan.start",
        &serde_json::json!({
            "source": copy, "out": session,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": false, "minLongSide": 0, "previews": false,
        }),
    );
    engine.drain_until_finished();
    let results = engine.call("scan.results", &serde_json::json!({ "session": session }));
    assert_eq!(
        results["result"]["artifacts"].as_array().map(Vec::len),
        Some(2),
        "the copy must recover what the original does: {results}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_cancelled_acquisition_reports_what_it_never_reached_as_untried_not_as_damage() {
    // The distinction the report exists for: a run its operator stopped says
    // nothing about the medium. If the sectors it never reached were counted as
    // unreadable, a cancelled copy would read as a failing disk.
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    fixture(&image);
    let copy = dir.path().join("copy.img");

    let mut engine = Engine::spawn();
    engine.handshake();
    engine.call(
        "acquire.start",
        &serde_json::json!({ "source": image, "to": copy }),
    );
    engine.call("scan.cancel", &serde_json::json!(null));

    let acquired = engine.wait_for("acquired");
    let params = &acquired["params"];
    // The fixture is small enough that the copy may well have finished before
    // the cancel landed; either outcome is correct, and both must be honest.
    if params["complete"] == true {
        assert_eq!(params["notAttempted"], 0);
        assert_eq!(params["stoppedEarly"], false);
    } else {
        assert_eq!(
            params["stoppedEarly"], true,
            "an incomplete copy of a healthy fixture can only be a stopped one: {acquired}"
        );
        assert!(
            params["notAttempted"].as_u64().is_some_and(|n| n > 0),
            "a stopped copy has sectors it never reached: {acquired}"
        );
        assert_eq!(
            params["unreadableRegions"], 0,
            "stopping is not damage: the fixture refused nothing"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn acquiring_onto_a_path_that_already_exists_is_refused_by_the_call() {
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    fixture(&image);
    let copy = dir.path().join("copy.img");
    std::fs::write(&copy, b"an earlier acquisition").expect("write the existing image");

    let mut engine = Engine::spawn();
    engine.handshake();
    let refused = engine.call(
        "acquire.start",
        &serde_json::json!({ "source": image, "to": copy }),
    );
    assert!(
        refused["error"].is_object(),
        "an existing destination must fail the call, not a notification: {refused}"
    );
    assert_eq!(
        std::fs::read(&copy).expect("the earlier image"),
        b"an earlier acquisition",
        "the earlier acquisition must survive untouched"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn an_export_copies_the_set_the_gallery_filter_admits_and_no_more() {
    // The window exports what it is showing rather than a list of hashes, so
    // the two have to agree about what a standing admits. They agree by using
    // one parser: a name the gallery accepts the export accepts, and a set the
    // gallery shows is the set that lands in the folder.
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    fixture(&image);
    let session = dir.path().join("session");

    let mut engine = Engine::spawn();
    engine.handshake();
    engine.call(
        "scan.start",
        &serde_json::json!({
            "source": image, "out": session,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": false, "minLongSide": 0, "previews": false,
        }),
    );
    engine.drain_until_finished();

    // What the gallery would show under the narrowest filter, and what the
    // export produces under the same one: the same count, by construction.
    let page = engine.call(
        "scan.gallery",
        &serde_json::json!({
            "session": session, "offset": 0, "limit": 100, "standing": "camera-named",
        }),
    );
    let shown = page["result"]["total"].as_u64().expect("a total");

    let narrow = dir.path().join("narrow");
    let copied = engine.call(
        "export.copy",
        &serde_json::json!({ "session": session, "to": narrow, "standing": "camera-named" }),
    );
    assert_eq!(
        copied["result"]["copied"], shown,
        "the export must copy exactly what the same filter shows: {copied}"
    );

    // And without a filter it copies more, so the filter is doing something
    // rather than being ignored.
    let all = dir.path().join("all");
    let everything = engine.call(
        "export.copy",
        &serde_json::json!({ "session": session, "to": all }),
    );
    assert!(
        everything["result"]["copied"].as_u64().expect("a count") > shown,
        "an unfiltered export must copy more than the narrowest filter: {everything}"
    );

    // A name this engine does not know is refused rather than silently
    // widening the set — a client one version ahead has to be told.
    let refused = engine.call(
        "export.copy",
        &serde_json::json!({ "session": session, "to": all, "standing": "photogenic" }),
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("not a standing")),
        "an unknown standing must be refused: {refused}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn results_and_export_read_a_session_the_scan_already_wrote() {
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    fixture(&image);
    let session = dir.path().join("session");
    let exported = dir.path().join("exported");

    let mut engine = Engine::spawn();
    engine.handshake();
    engine.call(
        "scan.start",
        &serde_json::json!({
            "source": image, "out": session,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": false, "minLongSide": 0, "previews": true,
        }),
    );
    engine.drain_until_finished();

    let results = engine.call("scan.results", &serde_json::json!({ "session": session }));
    let artifacts = results["result"]["artifacts"]
        .as_array()
        .expect("artifacts");
    assert_eq!(artifacts.len(), 2);
    let wanted = artifacts[0]["sha256"].as_str().expect("a hash").to_owned();

    let copied = engine.call(
        "export.copy",
        &serde_json::json!({ "session": session, "to": exported, "hashes": [wanted] }),
    );
    assert_eq!(copied["result"]["copied"], 1, "{copied}");
    assert!(
        copied["result"]["tampered"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(exported.join("000000.jpg").is_file());
    assert!(!exported.join("000001.png").exists());
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn the_gallery_pages_a_session_and_orders_it_without_the_client_deciding() {
    // What makes a results view possible at all: the engine orders and pages,
    // so a window never receives — or ranks — hundreds of thousands of records.
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    fixture(&image);
    let session = dir.path().join("session");

    let mut engine = Engine::spawn();
    engine.handshake();
    engine.call(
        "scan.start",
        &serde_json::json!({
            "source": image, "out": session,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": false, "minLongSide": 0, "previews": true,
        }),
    );
    engine.drain_until_finished();

    let first = engine.call(
        "scan.gallery",
        &serde_json::json!({ "session": session, "offset": 0, "limit": 1, "standing": null }),
    );
    let page = &first["result"];
    assert_eq!(
        page["artifacts"].as_array().map(Vec::len),
        Some(1),
        "the limit must bound the page: {first}"
    );
    assert_eq!(page["total"], 2, "and the total must count past it");
    assert_eq!(page["recorded"], 2);
    assert_eq!(page["previewDir"], "previews");
    // Every artifact carries the sort key the engine derived, so a client can
    // show it without deriving anything (`A-SHELL-NO-DOMAIN`).
    assert!(
        page["artifacts"][0]["standing"].is_string(),
        "a page must carry the standing: {first}"
    );

    let second = engine.call(
        "scan.gallery",
        &serde_json::json!({ "session": session, "offset": 1, "limit": 10, "standing": null }),
    );
    assert_eq!(
        second["result"]["artifacts"].as_array().map(Vec::len),
        Some(1)
    );
    assert_ne!(
        first["result"]["artifacts"][0]["sha256"], second["result"]["artifacts"][0]["sha256"],
        "paging must advance rather than repeat"
    );

    // A filter narrows without the client knowing what the names mean, and a
    // name the engine does not have is refused at the edge rather than guessed.
    let narrowed = engine.call(
        "scan.gallery",
        &serde_json::json!({
            "session": session, "offset": 0, "limit": 10, "standing": "camera-named",
        }),
    );
    assert!(
        narrowed["result"]["total"].as_u64().is_some_and(|n| n <= 2),
        "{narrowed}"
    );
    let refused = engine.call(
        "scan.gallery",
        &serde_json::json!({
            "session": session, "offset": 0, "limit": 10, "standing": "photograph",
        }),
    );
    assert!(refused["error"].is_object(), "{refused}");
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_scan_that_cannot_open_its_source_fails_the_call_rather_than_the_process() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut engine = Engine::spawn();
    engine.handshake();

    let refused = engine.call(
        "scan.start",
        &serde_json::json!({
            "source": dir.path().join("there-is-no-such-image.img"),
            "out": dir.path().join("out"),
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": false, "minLongSide": 0, "previews": false,
        }),
    );
    assert_eq!(refused["error"]["code"], -32002, "{refused}");

    // The engine is still there and still usable.
    let devices = engine.call("devices.list", &serde_json::json!(null));
    assert!(devices["result"]["devices"].is_array(), "{devices}");
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn cancelling_over_the_wire_still_leaves_a_manifest_behind() {
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    fixture(&image);
    let session = dir.path().join("session");

    let mut engine = Engine::spawn();
    engine.handshake();
    engine.call(
        "scan.start",
        &serde_json::json!({
            "source": image, "out": session,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": false, "minLongSide": 0, "previews": false,
        }),
    );
    let cancelled = engine.call("scan.cancel", &serde_json::json!(null));
    assert_eq!(cancelled["result"]["done"], true, "{cancelled}");

    // Whether the scan finished before the cancel landed or not, a manifest
    // describes whatever reached the output directory. Artifacts without one
    // would be bytes nothing can attribute to a sector.
    let (_, summary) = engine.drain_until_finished();
    let state = summary["state"].as_str().expect("a state");
    assert!(
        state == "cancelled" || state == "finished",
        "unexpected state {state}"
    );
    assert!(session.join("manifest.json").is_file());
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_client_that_disappears_mid_scan_still_leaves_a_manifest() {
    // This is how the shell shuts an engine down: it closes stdin and waits.
    // The engine's dispatch loop ends at end of input and cancels whatever is
    // running, and a cancelled scan still writes its manifest. Without that,
    // closing a window mid-scan would leave artifacts on disk with nothing
    // describing them — bytes no one can attribute to a sector (A-PROVENANCE).
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    fixture(&image);
    let session = dir.path().join("session");

    let mut engine = Engine::spawn();
    engine.handshake();
    engine.call(
        "scan.start",
        &serde_json::json!({
            "source": image, "out": session,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": false, "minLongSide": 0, "previews": false,
        }),
    );

    // Close stdin and nothing else — no kill, no cancel call.
    engine.close_input();

    let status = engine.child.wait().expect("wait for the engine");
    assert!(
        status.success(),
        "the engine should exit cleanly on end of input"
    );
    assert!(
        session.join("manifest.json").is_file(),
        "a scan interrupted by its client losing interest still has to be attributable"
    );
}

/// A disk shaped like a system disk: many small images, few large ones.
///
/// The shape matters. A handful of photographs cannot reproduce what a real
/// medium does to a client, because each artifact is read, hashed and written
/// in about a millisecond — so it is the *count* of artifacts, not their size,
/// that decides how fast the engine talks.
fn many_small_images(at: &Path, count: usize) -> usize {
    let mut disk = Disk::filled(count * 4096 + 512 * 1024);
    for index in 0..count {
        // Distinct content per image: identical bytes collapse at emit time,
        // and a fixture that deduplicates to nothing tests nothing. The two
        // moduli are coprime, so every index in `0..2600` is a different pair.
        let width = u32::try_from(index % 200).expect("a modulus of 200");
        let height = u32::try_from(index % 13).expect("a modulus of 13");
        let image = png(8 + width, 8 + height);
        disk = disk.with(64 * 1024 + index * 4096, &image);
    }
    let bytes = disk.into_bytes();
    std::fs::write(at, &bytes).expect("write fixture disk");
    count
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_client_is_never_asked_to_keep_up_with_the_report_stage() {
    // The failure this guards against: the report stage emits two events per
    // artifact, and artifacts are small. Unpaced, a disk full of icons pushes
    // tens of thousands of messages a second at whatever is listening, which
    // is enough to stop a web view drawing, stop its clock and get it killed.
    // What the engine produces is not the problem; what it *sends* is.
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("many.img");
    let planted = many_small_images(&image, 2000);
    let session = dir.path().join("session");

    let mut engine = Engine::spawn();
    engine.handshake();
    engine.call(
        "scan.start",
        &serde_json::json!({
            "source": image, "out": session,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": false, "minLongSide": 0, "previews": false,
        }),
    );

    let (methods, summary) = engine.drain_until_finished();
    let artifacts = usize::try_from(summary["artifacts"].as_u64().expect("a count"))
        .expect("a count that fits");
    assert!(
        artifacts > planted / 2,
        "the fixture must actually recover: {artifacts} of {planted}"
    );

    // Two events per artifact would be `2 * artifacts`. The cap is a rate, so
    // the bound is generous in absolute terms and still orders of magnitude
    // below one message per artifact.
    let progress = methods
        .iter()
        .filter(|method| *method == "progress" || *method == "stored")
        .count();
    assert!(
        progress < artifacts / 4,
        "the wire carried {progress} cumulative notifications for {artifacts} artifacts; \
         a client is being asked to keep up with the pipeline"
    );

    // And pacing must not cost the client the final figures: the last progress
    // pending when a stage ends is flushed before the stage ends.
    assert!(
        methods.iter().any(|method| method == "stored"),
        "a paced client still has to be told what was recovered: {methods:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn the_final_account_never_reports_an_unwritten_artifact_as_recovered() {
    // A run asked to leave synthetic assets unwritten records every one of
    // them, because the manifest is the account of the medium rather than of
    // the output directory. That makes the manifest's length the wrong number
    // to end a run with: a client showing it would tell its viewer that
    // thousands of images were recovered while the destination folder holds a
    // handful (`A-CONFIDENCE-HONEST`). What was written and what was only
    // recorded are two figures, and they are counted apart.
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("assets.img");
    let mut disk = Disk::noisy(2 * 1024 * 1024, 0x5EED_0F0F);
    for index in 0..8_usize {
        let seed = u32::try_from(index).expect("eight of them");
        disk = disk.with(64 * 1024 + index * 96 * 1024, &icon_png(96, seed));
    }
    disk = disk.with(64 * 1024 + 8 * 96 * 1024, &photo_jpeg(96, 96, 0x1234));
    std::fs::write(&image, disk.into_bytes()).expect("write fixture disk");
    let session = dir.path().join("session");

    let mut engine = Engine::spawn();
    engine.handshake();
    engine.call(
        "scan.start",
        &serde_json::json!({
            "source": image, "out": session,
            "filesystem": true, "carving": true, "reassembly": true,
            "triage": true, "minLongSide": 300, "previews": false,
        }),
    );
    let (_, summary) = engine.drain_until_finished();

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(session.join("manifest.json")).expect("manifest"))
            .expect("valid json");
    let records = manifest["artifacts"].as_array().expect("artifacts");
    let written = records
        .iter()
        .filter(|record| record["written"] == serde_json::json!(true))
        .count();
    let omitted = records.len() - written;

    assert!(omitted > 0, "the fixture must produce an omission");
    assert_eq!(
        summary["artifacts"].as_u64().expect("a count"),
        written as u64,
        "the account of what was recovered must count only what was written: {summary}"
    );
    assert_eq!(
        summary["omitted"].as_u64().expect("a count"),
        omitted as u64,
        "and must say how many were recorded without being written: {summary}"
    );

    // The two together are the whole manifest, so nothing falls between them.
    let counted = summary["artifacts"].as_u64().expect("a count")
        + summary["omitted"].as_u64().expect("a count");
    assert_eq!(counted, records.len() as u64);

    // And the byte total describes the directory, not the medium.
    let stored: u64 = records
        .iter()
        .filter(|record| record["written"] == serde_json::json!(true))
        .map(|record| record["length"].as_u64().expect("a length"))
        .sum();
    assert_eq!(summary["bytes"].as_u64().expect("a byte total"), stored);
}
