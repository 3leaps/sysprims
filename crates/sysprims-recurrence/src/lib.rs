//! RFC 5545 recurrence rules under `contract: recurrence/v0`.
//!
//! A [`RecurrenceRule`] is obtained only through a validating constructor
//! ([`RecurrenceRule::from_json`], [`RecurrenceRule::from_text`],
//! [`RecurrenceRule::from_document_json`]); every value is therefore valid
//! and in normal form. It produces canonical JSON (RFC 8785), a `sha256:`
//! digest, and canonical RFC 5545 text.
//!
//! Time zones come only from the tzdb bundle compiled into this crate
//! ([`tzdb_version`]); host time-zone data is never consulted. A
//! conformance claim names the pair ([`CORPUS_DIGEST`], [`tzdb_version`]).
//!
#![doc = include_str!("../README.md")]

mod civil;
mod json;
mod model;
mod refusal;
mod tz;
mod validate;

pub use civil::{CivilDateTime, Instant};
pub use model::{ByDay, Freq, RecurrenceRule, Weekday, CONTRACT};
pub use refusal::{Refusal, RefusalReason};
pub use tz::tzdb_version;

/// The recurrence/v0 conformance corpus this build is tested against.
pub const CORPUS_DIGEST: &str =
    "sha256:69387023c21ca147f2dd59c4e47f48447f5eb3ad77121975aab0e4fe0680b35b";
