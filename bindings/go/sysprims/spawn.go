package sysprims

/*
#include "sysprims.h"
*/
import "C"

import (
	"encoding/json"
	"unsafe"
)

// SpawnInGroupConfig spawns a process in a new process group (Unix) or Job Object (Windows).
//
// Env is treated as overrides/additions to the inherited environment.
type SpawnInGroupConfig struct {
	SchemaID string            `json:"schema_id"`
	Argv     []string          `json:"argv"`
	Cwd      *string           `json:"cwd,omitempty"`
	Env      map[string]string `json:"env,omitempty"`
}

// SpawnInGroupResult is the outcome of SpawnInGroup.
type SpawnInGroupResult struct {
	SchemaID            string   `json:"schema_id"`
	Timestamp           string   `json:"timestamp"`
	Platform            string   `json:"platform"`
	PID                 uint32   `json:"pid"`
	PGID                *uint32  `json:"pgid,omitempty"`
	TreeKillReliability string   `json:"tree_kill_reliability"`
	Warnings            []string `json:"warnings"`
}

func SpawnInGroup(config SpawnInGroupConfig) (*SpawnInGroupResult, error) {
	if config.SchemaID == "" {
		config.SchemaID = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/spawn-in-group-config.schema.json"
	}

	b, err := json.Marshal(config)
	if err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to serialize config: " + err.Error()}
	}

	cCfg := C.CString(string(b))
	defer C.free(unsafe.Pointer(cCfg))

	var out *C.char
	if err := callAndCheck(func() C.SysprimsErrorCode {
		return C.sysprims_spawn_in_group(cCfg, &out)
	}); err != nil {
		return nil, err
	}
	defer C.sysprims_free_string(out)

	var result SpawnInGroupResult
	if err := json.Unmarshal([]byte(C.GoString(out)), &result); err != nil {
		return nil, &Error{Code: ErrInternal, Message: "failed to parse response: " + err.Error()}
	}

	return &result, nil
}
