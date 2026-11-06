// SPDX-License-Identifier: MIT OR Apache-2.0

//! `report-admission:v0` bounded wire canonicalization and SHA-256 hashing over
//! `canonprims-json` (RFC 8785 / strict I-JSON).

use std::io::Read;

use canonprims_core::{Code, DigestAlg, Limits};
use serde_json::Value;
use thiserror::Error;

/// Maximum received report or evidence payload size in bytes (`256 KiB`).
pub const MAX_REPORT_BYTES: usize = 262_144;

/// Maximum JSON container nesting depth (`outermost array/object == 1`).
pub const MAX_JSON_DEPTH: usize = 16;

/// Maximum RFC 8785 canonical JSON output size in bytes (`256 KiB`).
pub const MAX_CANONICAL_BYTES: usize = 262_144;

/// Admitted wire document with RFC 8785 canonical JSON bytes, SHA-256 digest,
/// and parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedWire {
    /// RFC 8785 canonical UTF-8 JSON bytes (`<= 262_144` bytes).
    pub canonical_bytes: Vec<u8>,
    /// `sha256:<64 lowercase hex>` of `canonical_bytes`.
    pub digest: String,
    /// Parsed duplicate-free I-JSON value.
    pub value: Value,
}

/// Normative wire-admission refusal (`report-admission:v0` + `canonprims` codes).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code}")]
pub struct WireRefusal {
    /// Normative refusal code (for example `report-too-large`,
    /// `canonical-report-too-large`, `duplicate_member`, `too_deep`).
    pub code: String,
}

impl WireRefusal {
    /// Construct a wire refusal with `code`.
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }

    /// Borrow the normative refusal code.
    pub fn code(&self) -> &str {
        &self.code
    }
}

fn wire_limits() -> Limits {
    Limits {
        max_bytes: MAX_REPORT_BYTES,
        max_depth: MAX_JSON_DEPTH,
        ..Limits::default()
    }
}

/// Enforce `report-admission:v0` over a raw UTF-8 byte slice:
///
/// 1. Raw byte cap (`> 262_144` -> `report-too-large`) before parsing;
/// 2. Strict UTF-8, RFC 7493 I-JSON, duplicate-member, binary64 integer, finite
///    number, and depth `<= 16` checks via `canonprims_json::canonicalize`;
/// 3. Canonical output cap (`> 262_144` -> `canonical-report-too-large`);
/// 4. `sha256:<lowercase hex>` over the RFC 8785 canonical bytes.
pub fn admit_report_bytes(raw: &[u8]) -> Result<AdmittedWire, WireRefusal> {
    if raw.len() > MAX_REPORT_BYTES {
        return Err(WireRefusal::new("report-too-large"));
    }
    let canonical_bytes = canonprims_json::canonicalize(raw, &wire_limits()).map_err(|r| {
        if r.code == Code::TooLarge {
            WireRefusal::new("report-too-large")
        } else {
            WireRefusal::new(r.code.as_str())
        }
    })?;
    if canonical_bytes.len() > MAX_CANONICAL_BYTES {
        return Err(WireRefusal::new("canonical-report-too-large"));
    }
    let digest = DigestAlg::Sha256.format(&canonical_bytes);
    let value: Value =
        serde_json::from_slice(&canonical_bytes).map_err(|_| WireRefusal::new("invalid_json"))?;
    Ok(AdmittedWire {
        canonical_bytes,
        digest,
        value,
    })
}

/// Enforce `report-admission:v0` over a byte stream without unbounded buffering
/// (`Read::take(MAX_REPORT_BYTES + 1)`).
pub fn admit_report_reader(reader: impl Read) -> Result<AdmittedWire, WireRefusal> {
    let mut buf = Vec::new();
    reader
        .take((MAX_REPORT_BYTES + 1) as u64)
        .read_to_end(&mut buf)
        .map_err(|_| WireRefusal::new("io_error"))?;
    if buf.len() > MAX_REPORT_BYTES {
        return Err(WireRefusal::new("report-too-large"));
    }
    admit_report_bytes(&buf)
}

/// Compute `sha256:<lowercase hex>` over the RFC 8785 canonical JSON form of a
/// validated [`serde_json::Value`].
pub fn canonical_sha256_value(value: &Value) -> Result<String, WireRefusal> {
    let raw = serde_json::to_vec(value).map_err(|_| WireRefusal::new("invalid_json"))?;
    let canonical = canonprims_json::canonicalize(&raw, &Limits::default())
        .map_err(|r| WireRefusal::new(r.code.as_str()))?;
    Ok(DigestAlg::Sha256.format(&canonical))
}
