//! Semantic checks after schema acceptance (A6), in contract order.

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
