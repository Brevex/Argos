//! The pinned model artifact and its verification.
//!
//! The weights are compiled into the binary from a file in the source tree,
//! and their SHA-256 is a constant beside them. Verification failing does not
//! fail a scan: the caller falls back to the null classifier and the manifest
//! records that triage was disabled (A-MODEL-PINNED). There is no code path
//! that fetches a model from anywhere.

use argos_core::artifact::Digest;
use sha2::{Digest as _, Sha256};

use crate::error::TriageError;
use crate::net::Net;

/// Version of the pinned model, recorded in every manifest a scan writes.
pub const MODEL_VERSION: &str = "triage-cnn-v1";

/// SHA-256 the pinned model file must hash to, lowercase hex.
///
/// Produced by `tools/train_triage` together with the artifact; every model
/// replacement updates the file, this constant, and the eval numbers in the
/// harness, in one change (A-EVAL-GATED).
pub const MODEL_SHA256_HEX: &str =
    "eca41b1f96e28b021c354660698ccc4308edfd6e9967132bb2cfb63e3e171720";

/// The pinned weights, compiled in from the source tree.
const MODEL_BYTES: &[u8] = include_bytes!("../model/triage-v1.safetensors");

/// Verifies the pinned bytes and builds the network from them.
pub(crate) fn load_pinned() -> Result<(Net, Digest), TriageError> {
    verify_and_load(MODEL_BYTES)
}

/// Verifies `bytes` against the pinned hash and builds the network.
///
/// Split out from [`load_pinned`] so the failure path is reachable from a
/// test. `A-MODEL-PINNED` requires that a model which does not verify
/// disables triage and is reported, and *never* fails the scan — a contract
/// with no way to exercise it is a contract that will quietly stop holding.
pub(crate) fn verify_and_load(bytes: &[u8]) -> Result<(Net, Digest), TriageError> {
    let digest = Sha256::digest(bytes);
    let found = Digest::new(digest.into());
    if found.to_string() != MODEL_SHA256_HEX {
        return Err(TriageError::pin_mismatch(found));
    }
    let net = Net::load(bytes).map_err(TriageError::load)?;
    Ok((net, found))
}
