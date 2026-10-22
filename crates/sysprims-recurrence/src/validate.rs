//! Semantic checks after schema acceptance (A6), in contract order.

use crate::expand::Engine;
use crate::model::RecurrenceRule;
use crate::refusal::{Refusal, RefusalReason};
use crate::tz;

pub(crate) fn check_tzid(tzid: &str) -> Result<(), Refusal> {
    if tz::is_accepted(tzid) {
        Ok(())
    } else {
        Err(Refusal::new(
            RefusalReason::InvalidTzid,
            "tzid is not in the pinned roster",
        ))
    }
}

/// `dtstart` must be selected by the normalized rule's first period
/// (explicit-gap seed rule included), independently of `until`.
pub(crate) fn check_synchronized(rule: &RecurrenceRule) -> Result<(), Refusal> {
    if Engine::new(rule).synchronized() {
        Ok(())
    } else {
        Err(Refusal::new(
            RefusalReason::DtstartNotSynchronized,
            "dtstart is not selected by the first period",
        ))
    }
}
