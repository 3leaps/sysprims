// SPDX-License-Identifier: MIT OR Apache-2.0

// Command difftool emits deterministic RFC 8785 NDJSON evaluation records over
// testdata/conformance/signal/vectors.json for cross-language parity
// verification against crates/sysprims-signal, Python, and TypeScript.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"

	"github.com/3leaps/canonprims/canon"
	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
)

func emitLine(record map[string]any) {
	raw, err := json.Marshal(record)
	if err != nil {
		panic(fmt.Sprintf("marshal NDJSON record: %v", err))
	}
	canonical, err := canon.Canonicalize(canon.JCS, raw, canon.DefaultLimits())
	if err != nil {
		panic(fmt.Sprintf("canonicalize NDJSON record: %v", err))
	}
	fmt.Println(string(canonical))
}

func errorCodeSlug(err error) string {
	code, ok := syserror.CodeOf(err)
	if !ok {
		panic(fmt.Sprintf("expected *syserror.Error, got %T (%v)", err, err))
	}
	return code.Category()
}

func canonicalSignalNumber(name string) int {
	switch name {
	case "SIGHUP":
		return signal.SIGHUP
	case "SIGINT":
		return signal.SIGINT
	case "SIGQUIT":
		return signal.SIGQUIT
	case "SIGKILL":
		return signal.SIGKILL
	case "SIGPIPE":
		return signal.SIGPIPE
	case "SIGALRM":
		return signal.SIGALRM
	case "SIGTERM":
		return signal.SIGTERM
	case "SIGUSR1":
		return signal.SIGUSR1
	case "SIGUSR2":
		return signal.SIGUSR2
	default:
		panic("unknown canonical signal name: " + name)
	}
}

func defaultVectorsPath() string {
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		panic("runtime.Caller failed")
	}
	// signal/internal/difftool/main.go -> repo root is 3 dirs up
	repoRoot := filepath.Clean(filepath.Join(filepath.Dir(file), "..", "..", ".."))
	return filepath.Join(repoRoot, "testdata", "conformance", "signal", "vectors.json")
}

func main() {
	if len(os.Args) == 3 && os.Args[1] == "--live-pid" {
		pid64, err := strconv.ParseUint(os.Args[2], 10, 32)
		if err != nil {
			panic(fmt.Sprintf("invalid --live-pid %q: %v", os.Args[2], err))
		}
		handle, err := signal.OpenProcessHandle(uint32(pid64))
		if err != nil {
			panic(fmt.Sprintf("open live ProcessHandle: %v", err))
		}
		defer func() {
			_ = handle.Close()
		}()
		exited, err := handle.Exited()
		if err != nil {
			panic(fmt.Sprintf("poll live ProcessHandle Exited: %v", err))
		}
		emitLine(map[string]any{
			"pid":            handle.PID(),
			"start_identity": handle.StartIdentity(),
			"binding":        string(handle.Binding()),
			"exited":         exited,
		})
		return
	}

	vectorsPath := os.Getenv("SIGNAL_CONFORMANCE_VECTORS")
	if vectorsPath == "" {
		vectorsPath = defaultVectorsPath()
	}
	runVectors(vectorsPath)
}

func assertEq[T comparable](got, expected T, msg string) {
	if got != expected {
		panic(fmt.Sprintf("%s: got %v, expected %v", msg, got, expected))
	}
}

func runVectors(vectorsPath string) {
	clean := filepath.Clean(vectorsPath)
	// #nosec G304 G703 -- internal conformance harness reads local corpus fixtures selected by the test runner
	raw, err := os.ReadFile(clean)
	if err != nil {
		panic(fmt.Sprintf("read %s: %v", clean, err))
	}
	var doc map[string]any
	if err := json.Unmarshal(raw, &doc); err != nil {
		panic(fmt.Sprintf("unmarshal %s: %v", clean, err))
	}

	assertEq(doc["schema"].(string), "sysprims-signal-conformance/v1", "schema")

	// 1. Constants
	constants := doc["constants"].(map[string]any)
	assertEq(uint32(constants["max_safe_pid"].(float64)), signal.MaxSafePID, "max_safe_pid")
	common := constants["signals_common"].(map[string]any)
	assertEq(int(common["SIGHUP"].(float64)), signal.SIGHUP, "SIGHUP")
	assertEq(int(common["SIGINT"].(float64)), signal.SIGINT, "SIGINT")
	assertEq(int(common["SIGQUIT"].(float64)), signal.SIGQUIT, "SIGQUIT")
	assertEq(int(common["SIGKILL"].(float64)), signal.SIGKILL, "SIGKILL")
	assertEq(int(common["SIGPIPE"].(float64)), signal.SIGPIPE, "SIGPIPE")
	assertEq(int(common["SIGALRM"].(float64)), signal.SIGALRM, "SIGALRM")
	assertEq(int(common["SIGTERM"].(float64)), signal.SIGTERM, "SIGTERM")

	platKey := "other"
	if runtime.GOOS == "darwin" {
		platKey = "darwin"
	}
	plat := constants["signals_platform"].(map[string]any)[platKey].(map[string]any)
	assertEq(int(plat["SIGUSR1"].(float64)), signal.SIGUSR1, "SIGUSR1")
	assertEq(int(plat["SIGUSR2"].(float64)), signal.SIGUSR2, "SIGUSR2")

	emitLine(map[string]any{
		"suite":        "constants",
		"id":           "signal-constants",
		"max_safe_pid": signal.MaxSafePID,
		"sighup":       signal.SIGHUP,
		"sigint":       signal.SIGINT,
		"sigquit":      signal.SIGQUIT,
		"sigkill":      signal.SIGKILL,
		"sigpipe":      signal.SIGPIPE,
		"sigalrm":      signal.SIGALRM,
		"sigterm":      signal.SIGTERM,
		"sigusr1":      signal.SIGUSR1,
		"sigusr2":      signal.SIGUSR2,
	})

	// 2. PID / PGID validation cases
	for _, item := range doc["pid_validation_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		kind := c["kind"].(string)
		val := uint32(c["value"].(float64))
		expected := c["expected"].(string)

		var err error
		switch kind {
		case "pid":
			err = signal.ValidatePID(val)
		case "pgid":
			err = signal.ValidatePGID(val)
		default:
			panic("unknown pid_validation_cases kind: " + kind)
		}
		got := "ok"
		if err != nil {
			got = errorCodeSlug(err)
		}
		assertEq(got, expected, "pid validation "+id)
		emitLine(map[string]any{
			"suite":   "pid-validation",
			"id":      id,
			"kind":    kind,
			"outcome": got,
		})
	}

	// 3. Batch validation cases
	for _, item := range doc["batch_validation_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		rawPIDs := c["pids"].([]any)
		pids := make([]uint32, len(rawPIDs))
		for i, v := range rawPIDs {
			pids[i] = uint32(v.(float64))
		}
		expected := c["expected"].(string)
		got := "ok"
		if _, err := signal.KillMany(pids, signal.SIGTERM); err != nil {
			got = errorCodeSlug(err)
		}
		assertEq(got, expected, "batch validation "+id)
		emitLine(map[string]any{
			"suite":   "batch-validation",
			"id":      id,
			"outcome": got,
		})
	}

	// 4. Resolve signal cases
	for _, item := range doc["resolve_signal_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		input := c["input"].(string)
		expected := c["expected"].(string)
		signum, err := signal.ResolveSignalName(input)
		if err == nil {
			assertEq("ok", expected, "resolve signal "+id)
			expSigName := c["signal"].(string)
			assertEq(signum, canonicalSignalNumber(expSigName), "resolved signum "+id)
			emitLine(map[string]any{
				"suite":   "resolve-signal",
				"id":      id,
				"outcome": "ok",
				"signum":  signum,
			})
		} else {
			got := errorCodeSlug(err)
			assertEq(got, expected, "resolve signal error "+id)
			emitLine(map[string]any{
				"suite":   "resolve-signal",
				"id":      id,
				"outcome": got,
			})
		}
	}

	// 5. Match signal cases
	for _, item := range doc["match_signal_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		pattern := c["pattern"].(string)
		rawExpected := c["expected"].([]any)
		got := signal.MatchSignalNames(pattern)
		assertEq(len(got), len(rawExpected), "match signal len "+id)
		for i, v := range rawExpected {
			assertEq(got[i], v.(string), "match signal item "+id)
		}
		emitLine(map[string]any{
			"suite":   "match-signal",
			"id":      id,
			"matches": got,
		})
	}

	// 6. ProcessHandle validation cases
	for _, item := range doc["process_handle_validation_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		var pid uint32
		if s, ok := c["pid"].(string); ok && s == "self" {
			pid = uint32(os.Getpid()) // #nosec G115 -- os.Getpid() is always positive and <= MaxInt32
		} else {
			pid = uint32(c["pid"].(float64))
		}
		expected := c["expected"].(string)
		var (
			h   *signal.ProcessHandle
			err error
		)
		if expID, ok := c["expected_start_identity"].(string); ok {
			h, err = signal.OpenProcessHandleVerified(pid, expID)
		} else {
			h, err = signal.OpenProcessHandle(pid)
		}
		got := "ok"
		if err != nil {
			got = errorCodeSlug(err)
		} else if h != nil {
			_ = h.Close()
		}
		assertEq(got, expected, "process handle validation "+id)
		emitLine(map[string]any{
			"suite":   "process-handle-validation",
			"id":      id,
			"outcome": got,
		})
	}
}
