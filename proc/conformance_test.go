package proc_test

import (
	"encoding/json"
	"math"
	"os"
	"path/filepath"
	"slices"
	"testing"

	"github.com/3leaps/sysprims/proc"
	"github.com/3leaps/sysprims/syserror"
)

func findVectorsPath(t *testing.T) string {
	t.Helper()
	if envPath := os.Getenv("PROC_CONFORMANCE_VECTORS"); envPath != "" {
		return envPath
	}
	return filepath.Join("..", "testdata", "conformance", "proc", "vectors.json")
}

func TestConformanceVectors(t *testing.T) {
	path := findVectorsPath(t)
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("failed to read %s: %v", path, err)
	}

	var doc struct {
		SyntheticTree proc.DescendantsResult `json:"synthetic_tree"`
		Vectors       []struct {
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
		} `json:"vectors"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		t.Fatalf("failed to unmarshal %s: %v", path, err)
	}

	for _, v := range doc.Vectors {
		t.Run(v.ID, func(t *testing.T) {
			switch v.Kind {
			case "validate_pid":
				pid := uint32(math.MaxUint32)
				if v.PID <= math.MaxUint32 {
					pid = uint32(v.PID)
				}
				_, err := proc.GetProcess(pid)
				code, ok := syserror.CodeOf(err)
				if !ok || code.Category() != v.ExpectedErrorCategory {
					t.Fatalf("got (%v, %v), want category %q", code, err, v.ExpectedErrorCategory)
				}
			case "validate_filter":
				err := v.Filter.Validate()
				code, ok := syserror.CodeOf(err)
				if !ok || code.Category() != v.ExpectedErrorCategory {
					t.Fatalf("got (%v, %v), want category %q", code, err, v.ExpectedErrorCategory)
				}
			case "validate_port_filter":
				proto := proc.ProtocolTCP
				pf := &proc.PortFilter{Protocol: &proto, LocalPort: &v.LocalPort}
				_, err := proc.ListeningPorts(pf)
				code, ok := syserror.CodeOf(err)
				if !ok || code.Category() != v.ExpectedErrorCategory {
					t.Fatalf("got (%v, %v), want category %q", code, err, v.ExpectedErrorCategory)
				}
			case "guard_preset":
				p := proc.GuardPreset(v.Preset)
				if uint64(p.Interval().Milliseconds()) != uint64(v.Expected["interval_ms"].(float64)) {
					t.Fatalf("interval_ms mismatch for %s", v.Preset)
				}
				if uint64(p.SampleDuration().Milliseconds()) != uint64(v.Expected["sample_duration_ms"].(float64)) {
					t.Fatalf("sample_duration_ms mismatch for %s", v.Preset)
				}
			case "select_targets":
				got, err := proc.SelectDescendantTargets(&doc.SyntheticTree, v.Filter, v.Cascade)
				if err != nil {
					t.Fatalf("SelectDescendantTargets failed: %v", err)
				}
				if !slices.Equal(got, v.ExpectedPIDs) {
					t.Fatalf("got %v, want %v", got, v.ExpectedPIDs)
				}
			case "live_probe_self":
				selfPID := uint32(os.Getpid())
				info, err := proc.GetProcess(selfPID)
				if err != nil || info.PID != selfPID {
					t.Fatalf("GetProcess(self) = (%+v, %v)", info, err)
				}
			default:
				t.Fatalf("unknown vector kind: %s", v.Kind)
			}
		})
	}
}
