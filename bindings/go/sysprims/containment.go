package sysprims

/*
#include "sysprims.h"
#include <stdlib.h>
*/
import "C"
import (
	"encoding/json"
	"runtime"
	"sync/atomic"
	"time"
	"unsafe"
)

const containmentSpawnSchemaID = "https://schemas.3leaps.dev/sysprims/timeout/v1.0.0/containment-spawn-config.schema.json"

// SpawnContainedConfig configures sysprims-owned contained spawn.
//
// Argv is an argument vector, not a shell command string. The constructor is
// always contained; there is no foreground mode. Windows fails before spawn.
type SpawnContainedConfig struct {
	SchemaID            string            `json:"schema_id"`
	Argv                []string          `json:"argv"`
	Cwd                 *string           `json:"cwd,omitempty"`
	Env                 map[string]string `json:"env,omitempty"`
	ExecutionTimeoutMS  *uint64           `json:"execution_timeout_ms,omitempty"`
	GraceTimeoutMS      *uint64           `json:"grace_timeout_ms,omitempty"`
	KillTimeoutMS       *uint64           `json:"kill_timeout_ms,omitempty"`
	Signal              *int32            `json:"signal,omitempty"`
	KillSignal          *int32            `json:"kill_signal,omitempty"`
}

// ContainmentIdentity is diagnostic spawn evidence. PID is never authority.
type ContainmentIdentity struct {
	PID             uint32 `json:"pid"`
	StartTimeUnixMS uint64 `json:"start_time_unix_ms"`
	ExePath         string `json:"exe_path"`
}

// ContainmentCompletion is point-in-time membership evidence after cleanup.
type ContainmentCompletion struct {
	Status        string   `json:"status"`
	Observation   string   `json:"observation"`
	ObservedCount *uint32  `json:"observed_count,omitempty"`
	SurvivorPIDs  []uint32 `json:"survivor_pids,omitempty"`
}

// ContainmentSnapshot is a poll/wait/terminate result.
//
// Reliability and boundary_strength are immutable from spawn. Completion is
// independent of both. Survivor PIDs are evidence only.
type ContainmentSnapshot struct {
	SchemaID             string                 `json:"schema_id"`
	Timestamp            string                 `json:"timestamp"`
	Platform             string                 `json:"platform"`
	HandleState          string                 `json:"handle_state"`
	LeaderStatus         string                 `json:"leader_status"`
	Identity             ContainmentIdentity    `json:"identity"`
	TreeKillReliability  string                 `json:"tree_kill_reliability"`
	BoundaryStrength     string                 `json:"boundary_strength"`
	PGID                 *uint32                `json:"pgid,omitempty"`
	SignalSent           *int32                 `json:"signal_sent,omitempty"`
	KillSignal           *int32                 `json:"kill_signal,omitempty"`
	Escalated            *bool                  `json:"escalated,omitempty"`
	Exited               *bool                  `json:"exited,omitempty"`
	TimedOut             *bool                  `json:"timed_out,omitempty"`
	Completion           *ContainmentCompletion `json:"completion,omitempty"`
	Warnings             []string               `json:"warnings"`
}

// ContainedProcess is a Go projection of a native-owned containment handle.
//
// The wrapper stores only the scalar capability token. Close is deterministic;
// a finalizer is a leak backstop only.
type ContainedProcess struct {
	token atomic.Uint64
}

func containedFinalizer(handle *ContainedProcess) {
	token := handle.token.Swap(0)
	if token == 0 {
		return
	}
	_ = callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_containment_close(C.uint64_t(token))
	})
}

// SpawnContained asks sysprims to spawn and own a contained child.
func SpawnContained(config SpawnContainedConfig) (*ContainedProcess, error) {
	if config.SchemaID == "" {
		config.SchemaID = containmentSpawnSchemaID
	}
	if config.Argv == nil {
		config.Argv = []string{}
	}

	payload, err := json.Marshal(config)
	if err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to serialize config: " + err.Error()}
	}
	cCfg := C.CString(string(payload))
	defer C.free(unsafe.Pointer(cCfg))

	var handle C.uint64_t
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_containment_spawn(cCfg, &handle)
	}); err != nil {
		return nil, err
	}

	proc := &ContainedProcess{}
	proc.token.Store(uint64(handle))
	runtime.SetFinalizer(proc, containedFinalizer)
	return proc, nil
}

func (h *ContainedProcess) currentToken() (uint64, error) {
	if h == nil {
		return 0, &Error{Code: ErrInvalidArgument, Message: "containment handle is invalid"}
	}
	token := h.token.Load()
	if token == 0 {
		return 0, &Error{Code: ErrInvalidArgument, Message: "containment handle is stale or closed"}
	}
	return token, nil
}

func snapshotCall(call func(result **C.char) C.SysprimsErrorCode) (*ContainmentSnapshot, error) {
	var resultCStr *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return call(&resultCStr)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(resultCStr)

	var snapshot ContainmentSnapshot
	if err := json.Unmarshal([]byte(C.GoString(resultCStr)), &snapshot); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}
	return &snapshot, nil
}

// Identity returns immutable spawn evidence without consuming reap authority.
func (h *ContainedProcess) Identity() (*ContainmentSnapshot, error) {
	token, err := h.currentToken()
	if err != nil {
		return nil, err
	}
	return snapshotCall(func(result **C.char) C.SysprimsErrorCode {
		return C.sysprims_containment_identity(C.uint64_t(token), result)
	})
}

// Poll is non-destructive while the leader is running. If the leader has
// already exited, poll commits the terminal outcome.
func (h *ContainedProcess) Poll() (*ContainmentSnapshot, error) {
	token, err := h.currentToken()
	if err != nil {
		return nil, err
	}
	return snapshotCall(func(result **C.char) C.SysprimsErrorCode {
		return C.sysprims_containment_poll(C.uint64_t(token), result)
	})
}

// Wait waits for a terminal outcome or until timeout. A timeout of 0 waits
// until the native lifecycle finishes. The timeout only ends this wait.
func (h *ContainedProcess) Wait(timeout time.Duration) (*ContainmentSnapshot, error) {
	token, err := h.currentToken()
	if err != nil {
		return nil, err
	}
	var timeoutMS uint64
	if timeout > 0 {
		timeoutMS = uint64(timeout.Milliseconds())
	}
	return snapshotCall(func(result **C.char) C.SysprimsErrorCode {
		return C.sysprims_containment_wait(C.uint64_t(token), C.uint64_t(timeoutMS), result)
	})
}

// Terminate sends the configured grace/escalate policy once.
func (h *ContainedProcess) Terminate() (*ContainmentSnapshot, error) {
	token, err := h.currentToken()
	if err != nil {
		return nil, err
	}
	return snapshotCall(func(result **C.char) C.SysprimsErrorCode {
		return C.sysprims_containment_terminate(C.uint64_t(token), result)
	})
}

// Close releases the native registry entry. The second call is a no-op.
func (h *ContainedProcess) Close() error {
	if h == nil {
		return nil
	}
	token := h.token.Swap(0)
	if token == 0 {
		return nil
	}
	runtime.SetFinalizer(h, nil)
	return callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_containment_close(C.uint64_t(token))
	})
}
