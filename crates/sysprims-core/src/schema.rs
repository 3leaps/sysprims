//! Schema ID constants for JSON output contracts.
//!
//! All sysprims JSON outputs include a `schema_id` field that references
//! the corresponding schema. These constants define the canonical schema URLs.
//!
//! ## Schema Hosting
//!
//! sysprims schemas are hosted at `schemas.3leaps.dev/sysprims/` (not fulmenhq.dev).
//! While sysprims uses rsfulmen for signal/exit code constants from the Fulmen
//! ecosystem, sysprims itself is a 3leaps project aimed at wider community use.
//!
//! For Crucible/Fulmen schemas consumed via rsfulmen, use rsfulmen's resolver.
//!
//! ## URI Structure
//!
//! Follows the Canonical URI Resolution Standard:
//! ```text
//! https://schemas.3leaps.dev/<module>/<topic>/<version>/<filename>
//! ```
//!
//! Where:
//! - `module` = `sysprims` (source repository)
//! - `topic` = feature area (e.g., `timeout`, `process`, `signal`)
//! - `version` = SemVer (e.g., `v1.0.0`)
//! - `filename` = schema file with `.schema.json` suffix
//!
//! ## Validation Strategy
//!
//! sysprims does NOT perform runtime JSON schema validation (too heavy).
//! Instead:
//! - Input validation: `serde(deny_unknown_fields)` + manual range checks
//! - Output validation: goneat CLI in CI pipeline
//! - Schema ID verification: Unit tests against SSOT
//!
//! ## Example
//!
//! ```rust,ignore
//! use sysprims_core::schema::TIMEOUT_RESULT_V1;
//! use serde::Serialize;
//!
//! #[derive(Serialize)]
//! struct TimeoutResult {
//!     schema_id: &'static str,
//!     status: String,
//!     // ... other fields
//! }
//!
//! let result = TimeoutResult {
//!     schema_id: TIMEOUT_RESULT_V1,
//!     status: "completed".into(),
//! };
//! ```

/// Schema ID for timeout result JSON output (v1.0.0).
///
/// This schema defines the structure of `sysprims timeout --json` output.
///
/// Schema location: `schemas/timeout/v1.0.0/timeout-result.schema.json`
pub const TIMEOUT_RESULT_V1: &str =
    "https://schemas.3leaps.dev/sysprims/timeout/v1.0.0/timeout-result.schema.json";

/// Schema ID for process info JSON output (v1.1.0).
///
/// This schema defines the structure of `sysprims pstat --json` output.
///
/// Schema location: `schemas/process/v1.1.0/process-info.schema.json`
pub const PROCESS_INFO_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info.schema.json";

/// Schema ID for process snapshot output with sampled (monitor-style) CPU (v1.1.0).
///
/// This schema matches the shape of `process-info.schema.json` but relaxes
/// `cpu_percent` to allow values > 100 when a process uses multiple cores.
///
/// Schema location: `schemas/process/v1.1.0/process-info-sampled.schema.json`
pub const PROCESS_INFO_SAMPLED_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info-sampled.schema.json";

/// Schema ID for process filter input (v1.0.0).
///
/// This schema defines the structure of filter JSON accepted by
/// `sysprims_proc_list()` FFI function.
///
/// Schema location: `schemas/process/v1.0.0/process-filter.schema.json`
pub const PROC_FILTER_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/process-filter.schema.json";

/// Schema ID for port binding snapshot output (v1.0.0).
///
/// This schema defines the structure of `listening_ports()` output.
///
/// Schema location: `schemas/process/v1.0.0/port-bindings.schema.json`
pub const PORT_BINDINGS_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-bindings.schema.json";

/// Schema ID for port filter input (v1.0.0).
///
/// This schema defines the structure of filter JSON accepted by
/// `sysprims_proc_listening_ports()` FFI function.
///
/// Schema location: `schemas/process/v1.0.0/port-filter.schema.json`
pub const PORT_FILTER_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-filter.schema.json";

/// Schema ID for file descriptor snapshot output (v1.0.0).
///
/// Schema location: `schemas/process/v1.0.0/fd-snapshot.schema.json`
pub const FD_SNAPSHOT_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-snapshot.schema.json";

/// Schema ID for file descriptor filter input (v1.0.0).
///
/// Schema location: `schemas/process/v1.0.0/fd-filter.schema.json`
pub const FD_FILTER_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-filter.schema.json";

/// Schema ID for wait-pid result JSON output (v1.0.0).
///
/// This schema defines the structure of `wait_pid()` output.
///
/// Schema location: `schemas/process/v1.0.0/wait-pid-result.schema.json`
pub const WAIT_PID_RESULT_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/wait-pid-result.schema.json";

/// Schema ID for batch kill result JSON output (v1.0.0).
///
/// This schema defines the structure of `sysprims kill --json` output.
///
/// Schema location: `schemas/signal/v1.0.0/batch-kill-result.schema.json`
pub const BATCH_KILL_RESULT_V1: &str =
    "https://schemas.3leaps.dev/sysprims/signal/v1.0.0/batch-kill-result.schema.json";

/// Schema ID for terminate-tree config JSON input (v1.0.0).
///
/// Schema location: `schemas/process/v1.0.0/terminate-tree-config.schema.json`
pub const TERMINATE_TREE_CONFIG_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-config.schema.json";

/// Schema ID for terminate-tree result JSON output (v1.0.0).
///
/// Schema location: `schemas/process/v1.0.0/terminate-tree-result.schema.json`
pub const TERMINATE_TREE_RESULT_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-result.schema.json";

/// Schema ID for spawn-in-group config JSON input (v1.0.0).
///
/// Schema location: `schemas/process/v1.0.0/spawn-in-group-config.schema.json`
pub const SPAWN_IN_GROUP_CONFIG_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/spawn-in-group-config.schema.json";

/// Schema ID for spawn-in-group result JSON output (v1.0.0).
///
/// Schema location: `schemas/process/v1.0.0/spawn-in-group-result.schema.json`
pub const SPAWN_IN_GROUP_RESULT_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/spawn-in-group-result.schema.json";

/// Schema ID for descendants result JSON output (v1.0.0).
///
/// This schema defines the structure of `sysprims descendants --json` output.
///
/// Schema location: `schemas/process/v1.0.0/descendants-result.schema.json`
pub const DESCENDANTS_RESULT_V1: &str =
    "https://schemas.3leaps.dev/sysprims/process/v1.0.0/descendants-result.schema.json";

// ============================================================================
// Schema Host Constants
// ============================================================================

/// Base URL for sysprims schemas.
pub const SCHEMA_HOST: &str = "https://schemas.3leaps.dev";

/// Module name for sysprims in schema URIs.
pub const SCHEMA_MODULE: &str = "sysprims";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_ids_are_valid_urls() {
        // All schema IDs should be valid HTTPS URLs
        assert!(TIMEOUT_RESULT_V1.starts_with("https://"));
        assert!(PROCESS_INFO_V1.starts_with("https://"));
        assert!(PROCESS_INFO_SAMPLED_V1.starts_with("https://"));
        assert!(PROC_FILTER_V1.starts_with("https://"));
        assert!(PORT_BINDINGS_V1.starts_with("https://"));
        assert!(PORT_FILTER_V1.starts_with("https://"));
        assert!(FD_SNAPSHOT_V1.starts_with("https://"));
        assert!(FD_FILTER_V1.starts_with("https://"));
        assert!(WAIT_PID_RESULT_V1.starts_with("https://"));
        assert!(BATCH_KILL_RESULT_V1.starts_with("https://"));
        assert!(TERMINATE_TREE_CONFIG_V1.starts_with("https://"));
        assert!(TERMINATE_TREE_RESULT_V1.starts_with("https://"));
        assert!(SPAWN_IN_GROUP_CONFIG_V1.starts_with("https://"));
        assert!(SPAWN_IN_GROUP_RESULT_V1.starts_with("https://"));
        assert!(DESCENDANTS_RESULT_V1.starts_with("https://"));
    }

    #[test]
    fn test_schema_ids_use_3leaps_host() {
        // sysprims schemas live at schemas.3leaps.dev, not fulmenhq.dev
        let expected_prefix = "https://schemas.3leaps.dev/sysprims/";

        assert!(
            TIMEOUT_RESULT_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            PROCESS_INFO_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            PROCESS_INFO_SAMPLED_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            PROC_FILTER_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            PORT_BINDINGS_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            PORT_FILTER_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            FD_SNAPSHOT_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            FD_FILTER_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            WAIT_PID_RESULT_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            BATCH_KILL_RESULT_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            TERMINATE_TREE_CONFIG_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            TERMINATE_TREE_RESULT_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            SPAWN_IN_GROUP_CONFIG_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            SPAWN_IN_GROUP_RESULT_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
        assert!(
            DESCENDANTS_RESULT_V1.starts_with(expected_prefix),
            "Expected 3leaps.dev host"
        );
    }

    #[test]
    fn test_schema_ids_follow_canonical_uri_pattern() {
        // Pattern: https://schemas.3leaps.dev/sysprims/<topic>/<version>/<filename>.schema.json

        // All should end with .schema.json
        assert!(TIMEOUT_RESULT_V1.ends_with(".schema.json"));
        assert!(PROCESS_INFO_V1.ends_with(".schema.json"));
        assert!(PROCESS_INFO_SAMPLED_V1.ends_with(".schema.json"));
        assert!(PROC_FILTER_V1.ends_with(".schema.json"));
        assert!(PORT_BINDINGS_V1.ends_with(".schema.json"));
        assert!(PORT_FILTER_V1.ends_with(".schema.json"));
        assert!(FD_SNAPSHOT_V1.ends_with(".schema.json"));
        assert!(FD_FILTER_V1.ends_with(".schema.json"));
        assert!(WAIT_PID_RESULT_V1.ends_with(".schema.json"));
        assert!(BATCH_KILL_RESULT_V1.ends_with(".schema.json"));
        assert!(TERMINATE_TREE_CONFIG_V1.ends_with(".schema.json"));
        assert!(TERMINATE_TREE_RESULT_V1.ends_with(".schema.json"));
        assert!(SPAWN_IN_GROUP_CONFIG_V1.ends_with(".schema.json"));
        assert!(SPAWN_IN_GROUP_RESULT_V1.ends_with(".schema.json"));
        assert!(DESCENDANTS_RESULT_V1.ends_with(".schema.json"));

        // Process snapshot schemas are v1.1.0 (additive ProcessInfo fields).
        assert!(PROCESS_INFO_V1.contains("/v1.1.0/"));
        assert!(PROCESS_INFO_SAMPLED_V1.contains("/v1.1.0/"));

        // Remaining schemas are currently v1.0.0.
        assert!(TIMEOUT_RESULT_V1.contains("/v1.0.0/"));
        assert!(PROC_FILTER_V1.contains("/v1.0.0/"));
        assert!(PORT_BINDINGS_V1.contains("/v1.0.0/"));
        assert!(PORT_FILTER_V1.contains("/v1.0.0/"));
        assert!(FD_SNAPSHOT_V1.contains("/v1.0.0/"));
        assert!(FD_FILTER_V1.contains("/v1.0.0/"));
        assert!(WAIT_PID_RESULT_V1.contains("/v1.0.0/"));
        assert!(BATCH_KILL_RESULT_V1.contains("/v1.0.0/"));
        assert!(TERMINATE_TREE_CONFIG_V1.contains("/v1.0.0/"));
        assert!(TERMINATE_TREE_RESULT_V1.contains("/v1.0.0/"));
        assert!(SPAWN_IN_GROUP_CONFIG_V1.contains("/v1.0.0/"));
        assert!(SPAWN_IN_GROUP_RESULT_V1.contains("/v1.0.0/"));
        assert!(DESCENDANTS_RESULT_V1.contains("/v1.0.0/"));
    }

    #[test]
    fn test_schema_ids_have_correct_topics() {
        // Verify topic segments are correct
        assert!(
            TIMEOUT_RESULT_V1.contains("/timeout/"),
            "timeout schema should have timeout topic"
        );
        assert!(
            PROCESS_INFO_V1.contains("/process/"),
            "process-info schema should have process topic"
        );
        assert!(
            PROCESS_INFO_SAMPLED_V1.contains("/process/"),
            "process-info-sampled schema should have process topic"
        );
        assert!(
            PROC_FILTER_V1.contains("/process/"),
            "process-filter schema should have process topic"
        );
        assert!(
            PORT_BINDINGS_V1.contains("/process/"),
            "port-bindings schema should have process topic"
        );
        assert!(
            PORT_FILTER_V1.contains("/process/"),
            "port-filter schema should have process topic"
        );
        assert!(
            FD_SNAPSHOT_V1.contains("/process/"),
            "fd-snapshot schema should have process topic"
        );
        assert!(
            FD_FILTER_V1.contains("/process/"),
            "fd-filter schema should have process topic"
        );
        assert!(
            WAIT_PID_RESULT_V1.contains("/process/"),
            "wait-pid-result schema should have process topic"
        );
        assert!(
            BATCH_KILL_RESULT_V1.contains("/signal/"),
            "batch-kill-result schema should have signal topic"
        );
        assert!(
            TERMINATE_TREE_CONFIG_V1.contains("/process/"),
            "terminate-tree-config schema should have process topic"
        );
        assert!(
            TERMINATE_TREE_RESULT_V1.contains("/process/"),
            "terminate-tree-result schema should have process topic"
        );
        assert!(
            SPAWN_IN_GROUP_CONFIG_V1.contains("/process/"),
            "spawn-in-group-config schema should have process topic"
        );
        assert!(
            SPAWN_IN_GROUP_RESULT_V1.contains("/process/"),
            "spawn-in-group-result schema should have process topic"
        );
        assert!(
            DESCENDANTS_RESULT_V1.contains("/process/"),
            "descendants-result schema should have process topic"
        );
    }

    #[test]
    fn test_schema_ids_are_unique() {
        let ids = [
            TIMEOUT_RESULT_V1,
            PROCESS_INFO_V1,
            PROCESS_INFO_SAMPLED_V1,
            PROC_FILTER_V1,
            PORT_BINDINGS_V1,
            PORT_FILTER_V1,
            FD_SNAPSHOT_V1,
            FD_FILTER_V1,
            WAIT_PID_RESULT_V1,
            BATCH_KILL_RESULT_V1,
            TERMINATE_TREE_CONFIG_V1,
            TERMINATE_TREE_RESULT_V1,
            SPAWN_IN_GROUP_CONFIG_V1,
            SPAWN_IN_GROUP_RESULT_V1,
            DESCENDANTS_RESULT_V1,
        ];

        // Check all pairs are different
        for (i, a) in ids.iter().enumerate() {
            for (j, b) in ids.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "Schema IDs must be unique");
                }
            }
        }
    }

    #[test]
    fn test_schema_host_constants() {
        assert_eq!(SCHEMA_HOST, "https://schemas.3leaps.dev");
        assert_eq!(SCHEMA_MODULE, "sysprims");

        // All schema IDs should start with host/module
        let prefix = format!("{}/{}/", SCHEMA_HOST, SCHEMA_MODULE);
        assert!(TIMEOUT_RESULT_V1.starts_with(&prefix));
        assert!(PROCESS_INFO_V1.starts_with(&prefix));
        assert!(PROCESS_INFO_SAMPLED_V1.starts_with(&prefix));
        assert!(PROC_FILTER_V1.starts_with(&prefix));
        assert!(PORT_BINDINGS_V1.starts_with(&prefix));
        assert!(PORT_FILTER_V1.starts_with(&prefix));
        assert!(FD_SNAPSHOT_V1.starts_with(&prefix));
        assert!(FD_FILTER_V1.starts_with(&prefix));
        assert!(WAIT_PID_RESULT_V1.starts_with(&prefix));
        assert!(BATCH_KILL_RESULT_V1.starts_with(&prefix));
        assert!(TERMINATE_TREE_CONFIG_V1.starts_with(&prefix));
        assert!(TERMINATE_TREE_RESULT_V1.starts_with(&prefix));
        assert!(SPAWN_IN_GROUP_CONFIG_V1.starts_with(&prefix));
        assert!(SPAWN_IN_GROUP_RESULT_V1.starts_with(&prefix));
        assert!(DESCENDANTS_RESULT_V1.starts_with(&prefix));
    }
}
