// SPDX-License-Identifier: MIT OR Apache-2.0

// Package main implements the pure-Go timeout and containment conformance runner,
// emitting RFC 8785 JCS canonical NDJSON lines to compare byte-for-byte with
// Rust, Python, and TypeScript.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"time"

	"github.com/3leaps/canonprims/canon"
	"github.com/3leaps/sysprims/syserror"
	"github.com/3leaps/sysprims/timeout"
)

func errorCodeSlug(err error) string {
	if err == nil {
		return "ok"
	}
	if code, ok := syserror.CodeOf(err); ok {
		return code.Category()
	}
	return "internal"
}

func emit(v any) {
	raw, err := json.Marshal(v)
	if err != nil {
		fmt.Fprintf(os.Stderr, "marshal failed: %v\n", err)
		os.Exit(2)
	}
	canonical, err := canon.Canonicalize(canon.JCS, raw, canon.DefaultLimits())
	if err != nil {
		fmt.Fprintf(os.Stderr, "jcs failed: %v\n", err)
		os.Exit(2)
	}
	fmt.Println(string(canonical))
}

func toStringSlice(v any) []string {
	arr, ok := v.([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(arr))
	for _, item := range arr {
		out = append(out, item.(string))
	}
	return out
}

func defaultVectorsPath() string {
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		panic("runtime.Caller failed")
	}
	// timeout/internal/difftool/main.go -> repo root is 3 dirs up
	repoRoot := filepath.Clean(filepath.Join(filepath.Dir(file), "..", "..", ".."))
	return filepath.Join(repoRoot, "testdata", "conformance", "timeout", "vectors.json")
}

func main() {
	vectorsPath := os.Getenv("TIMEOUT_CONFORMANCE_VECTORS")
	if vectorsPath == "" {
		if len(os.Args) > 1 {
			vectorsPath = os.Args[1]
		} else {
			vectorsPath = defaultVectorsPath()
		}
	}

	raw, err := os.ReadFile(filepath.Clean(vectorsPath)) // #nosec G304 G703 -- CLI test runner reads user-specified conformance vectors path
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to read %s: %v\n", vectorsPath, err)
		os.Exit(2)
	}

	var doc map[string]any
	if err := json.Unmarshal(raw, &doc); err != nil {
		fmt.Fprintf(os.Stderr, "failed to parse vectors.json: %v\n", err)
		os.Exit(2)
	}

	// 1. Schema constants
	emit(map[string]any{
		"suite":                       "schemas",
		"timeout_result_v1_1":         timeout.TimeoutResultSchemaID,
		"terminate_tree_config_v1":    timeout.TerminateTreeConfigSchemaID,
		"terminate_tree_result_v1":    timeout.TerminateTreeResultSchemaID,
		"containment_spawn_config_v1": timeout.ContainmentSpawnConfigSchemaID,
		"containment_snapshot_v1":     timeout.ContainmentSnapshotSchemaID,
	})

	// 2. Terminate tree validation cases
	for _, item := range doc["terminate_tree_validation_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		pidFloat := c["pid"].(float64)
		expected := c["expected"].(string)

		var outcome string
		if pidFloat < 0 || pidFloat > float64(^uint32(0)) {
			outcome = "invalid_argument"
		} else {
			pid := uint32(pidFloat)
			var cfg timeout.TerminateTreeConfig
			if s, ok := c["schema_id"].(string); ok {
				cfg.SchemaID = s
			}
			if gt, ok := c["grace_timeout_ms"].(float64); ok {
				v := uint64(gt)
				cfg.GraceTimeoutMS = &v
			}
			if kt, ok := c["kill_timeout_ms"].(float64); ok {
				v := uint64(kt)
				cfg.KillTimeoutMS = &v
			}
			if sig, ok := c["signal"].(float64); ok {
				v := int32(sig)
				cfg.Signal = &v
			}
			if ksig, ok := c["kill_signal"].(float64); ok {
				v := int32(ksig)
				cfg.KillSignal = &v
			}
			_, callErr := timeout.TerminateTree(pid, &cfg)
			outcome = errorCodeSlug(callErr)
		}
		if outcome != expected {
			fmt.Fprintf(os.Stderr, "case %s: got %s, expected %s\n", id, outcome, expected)
			os.Exit(1)
		}
		emit(map[string]any{
			"suite":   "terminate-tree-validation",
			"id":      id,
			"outcome": outcome,
		})
	}

	// 3. Spawn contained validation cases
	for _, item := range doc["spawn_contained_validation_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		argv := toStringSlice(c["argv"])
		expected := c["expected"].(string)

		cfg := timeout.SpawnContainedConfig{
			Argv: argv,
		}
		if cwd, ok := c["cwd"].(string); ok {
			cfg.Cwd = &cwd
		}
		if et, ok := c["execution_timeout_ms"].(float64); ok {
			v := uint64(et)
			cfg.ExecutionTimeoutMS = &v
		}
		if gt, ok := c["grace_timeout_ms"].(float64); ok {
			v := uint64(gt)
			cfg.GraceTimeoutMS = &v
		}
		if kt, ok := c["kill_timeout_ms"].(float64); ok {
			v := uint64(kt)
			cfg.KillTimeoutMS = &v
		}
		if sig, ok := c["signal"].(float64); ok {
			v := int32(sig)
			cfg.Signal = &v
		}
		if ksig, ok := c["kill_signal"].(float64); ok {
			v := int32(ksig)
			cfg.KillSignal = &v
		}

		h, callErr := timeout.SpawnContained(cfg)
		if callErr == nil {
			_ = h.Close()
		}
		outcome := errorCodeSlug(callErr)
		if outcome != expected {
			fmt.Fprintf(os.Stderr, "case %s: got %s, expected %s (err=%v)\n", id, outcome, expected, callErr)
			os.Exit(1)
		}
		emit(map[string]any{
			"suite":   "spawn-contained-validation",
			"id":      id,
			"outcome": outcome,
		})
	}

	// 4. Containment lifecycle cases
	for _, item := range doc["containment_lifecycle_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		op := c["op"].(string)
		argv := toStringSlice(c["argv"])
		expectedObj := c["expected_unix"].(map[string]any)
		if runtime.GOOS == "windows" {
			expectedObj = c["expected_windows"].(map[string]any)
		}

		cfg := timeout.SpawnContainedConfig{
			Argv: argv,
		}
		if et, ok := c["execution_timeout_ms"].(float64); ok {
			v := uint64(et)
			cfg.ExecutionTimeoutMS = &v
		}
		if gt, ok := c["grace_timeout_ms"].(float64); ok {
			v := uint64(gt)
			cfg.GraceTimeoutMS = &v
		}
		if kt, ok := c["kill_timeout_ms"].(float64); ok {
			v := uint64(kt)
			cfg.KillTimeoutMS = &v
		}
		waitTimeoutMS := uint64(5000)
		if wt, ok := c["wait_timeout_ms"].(float64); ok {
			waitTimeoutMS = uint64(wt)
		}

		var row map[string]any
		h, spawnErr := timeout.SpawnContained(cfg)
		if spawnErr != nil {
			row = map[string]any{
				"suite":   "containment-lifecycle",
				"id":      id,
				"outcome": errorCodeSlug(spawnErr),
			}
		} else {
			var snap *timeout.ContainmentSnapshot
			var opErr error
			switch op {
			case "wait", "deadline":
				snap, opErr = h.Wait(time.Duration(waitTimeoutMS) * time.Millisecond) // #nosec G115 -- waitTimeoutMS validated in conformance vector
			case "terminate":
				snap, opErr = h.Terminate()
			default:
				fmt.Fprintf(os.Stderr, "unknown op %s\n", op)
				os.Exit(1)
			}
			_ = h.Close()
			if opErr != nil {
				row = map[string]any{
					"suite":   "containment-lifecycle",
					"id":      id,
					"outcome": errorCodeSlug(opErr),
				}
			} else {
				row = map[string]any{
					"suite":                 "containment-lifecycle",
					"id":                    id,
					"outcome":               "ok",
					"handle_state":          snap.HandleState,
					"leader_status":         snap.LeaderStatus,
					"tree_kill_reliability": snap.TreeKillReliability,
					"boundary_strength":     snap.BoundaryStrength,
					"exited":                snap.Exited,
					"timed_out":             snap.TimedOut,
				}
			}
		}

		for k, want := range expectedObj {
			got := row[k]
			if wantBool, ok := want.(bool); ok {
				if gotPtr, ok := got.(*bool); ok && gotPtr != nil && *gotPtr == wantBool {
					continue
				}
			}
			if fmt.Sprintf("%v", got) != fmt.Sprintf("%v", want) {
				fmt.Fprintf(os.Stderr, "case %s field %s: got %v, expected %v\n", id, k, got, want)
				os.Exit(1)
			}
		}
		emit(row)
	}
}
