//! sysprims-core: Core types, errors, and platform abstractions
//!
//! This crate provides the foundational types used across all sysprims modules:
//! - Error types per ADR-0008
//! - Schema ID constants for JSON output contracts
//! - Re-exports from rsfulmen for signal and exit code constants
//! - Platform detection utilities
//!
//! ## Error Handling
//!
//! sysprims uses a single canonical error type [`SysprimsError`] that maps
//! cleanly to FFI error codes. See ADR-0008 for the full strategy.
//!
//! ## Schema Integration
//!
//! JSON outputs include `schema_id` fields referencing schemas hosted at
//! `schemas.3leaps.dev/sysprims/`. See the [`schema`] module for constants.
//!
//! ## Ecosystem Integration
//!
//! Signal and exit code constants are re-exported from rsfulmen for
//! Fulmen ecosystem alignment. Access via [`signals`] and [`exit_codes`].

use std::env::consts::OS;

pub mod error;
pub mod schema;

// Re-export canonical error type at crate root
pub use error::{SysprimsError, SysprimsResult};

// Re-export rsfulmen foundry types for ecosystem alignment
// Using module re-exports (not glob) to keep origin obvious and avoid pollution
pub use rsfulmen::foundry::exit_codes;
pub use rsfulmen::foundry::signals;

// ============================================================================
// Platform Detection
// ============================================================================

/// Get the current platform identifier.
///
/// Returns one of: "linux", "macos", "windows", "freebsd", etc.
///
/// This is a pure function with no side effects.
#[inline]
pub fn get_platform() -> &'static str {
    OS
}

/// Check if running on a Unix-like platform.
#[inline]
#[cfg(unix)]
pub const fn is_unix() -> bool {
    true
}

#[inline]
#[cfg(not(unix))]
pub const fn is_unix() -> bool {
    false
}

/// Check if running on Windows.
#[inline]
#[cfg(windows)]
pub const fn is_windows() -> bool {
    true
}

#[inline]
#[cfg(not(windows))]
pub const fn is_windows() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_platform() {
        let platform = get_platform();
        assert!(!platform.is_empty());
        // Platform should be one of the known values
        assert!(
            ["linux", "macos", "windows", "freebsd"].contains(&platform),
            "Unexpected platform: {}",
            platform
        );
    }

    #[test]
    fn test_platform_detection_consistency() {
        // On Unix, is_unix should be true
        #[cfg(unix)]
        {
            assert!(is_unix());
            assert!(!is_windows());
        }

        // On Windows, is_windows should be true
        #[cfg(windows)]
        {
            assert!(is_windows());
            assert!(!is_unix());
        }
    }

    #[test]
    fn test_rsfulmen_reexports() {
        // Verify rsfulmen constants are accessible through our re-exports
        assert_eq!(signals::SIGTERM, 15);
        assert_eq!(signals::SIGKILL, 9);
        assert_eq!(exit_codes::EXIT_SUCCESS, 0);
        assert_eq!(exit_codes::EXIT_SIGNAL_TERM, 143);
    }
}
