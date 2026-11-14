// SPDX-License-Identifier: MIT OR Apache-2.0

// Package main implements the pure-Go session conformance runner, emitting RFC
// 8785 JCS canonical NDJSON lines to compare byte-for-byte with Rust, Python,
// and TypeScript.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"github.com/3leaps/canonprims/canon"
	"github.com/3leaps/sysprims/session"
	"github.com/3leaps/sysprims/syserror"
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
	// session/internal/difftool/main.go -> repo root is 3 dirs up
	repoRoot := filepath.Clean(filepath.Join(filepath.Dir(file), "..", "..", ".."))
	return filepath.Join(repoRoot, "testdata", "conformance", "session", "vectors.json")
}

func main() {
	vectorsPath := os.Getenv("SESSION_CONFORMANCE_VECTORS")
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
		"suite":                    "schemas",
		"run_setsid_config_v1":     session.RunSetsidConfigSchemaID,
		"run_nohup_config_v1":      session.RunNohupConfigSchemaID,
		"session_spawn_result_v1":  session.SessionSpawnResultSchemaID,
		"spawn_in_group_config_v1": session.SpawnInGroupConfigSchemaID,
		"spawn_in_group_result_v1": session.SpawnInGroupResultSchemaID,
	})

	// 2. Config validation cases
	for _, item := range doc["config_validation_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		verb := c["verb"].(string)
		schemaID := c["schema_id"].(string)
		argv := toStringSlice(c["argv"])
		expected := c["expected"].(string)

		var callErr error
		switch verb {
		case "setsid":
			_, callErr = session.RunSetsid(session.RunSetsidConfig{
				SchemaID: schemaID,
				Argv:     argv,
				Wait:     true,
			})
		case "nohup":
			_, callErr = session.RunNohup(session.RunNohupConfig{
				SchemaID: schemaID,
				Argv:     argv,
				Wait:     true,
			})
		case "spawn_in_group":
			_, callErr = session.SpawnInGroup(session.SpawnInGroupConfig{
				SchemaID: schemaID,
				Argv:     argv,
			})
		default:
			fmt.Fprintf(os.Stderr, "unknown verb %s\n", verb)
			os.Exit(1)
		}
		outcome := errorCodeSlug(callErr)
		if outcome != expected {
			fmt.Fprintf(os.Stderr, "case %s: got %s, expected %s (err=%v)\n", id, outcome, expected, callErr)
			os.Exit(1)
		}
		emit(map[string]any{
			"suite":   "config-validation",
			"id":      id,
			"verb":    verb,
			"outcome": outcome,
		})
	}

	// 3. Self introspection cases
	for _, item := range doc["self_introspection_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		op := c["op"].(string)
		expected := c["expected_unix"].(string)
		if runtime.GOOS == "windows" {
			expected = c["expected_windows"].(string)
		}
		var callErr error
		var val uint32
		switch op {
		case "self_getpgid":
			val, callErr = session.SelfPGID()
		case "self_getsid":
			val, callErr = session.SelfSID()
		default:
			fmt.Fprintf(os.Stderr, "unknown op %s\n", op)
			os.Exit(1)
		}
		outcome := errorCodeSlug(callErr)
		if callErr == nil && val == 0 {
			outcome = "invalid_value"
		}
		if outcome != expected {
			fmt.Fprintf(os.Stderr, "case %s: got %s, expected %s (err=%v)\n", id, outcome, expected, callErr)
			os.Exit(1)
		}
		emit(map[string]any{
			"suite":   "self-introspection",
			"id":      id,
			"op":      op,
			"outcome": outcome,
		})
	}

	// 4. Spawn cases
	for _, item := range doc["spawn_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		verb := c["verb"].(string)
		argv := toStringSlice(c["argv"])
		wait, _ := c["wait"].(bool)
		useTempOutput, _ := c["use_temp_output_file"].(bool)
		useSymlinkOutput, _ := c["use_symlink_output_file"].(bool)
		expectedObj := c["expected_unix"].(map[string]any)
		if runtime.GOOS == "windows" {
			expectedObj = c["expected_windows"].(map[string]any)
		}

		var row map[string]any
		switch verb {
		case "setsid":
			res, err := session.RunSetsid(session.RunSetsidConfig{
				Argv: argv,
				Wait: wait,
			})
			if err != nil {
				row = map[string]any{
					"suite":   "spawn",
					"id":      id,
					"outcome": errorCodeSlug(err),
				}
			} else {
				structuralMatch := res.PID != nil && res.SID != nil && res.PGID != nil &&
					*res.PID > 0 && *res.PID == *res.SID && *res.PID == *res.PGID
				row = map[string]any{
					"suite":                 "spawn",
					"id":                    id,
					"outcome":               "ok",
					"verb":                  res.Verb,
					"status":                res.Status,
					"session_kind":          string(res.SessionKind),
					"identifier_provenance": string(res.IdentifierProvenance),
					"structural_ids_match":  structuralMatch,
					"exit_code":             res.ExitCode,
				}
			}

		case "nohup":
			tempDir, err := os.MkdirTemp("", "sysprims-session-go-*")
			if err != nil {
				fmt.Fprintf(os.Stderr, "mkdirtemp failed: %v\n", err)
				os.Exit(2)
			}
			var outputFile *string
			if useSymlinkOutput {
				target := filepath.Join(tempDir, "target.log")
				link := filepath.Join(tempDir, "link.log")
				_ = os.WriteFile(target, []byte("existing"), 0o600)
				_ = os.Symlink(target, link)
				outputFile = &link
			} else if useTempOutput {
				outPath := filepath.Join(tempDir, "nohup.out")
				outputFile = &outPath
			}

			callerSID, _ := session.SelfSID()
			callerPGID, _ := session.SelfPGID()
			res, callErr := session.RunNohup(session.RunNohupConfig{
				Argv:       argv,
				Wait:       wait,
				OutputFile: outputFile,
			})
			_ = os.RemoveAll(tempDir)
			if callErr != nil {
				row = map[string]any{
					"suite":   "spawn",
					"id":      id,
					"outcome": errorCodeSlug(callErr),
				}
			} else {
				inheritedMatch := res.PID != nil && *res.PID > 0 &&
					res.SID != nil && *res.SID == callerSID &&
					res.PGID != nil && *res.PGID == callerPGID
				row = map[string]any{
					"suite":                     "spawn",
					"id":                        id,
					"outcome":                   "ok",
					"verb":                      res.Verb,
					"status":                    res.Status,
					"session_kind":              string(res.SessionKind),
					"identifier_provenance":     string(res.IdentifierProvenance),
					"inherited_context_matches": inheritedMatch,
					"exit_code":                 res.ExitCode,
				}
			}

		case "spawn_in_group":
			res, err := session.SpawnInGroup(session.SpawnInGroupConfig{
				Argv: argv,
			})
			if err != nil {
				row = map[string]any{
					"suite":   "spawn",
					"id":      id,
					"outcome": errorCodeSlug(err),
				}
			} else {
				structuralMatch := res.PID > 0 && res.PGID != nil && *res.PGID == res.PID
				row = map[string]any{
					"suite":                 "spawn",
					"id":                    id,
					"outcome":               "ok",
					"structural_ids_match":  structuralMatch,
					"tree_kill_reliability": res.TreeKillReliability,
				}
			}
		}

		for k, want := range expectedObj {
			got := row[k]
			if wantNum, ok := want.(float64); ok {
				if gotPtr, ok := got.(*int32); ok && gotPtr != nil && float64(*gotPtr) == wantNum {
					continue
				}
			}
			if fmt.Sprintf("%v", got) != fmt.Sprintf("%v", want) {
				fmt.Fprintf(os.Stderr, "spawn case %s.%s: got %#v, expected %#v\n", id, k, got, want)
				os.Exit(1)
			}
		}
		emit(row)
	}
}
