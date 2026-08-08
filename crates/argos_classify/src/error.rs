//! What triage fails with.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;

use argos_core::artifact::Digest;

use crate::model::MODEL_SHA256_HEX;
use crate::weights::WeightError;

/// Triage could not be set up.
///
/// Never fails a scan: the caller proceeds with the null classifier and the
/// manifest records that triage was disabled, with the reason
/// (A-MODEL-PINNED, A-TRIAGE-NOT-VERDICT).
///
/// Boxed inside, so the `Result`s this crate returns stay small enough that
/// the success path does not pay for the failure path.
#[derive(Debug)]
pub struct TriageError(Box<Inner>);

#[derive(Debug)]
struct Inner {
    kind: Kind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum Kind {
    /// The compiled-in model does not hash to the pinned constant.
    PinMismatch { found: Digest },
    /// The verified bytes did not load as the expected network.
    Load(WeightError),
}

impl TriageError {
    pub(crate) fn pin_mismatch(found: Digest) -> Self {
        Self::new(Kind::PinMismatch { found })
    }

    pub(crate) fn load(source: WeightError) -> Self {
        Self::new(Kind::Load(source))
    }

    fn new(kind: Kind) -> Self {
        Self(Box::new(Inner {
            kind,
            backtrace: Backtrace::capture(),
        }))
    }

    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.0.backtrace
    }
}

impl fmt::Display for TriageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0.kind {
            Kind::PinMismatch { found } => write!(
                f,
                "triage model failed pin verification: expected sha256 {MODEL_SHA256_HEX}, \
                 found {found}"
            ),
            Kind::Load(source) => write!(f, "triage model did not load: {source}"),
        }?;
        if self.0.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.0.backtrace)?;
        }
        Ok(())
    }
}

impl Error for TriageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.0.kind {
            Kind::PinMismatch { .. } => None,
            Kind::Load(source) => Some(source),
        }
    }
}
