// SPDX-License-Identifier: MIT OR Apache-2.0

// Package main implements the pure-Go proc conformance runner, emitting RFC
// 8785 JCS canonical NDJSON lines to compare byte-for-byte with Rust, Python,
// and TypeScript.
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"runtime"
	"slices"

	"github.com/3leaps/canonprims/canon"
	"github.com/3leaps/sysprims/proc"
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

func defaultVectorsPath() string {
	if envPath := os.Getenv("PROC_CONFORMANCE_VECTORS"); envPath != "" {
		return envPath
	}
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		panic("runtime.Caller failed")
	}
	return filepath.Clean(filepath.Join(filepath.Dir(file), "..", "..", "..", "testdata", "conformance", "proc", "vectors.json"))
}

type vectorsDoc struct {
	SyntheticTree proc.DescendantsResult `json:"synthetic_tree"`
	Vectors       []vectorEntry          `json:"vectors"`
}

type vectorEntry struct {
	ID                    string              `json:"id"`
	Kind                  string              `json:"kind"`
	PID                   uint64              `json:"pid"`
	Filter                *proc.ProcessFilter `json:"filter"`
	LocalPort             uint16              `json:"local_port"`
	Preset                string              `json:"preset"`
	Cascade               bool                `json:"cascade"`
	ExpectedErrorCategory string              `json:"expected_error_category"`
	ExpectedPIDs          []uint32            `json:"expected_pids"`
	Expected              map[string]any      `json:"expected"`
}

func main() {
	vectorsPath := defaultVectorsPath()
	if len(os.Args) > 1 {
		vectorsPath = os.Args[1]
	}

	raw, err := os.ReadFile(vectorsPath) // #nosec G304 G703 -- test harness reads conformance vector file path
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to read %s: %v\n", vectorsPath, err)
		os.Exit(2)
	}
	var doc vectorsDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		fmt.Fprintf(os.Stderr, "failed to parse %s: %v\n", vectorsPath, err)
		os.Exit(2)
	}

	// 1. Schema constants
	emit(map[string]any{
		"suite":                         "schemas",
		"process_info_v1":               proc.ProcessInfoSchemaID,
		"process_info_sampled_v1":       proc.ProcessInfoSampledSchemaID,
		"descendants_result_v1":         proc.DescendantsResultSchemaID,
		"descendants_result_sampled_v1": proc.DescendantsResultSampledSchemaID,
		"ancestors_result_v1":           proc.AncestorsResultSchemaID,
		"wait_pid_result_v1":            proc.WaitPidResultSchemaID,
		"fd_snapshot_v1":                proc.FdSnapshotSchemaID,
		"port_bindings_v1":              proc.PortBindingsSchemaID,
		"port_filter_v1":                proc.PortFilterSchemaID,
		"guard_event_v1":                proc.GuardEventSchemaID,
	})

	for _, v := range doc.Vectors {
		switch v.Kind {
		case "validate_pid":
			pid := uint32(math.MaxUint32)
			if v.PID <= math.MaxUint32 {
				pid = uint32(v.PID)
			}
			_, err := proc.GetProcess(pid)
			slug := errorCodeSlug(err)
			if slug != v.ExpectedErrorCategory {
				fmt.Fprintf(os.Stderr, "vector %s mismatch: got %s, want %s\n", v.ID, slug, v.ExpectedErrorCategory)
				os.Exit(1)
			}
			emit(map[string]any{
				"id":      v.ID,
				"kind":    v.Kind,
				"outcome": slug,
			})
		case "validate_filter":
			err := v.Filter.Validate()
			slug := errorCodeSlug(err)
			if slug != v.ExpectedErrorCategory {
				fmt.Fprintf(os.Stderr, "vector %s mismatch: got %s, want %s\n", v.ID, slug, v.ExpectedErrorCategory)
				os.Exit(1)
			}
			emit(map[string]any{
				"id":      v.ID,
				"kind":    v.Kind,
				"outcome": slug,
			})
		case "validate_port_filter":
			proto := proc.ProtocolTCP
			pf := &proc.PortFilter{
				Protocol:  &proto,
				LocalPort: &v.LocalPort,
			}
			_, err := proc.ListeningPorts(pf)
			slug := errorCodeSlug(err)
			if slug != v.ExpectedErrorCategory {
				fmt.Fprintf(os.Stderr, "vector %s mismatch: got %s, want %s\n", v.ID, slug, v.ExpectedErrorCategory)
				os.Exit(1)
			}
			emit(map[string]any{
				"id":      v.ID,
				"kind":    v.Kind,
				"outcome": slug,
			})
		case "guard_preset":
			p := proc.GuardPreset(v.Preset)
			intervalMS := uint64(p.Interval().Milliseconds())     // #nosec G115 -- preset duration > 0
			sampleMS := uint64(p.SampleDuration().Milliseconds()) // #nosec G115 -- preset duration > 0
			wantInterval := uint64(v.Expected["interval_ms"].(float64))
			wantSample := uint64(v.Expected["sample_duration_ms"].(float64))
			if intervalMS != wantInterval || sampleMS != wantSample {
				fmt.Fprintf(os.Stderr, "vector %s mismatch: got (%d, %d), want (%d, %d)\n", v.ID, intervalMS, sampleMS, wantInterval, wantSample)
				os.Exit(1)
			}
			emit(map[string]any{
				"id":                 v.ID,
				"kind":               v.Kind,
				"preset":             v.Preset,
				"interval_ms":        intervalMS,
				"sample_duration_ms": sampleMS,
			})
		case "select_targets":
			gotPIDs, err := proc.SelectDescendantTargets(&doc.SyntheticTree, v.Filter, v.Cascade)
			if err != nil {
				fmt.Fprintf(os.Stderr, "vector %s unexpected error: %v\n", v.ID, err)
				os.Exit(1)
			}
			if !slices.Equal(gotPIDs, v.ExpectedPIDs) {
				fmt.Fprintf(os.Stderr, "vector %s mismatch: got %v, want %v\n", v.ID, gotPIDs, v.ExpectedPIDs)
				os.Exit(1)
			}
			emit(map[string]any{
				"id":           v.ID,
				"kind":         v.Kind,
				"cascade":      v.Cascade,
				"matched_pids": gotPIDs,
			})
		case "live_probe_self":
			selfPID := uint32(os.Getpid()) // #nosec G115 -- os.Getpid() > 0
			selfInfo, err := proc.GetProcess(selfPID)
			if err != nil {
				fmt.Fprintf(os.Stderr, "GetProcess(self) failed: %v\n", err)
				os.Exit(1)
			}
			anc, err := proc.Ancestors(selfPID, 3, nil)
			if err != nil {
				fmt.Fprintf(os.Stderr, "Ancestors(self) failed: %v\n", err)
				os.Exit(1)
			}
			desc, err := proc.Descendants(selfPID, 1, nil)
			if err != nil {
				fmt.Fprintf(os.Stderr, "Descendants(self) failed: %v\n", err)
				os.Exit(1)
			}
			maxLevels := uint32(1)
			guard, err := proc.GuardStep(&proc.GuardConfig{
				Rule: proc.GuardRule{
					RootPID:   selfPID,
					MaxLevels: &maxLevels,
					CpuMode:   proc.CpuModeLifetime,
				},
				ActionEnabled: false,
				MaxTargets:    8,
			})
			if err != nil {
				fmt.Fprintf(os.Stderr, "GuardStep(self) failed: %v\n", err)
				os.Exit(1)
			}
			emit(map[string]any{
				"id":                        v.ID,
				"kind":                      v.Kind,
				"ancestors_chain_non_empty": len(anc.Chain) > 0 && anc.Chain[0].PID == selfPID,
				"ancestors_schema_id":       anc.SchemaID,
				"descendants_schema_id":     desc.SchemaID,
				"fd_snapshot_schema_id":     proc.FdSnapshotSchemaID,
				"guard_event_schema_id":     guard.SchemaID,
				"port_bindings_schema_id":   proc.PortBindingsSchemaID,
				"process_info_schema_id":    proc.ProcessInfoSchemaID,
				"self_found":                selfInfo.PID == selfPID && selfInfo.Name != "",
			})
		default:
			fmt.Fprintf(os.Stderr, "unknown vector kind: %s\n", v.Kind)
			os.Exit(2)
		}
	}
}
