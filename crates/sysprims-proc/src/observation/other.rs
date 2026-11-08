// SPDX-License-Identifier: MIT OR Apache-2.0

//! Explicit refusal implementation for platforms without a qualified
//! `native-process-observation:v0` collector cell (including Windows).

use std::time::Duration;

use serde_json::Value;

use super::{CollectorRefusal, ObservationOptions};

pub(super) struct PlatformHandle;

impl std::fmt::Debug for PlatformHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformHandle")
            .field("backend", &"unsupported")
            .finish_non_exhaustive()
    }
}

impl PlatformHandle {
    #[cfg(test)]
    pub(super) fn new_sentinel_for_debug_test(_fd: i32) -> Self {
        Self
    }

    #[cfg(test)]
    pub(super) fn disarm_for_debug_test(&mut self) {}

    pub(super) fn poll_exited(&self) -> Result<bool, CollectorRefusal> {
        Err(unqualified_refusal())
    }

    pub(super) fn wait_exit(&self, _timeout: Duration) -> Result<bool, CollectorRefusal> {
        Err(unqualified_refusal())
    }
}

fn unqualified_refusal() -> CollectorRefusal {
    if cfg!(windows) {
        CollectorRefusal::unsupported(
            "windows-collector-not-qualified",
            "Windows start-identity profile (windows-filetime:v0) is reserved and not yet qualified",
        )
    } else {
        CollectorRefusal::unsupported(
            "unsupported-platform",
            "No native-process-observation:v0 collector is qualified for this target OS",
        )
    }
}

pub(super) fn observe_process_impl(
    _pid: u32,
    _host_scope_json: Value,
    _observed_at: &str,
    _options: &ObservationOptions,
) -> Result<Value, CollectorRefusal> {
    Err(unqualified_refusal())
}

pub(super) fn open_continuity_handle_impl(
    _pid: u32,
    _host_scope_json: Value,
    _observed_at: &str,
    _options: &ObservationOptions,
) -> Result<(Value, PlatformHandle), CollectorRefusal> {
    Err(unqualified_refusal())
}
