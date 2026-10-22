use std::fmt;

/// The contract's refusal-reason table (A6). Each refused input has exactly one reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RefusalReason {
    /// The JSON model fails the schema (also: text integers outside value bounds).
    InvalidRule,
    /// RFC 5545 text is malformed or uses a non-local `DTSTART` form.
    InvalidText,
    /// Text uses a part or property outside the frozen subset.
    UnsupportedPart,
    /// The `tzid` is not an accepted name in the pinned tzdb.
    InvalidTzid,
    /// A civil date-time names a date or time that does not exist.
    InvalidDate,
    /// `dtstart` does not match the normalized first-period pattern.
    DtstartNotSynchronized,
}

impl RefusalReason {
    /// The contract's wire spelling of this reason.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidRule => "invalid_rule",
            Self::InvalidText => "invalid_text",
            Self::UnsupportedPart => "unsupported_part",
            Self::InvalidTzid => "invalid_tzid",
            Self::InvalidDate => "invalid_date",
            Self::DtstartNotSynchronized => "dtstart_not_synchronized",
        }
    }
}

impl fmt::Display for RefusalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A refused rule. `detail` is diagnostic text for humans; it names the
/// offending field but never echoes input values, and it is not part of any
/// conformance comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    reason: RefusalReason,
    detail: &'static str,
}

impl Refusal {
    pub(crate) fn new(reason: RefusalReason, detail: &'static str) -> Self {
        Self { reason, detail }
    }
    pub fn reason(&self) -> RefusalReason {
        self.reason
    }
    pub fn detail(&self) -> &'static str {
        self.detail
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.reason, self.detail)
    }
}

impl std::error::Error for Refusal {}

pub(crate) fn rule(detail: &'static str) -> Refusal {
    Refusal::new(RefusalReason::InvalidRule, detail)
}
pub(crate) fn text(detail: &'static str) -> Refusal {
    Refusal::new(RefusalReason::InvalidText, detail)
}
pub(crate) fn unsupported(detail: &'static str) -> Refusal {
    Refusal::new(RefusalReason::UnsupportedPart, detail)
}
pub(crate) fn date(detail: &'static str) -> Refusal {
    Refusal::new(RefusalReason::InvalidDate, detail)
}
