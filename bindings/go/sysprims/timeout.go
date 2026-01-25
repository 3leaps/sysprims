package sysprims

/*
#include "sysprims.h"
#include <stdlib.h>
*/
import "C"
import (
	"encoding/json"
	"time"
	"unsafe"
)

// GroupingMode controls process group creation for timeout execution.
type GroupingMode int32

const (
	// GroupByDefault creates a new process group (Unix) or Job Object (Windows)
	// to enable tree-kill on timeout. This is the recommended default.
	GroupByDefault GroupingMode = 0
	// Foreground runs without creating a new process group.
	// Only the direct child is killed on timeout; grandchildren may survive.
	Foreground GroupingMode = 1
)

// TimeoutConfig configures the behavior of [RunWithTimeout].
type TimeoutConfig struct {
	// Signal is the signal to send on timeout (default: SIGTERM).
	Signal int
	// KillAfter is the delay before escalating to SIGKILL if the process
	// doesn't terminate. Set to 0 for immediate escalation.
	KillAfter time.Duration
	// Grouping controls process group creation for tree-kill.
	Grouping GroupingMode
	// PreserveStatus causes the function to return the child's exit code
	// when the command completes (instead of always returning 0 for success).
	PreserveStatus bool
}

// DefaultTimeoutConfig returns sensible defaults for timeout execution.
//
// Defaults:
//   - Signal: SIGTERM
//   - KillAfter: 10 seconds
//   - Grouping: GroupByDefault
//   - PreserveStatus: false
func DefaultTimeoutConfig() TimeoutConfig {
	return TimeoutConfig{
		Signal:         SIGTERM,
		KillAfter:      10 * time.Second,
		Grouping:       GroupByDefault,
		PreserveStatus: false,
	}
}

// TimeoutResult represents the outcome of a timeout execution.
type TimeoutResult struct {
	// SchemaID identifies the JSON schema version.
	SchemaID string `json:"schema_id"`
	// Status is either "completed" or "timed_out".
	Status string `json:"status"`
	// ExitCode is the exit code if the command completed (nil if timed out).
	ExitCode *int `json:"exit_code,omitempty"`
	// SignalSent is the signal sent if the command timed out (nil if completed).
	SignalSent *int `json:"signal_sent,omitempty"`
	// Escalated indicates whether escalation to SIGKILL occurred (nil if completed).
	Escalated *bool `json:"escalated,omitempty"`
	// TreeKillReliability indicates tree-kill reliability: "guaranteed" or "best_effort".
	// Only present if the command timed out. "best_effort" indicates that on Windows,
	// Job Object creation may have failed and some child processes might have escaped.
	TreeKillReliability *string `json:"tree_kill_reliability,omitempty"`
}

// -----------------------------------------------------------------------------
// TerminateTree
// -----------------------------------------------------------------------------

// TerminateTreeConfig configures terminate-tree behavior.
//
// JSON keys use snake_case to match the schema/FFI conventions.
type TerminateTreeConfig struct {
	SchemaID string `json:"schema_id"`

	GraceTimeoutMS *uint64 `json:"grace_timeout_ms,omitempty"`
	KillTimeoutMS  *uint64 `json:"kill_timeout_ms,omitempty"`
	Signal         *int32  `json:"signal,omitempty"`
	KillSignal     *int32  `json:"kill_signal,omitempty"`
}

// TerminateTreeResult is the outcome of a terminate-tree operation.
type TerminateTreeResult struct {
	SchemaID            string   `json:"schema_id"`
	Timestamp           string   `json:"timestamp"`
	Platform            string   `json:"platform"`
	PID                 uint32   `json:"pid"`
	PGID                *uint32  `json:"pgid,omitempty"`
	SignalSent          int32    `json:"signal_sent"`
	KillSignal          *int32   `json:"kill_signal,omitempty"`
	Escalated           bool     `json:"escalated"`
	Exited              bool     `json:"exited"`
	TimedOut            bool     `json:"timed_out"`
	TreeKillReliability string   `json:"tree_kill_reliability"`
	Warnings            []string `json:"warnings"`
}

// Completed returns true if the command completed without timing out.
func (r *TimeoutResult) Completed() bool {
	return r.Status == "completed"
}

// TimedOut returns true if the command was terminated due to timeout.
func (r *TimeoutResult) TimedOut() bool {
	return r.Status == "timed_out"
}

// RunWithTimeout executes a command with a timeout.
//
// If the command doesn't complete within the timeout, it is killed.
// When using [GroupByDefault] (the default), the entire process tree is killed.
//
// # Arguments
//
//   - command: The command to execute
//   - args: Command arguments (may be nil or empty)
//   - timeout: Maximum duration to wait for the command
//   - config: Execution configuration (use [DefaultTimeoutConfig] for sensible defaults)
//
// # Example
//
//	result, err := sysprims.RunWithTimeout(
//	    "make", []string{"build"},
//	    5*time.Minute,
//	    sysprims.DefaultTimeoutConfig(),
//	)
//	if err != nil {
//	    log.Fatal(err)
//	}
//	if result.TimedOut() {
//	    log.Println("Build timed out!")
//	}
//
// # Errors
//
//   - [ErrInvalidArgument]: Invalid command or configuration
//   - [ErrSpawnFailed]: Failed to spawn the command
//   - [ErrNotFound]: Command not found
//   - [ErrPermissionDenied]: Command not executable
func RunWithTimeout(command string, args []string, timeout time.Duration, config TimeoutConfig) (*TimeoutResult, error) {
	// Prepare command string
	cCommand := C.CString(command)
	defer C.free(unsafe.Pointer(cCommand))

	// Prepare args array - allocate in C memory to avoid Go pointer issues
	var cArgs **C.char
	var cArgPtrs []unsafe.Pointer // Track allocations for cleanup
	if len(args) > 0 {
		// Allocate array of pointers in C memory
		argsSize := C.size_t(len(args)) * C.size_t(unsafe.Sizeof((*C.char)(nil)))
		cArgsPtr := C.malloc(argsSize)
		if cArgsPtr == nil {
			return nil, &Error{Code: ErrInternal, Message: "failed to allocate args array"}
		}
		defer C.free(cArgsPtr)

		// Convert to Go slice for indexing (but memory is C-allocated)
		cArgsArray := (*[1 << 30]*C.char)(cArgsPtr)[:len(args):len(args)]

		// Allocate each arg string and store in C array
		cArgPtrs = make([]unsafe.Pointer, len(args))
		for i, arg := range args {
			cStr := C.CString(arg)
			cArgPtrs[i] = unsafe.Pointer(cStr)
			cArgsArray[i] = cStr
		}
		defer func() {
			for _, ptr := range cArgPtrs {
				C.free(ptr)
			}
		}()

		cArgs = (**C.char)(cArgsPtr)
	}

	// Build C config struct
	cConfig := C.SysprimsTimeoutConfig{
		command:         cCommand,
		args:            cArgs,
		args_len:        C.uintptr_t(len(args)),
		timeout_ms:      C.uint64_t(timeout.Milliseconds()),
		kill_after_ms:   C.uint64_t(config.KillAfter.Milliseconds()),
		signal:          C.int32_t(config.Signal),
		grouping:        C.SysprimsGroupingMode(config.Grouping),
		preserve_status: C.bool(config.PreserveStatus),
	}

	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_timeout_run(&cConfig, &resultCStr)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var result TimeoutResult
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &result); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &result, nil
}

// TerminateTree sends a graceful signal, waits, then escalates to kill.
//
// This is intended for supervisor stop flows:
// - send TERM
// - wait
// - send KILL
//
// On Unix, if the target PID is a process group leader, sysprims will prefer
// group kill for better coverage.
func TerminateTree(pid uint32, config TerminateTreeConfig) (*TerminateTreeResult, error) {
	if config.SchemaID == "" {
		config.SchemaID = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-config.schema.json"
	}

	configJSON, err := json.Marshal(config)
	if err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to serialize config: " + err.Error()}
	}

	configCStr := C.CString(string(configJSON))
	defer C.free(unsafe.Pointer(configCStr))

	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_terminate_tree(C.uint32_t(pid), configCStr, &resultCStr)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var result TerminateTreeResult
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &result); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &result, nil
}
