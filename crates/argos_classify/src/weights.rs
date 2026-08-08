//! Reading the pinned weight file.
//!
//! The format is safetensors: an eight-byte little-endian header length, a
//! JSON header naming each tensor with its dtype, shape and byte range, then
//! the tensor data. Reading it here rather than through an inference runtime
//! is what keeps the evidence path free of a two-hundred-crate dependency
//! tree; the training tool, which lives outside the workspace, still writes
//! the file with the runtime that produced it.
//!
//! The file is hash-verified before it reaches this module, so it is trusted
//! input — but it is still parsed defensively, because "trusted" here means
//! "matches a constant in the source tree", and a build that got that wrong
//! should fail cleanly rather than index out of bounds.

use std::collections::BTreeMap;

/// One tensor: its shape and its values.
#[derive(Clone, Debug)]
pub(crate) struct Tensor {
    pub shape: Vec<usize>,
    pub values: Vec<f32>,
}

impl Tensor {
    /// Number of values the shape calls for.
    fn expected_len(&self) -> Option<usize> {
        self.shape.iter().try_fold(1_usize, |product, dimension| {
            product.checked_mul(*dimension)
        })
    }
}

/// Every tensor in a weight file, by name.
pub(crate) type Weights = BTreeMap<String, Tensor>;

/// What a weight file can be wrong about.
///
/// Only reachable through [`Net::load`](crate::net::Net::load); a scan meets
/// it as a [`TriageError`](crate::TriageError) instead, which is what turns
/// a bad weight file into "triage disabled" rather than a failed scan.
#[derive(Debug)]
pub enum WeightError {
    /// The file is too short to hold its own header.
    Truncated,
    /// The JSON header did not parse, or was not the expected shape.
    Header(String),
    /// A tensor's declared byte range does not lie inside the file, or does
    /// not match its declared shape.
    Tensor {
        /// Name the header gave the tensor.
        name: String,
        /// What is wrong with it.
        problem: String,
    },
}

impl std::fmt::Display for WeightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => f.write_str("weight file ends before its header does"),
            Self::Header(problem) => write!(f, "weight file header is not usable: {problem}"),
            Self::Tensor { name, problem } => {
                write!(f, "weight file tensor {name} is not usable: {problem}")
            }
        }
    }
}

impl std::error::Error for WeightError {}

/// Bytes of the length prefix that precedes the JSON header.
const LENGTH_PREFIX_BYTES: usize = 8;

/// Ceiling on the JSON header, so a corrupt length cannot ask for an
/// enormous slice. The real header is under a kilobyte.
const MAX_HEADER_BYTES: u64 = 1 << 20;

/// Parses a safetensors file into its `F32` tensors.
///
/// Tensors of any other dtype are ignored; the network this crate builds
/// declares exactly the ones it needs, so a missing tensor fails there with
/// its name rather than here with a type code.
pub(crate) fn parse(bytes: &[u8]) -> Result<Weights, WeightError> {
    let prefix = bytes
        .get(..LENGTH_PREFIX_BYTES)
        .ok_or(WeightError::Truncated)?;
    let header_len = u64::from_le_bytes(
        prefix
            .try_into()
            .unwrap_or_else(|_| unreachable!("the slice is exactly eight bytes")),
    );
    if header_len > MAX_HEADER_BYTES {
        return Err(WeightError::Header(format!(
            "declared header of {header_len} bytes is past the {MAX_HEADER_BYTES} this reader \
             accepts"
        )));
    }
    let header_len =
        usize::try_from(header_len).map_err(|_unrepresentable| WeightError::Truncated)?;
    let header_end = LENGTH_PREFIX_BYTES
        .checked_add(header_len)
        .ok_or(WeightError::Truncated)?;
    let header = bytes
        .get(LENGTH_PREFIX_BYTES..header_end)
        .ok_or(WeightError::Truncated)?;
    let data = bytes.get(header_end..).ok_or(WeightError::Truncated)?;

    let parsed: serde_json::Value =
        serde_json::from_slice(header).map_err(|err| WeightError::Header(err.to_string()))?;
    let entries = parsed
        .as_object()
        .ok_or_else(|| WeightError::Header("header is not a JSON object".to_owned()))?;

    let mut weights = Weights::new();
    for (name, entry) in entries {
        // Metadata carries no tensor.
        if name == "__metadata__" {
            continue;
        }
        if entry.get("dtype").and_then(serde_json::Value::as_str) != Some("F32") {
            continue;
        }
        weights.insert(name.clone(), tensor_from(name, entry, data)?);
    }
    Ok(weights)
}

/// Reads one `F32` tensor described by `entry` out of the data section.
fn tensor_from(name: &str, entry: &serde_json::Value, data: &[u8]) -> Result<Tensor, WeightError> {
    let fail = |problem: String| WeightError::Tensor {
        name: name.to_owned(),
        problem,
    };
    let shape: Vec<usize> = entry
        .get("shape")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| fail("no shape".to_owned()))?
        .iter()
        .map(|dimension| {
            dimension
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| fail("shape holds a value that is not a length".to_owned()))
        })
        .collect::<Result<_, _>>()?;

    let offsets = entry
        .get("data_offsets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| fail("no data offsets".to_owned()))?;
    let [start, end] = offsets.as_slice() else {
        return Err(fail("data offsets are not a pair".to_owned()));
    };
    let position = |value: &serde_json::Value, which: &str| {
        value
            .as_u64()
            .and_then(|at| usize::try_from(at).ok())
            .ok_or_else(|| fail(format!("{which} offset is not a position in this file")))
    };
    let start = position(start, "start")?;
    let end = position(end, "end")?;
    if end < start {
        return Err(fail("data offsets run backwards".to_owned()));
    }
    let raw = data
        .get(start..end)
        .ok_or_else(|| fail("data offsets run past the end of the file".to_owned()))?;
    if !raw.len().is_multiple_of(size_of::<f32>()) {
        return Err(fail("data length is not a whole number of f32".to_owned()));
    }

    let values: Vec<f32> = raw
        .chunks_exact(size_of::<f32>())
        .map(|chunk| {
            f32::from_le_bytes(
                chunk
                    .try_into()
                    .unwrap_or_else(|_| unreachable!("chunks_exact yields four bytes")),
            )
        })
        .collect();
    let tensor = Tensor { shape, values };
    if tensor.expected_len() != Some(tensor.values.len()) {
        return Err(fail(format!(
            "shape {:?} calls for a different number of values than the {} stored",
            tensor.shape,
            tensor.values.len()
        )));
    }
    Ok(tensor)
}

/// Takes the tensor named `name`, checking its shape.
pub(crate) fn take(
    weights: &mut Weights,
    name: &str,
    shape: &[usize],
) -> Result<Vec<f32>, WeightError> {
    let tensor = weights.remove(name).ok_or_else(|| WeightError::Tensor {
        name: name.to_owned(),
        problem: "not in the weight file".to_owned(),
    })?;
    if tensor.shape != shape {
        return Err(WeightError::Tensor {
            name: name.to_owned(),
            problem: format!("shape is {:?}, expected {shape:?}", tensor.shape),
        });
    }
    Ok(tensor.values)
}
