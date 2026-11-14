// Package proc provides cross-platform process table inspection, lineage
// traversal (ancestors and descendants), batch signal dispatch, resource
// guard evaluation and runner loops, file descriptor enumeration, and
// listening port discovery.
//
// All functionality is implemented in pure Go (CGO_ENABLED=0) with identical
// safety invariants (ADR-0011 PID validation, self/PID-1/kernel-task protection)
// and canonical JSON schemas across Rust, Go, Python, and TypeScript.
package proc

import (
	"fmt"
	"math"
	"os"
	"os/user"
	"runtime"
	"slices"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/3leaps/sysprims/signal"
	"github.com/3leaps/sysprims/syserror"
)

// Schema URL constants for process snapshots, lineage, port bindings, FD snapshots, and guard events.
const (
	ProcessInfoSchemaID              = "https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info.schema.json"
	ProcessInfoSampledSchemaID       = "https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info-sampled.schema.json"
	DescendantsResultSchemaID        = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/descendants-result.schema.json"
	DescendantsResultSampledSchemaID = "https://schemas.3leaps.dev/sysprims/process/v1.1.0/descendants-result-sampled.schema.json"
	AncestorsResultSchemaID          = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/ancestors-result.schema.json"
	WaitPidResultSchemaID            = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/wait-pid-result.schema.json"
	FdSnapshotSchemaID               = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-snapshot.schema.json"
	PortBindingsSchemaID             = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-bindings.schema.json"
	PortFilterSchemaID               = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-filter.schema.json"
	GuardEventSchemaID               = "https://schemas.3leaps.dev/sysprims/process/v1.0.0/guard-event.schema.json"
	BatchKillResultSchemaID          = "https://schemas.3leaps.dev/sysprims/signal/v1.0.0/batch-kill-result.schema.json"
)

// MaxSafePID is the maximum PID accepted across sysprims (ADR-0011: MaxSafeInt32).
const MaxSafePID = signal.MaxSafePID

// Re-export PID-reuse-safe process handles from package signal for convenience.
type (
	ProcessHandle = signal.ProcessHandle
	SignalBinding = signal.SignalBinding
)

var (
	// OpenProcessHandle acquires a PID-reuse-safe handle to a target process.
	OpenProcessHandle = signal.OpenProcessHandle
	// OpenProcessHandleVerified acquires a PID-reuse-safe handle and verifies its start time.
	OpenProcessHandleVerified = signal.OpenProcessHandleVerified
)

// ValidatePID validates a PID against ADR-0011 rules (0 < pid <= math.MaxInt32).
func ValidatePID(pid uint32) error {
	if pid == 0 {
		return syserror.InvalidArgument("PID 0 is not valid for process inspection (represents process group)")
	}
	if pid > MaxSafePID {
		return syserror.InvalidArgument(fmt.Sprintf("PID %d exceeds maximum safe value %d (would overflow signed int32)", pid, MaxSafePID))
	}
	return nil
}

// CurrentPlatform returns the canonical sysprims platform identifier ("macos", "linux", "windows", etc.).
func CurrentPlatform() string {
	if runtime.GOOS == "darwin" {
		return "macos"
	}
	return runtime.GOOS
}

func nowRFC3339() string {
	return time.Now().UTC().Format("2006-01-02T15:04:05.999999999Z07:00")
}

// lookupUsername resolves a numeric UID to a username string in pure Go (CGO_ENABLED=0).
func lookupUsername(uid uint32) *string {
	if int(uid) == os.Getuid() || int(uid) == os.Geteuid() {
		if u, err := user.Current(); err == nil && u.Username != "" {
			name := u.Username
			return &name
		}
		if envUser := os.Getenv("USER"); envUser != "" {
			return &envUser
		}
	}
	if u, err := user.LookupId(strconv.FormatUint(uint64(uid), 10)); err == nil && u.Username != "" {
		name := u.Username
		return &name
	}
	return nil
}

// ProcessState represents normalized process execution state across platforms.
type ProcessState string

const (
	StateRunning  ProcessState = "running"
	StateSleeping ProcessState = "sleeping"
	StateStopped  ProcessState = "stopped"
	StateZombie   ProcessState = "zombie"
	StateUnknown  ProcessState = "unknown"
)

func isValidProcessState(s ProcessState) bool {
	switch s {
	case StateRunning, StateSleeping, StateStopped, StateZombie, StateUnknown:
		return true
	default:
		return false
	}
}

// ProcessOptions controls optional enrichment fields during process enumeration.
type ProcessOptions struct {
	IncludeEnv     bool `json:"include_env,omitempty"`
	IncludeThreads bool `json:"include_threads,omitempty"`
}

// ProcessInfo holds normalized information about a single process.
type ProcessInfo struct {
	PID             uint32            `json:"pid"`
	PPID            uint32            `json:"ppid"`
	Name            string            `json:"name"`
	User            *string           `json:"user,omitempty"`
	CPUPercent      float64           `json:"cpu_percent"`
	MemoryKB        uint64            `json:"memory_kb"`
	ElapsedSeconds  uint64            `json:"elapsed_seconds"`
	StartTimeUnixMS *uint64           `json:"start_time_unix_ms,omitempty"`
	ExePath         *string           `json:"exe_path,omitempty"`
	State           ProcessState      `json:"state"`
	Cmdline         []string          `json:"cmdline"`
	Env             map[string]string `json:"env,omitempty"`
	ThreadCount     *uint32           `json:"thread_count,omitempty"`
}

func normalizeProcessInfo(p *ProcessInfo) {
	if p.Cmdline == nil {
		p.Cmdline = []string{}
	}
}

// ProcessSnapshot represents a point-in-time snapshot of processes.
type ProcessSnapshot struct {
	SchemaID  string        `json:"schema_id"`
	Timestamp string        `json:"timestamp"`
	Processes []ProcessInfo `json:"processes"`
}

// ProcessFilter specifies filter criteria for process queries.
// All non-nil / non-empty criteria are combined with logical AND.
type ProcessFilter struct {
	NameContains          *string        `json:"name_contains,omitempty"`
	NameEquals            *string        `json:"name_equals,omitempty"`
	UserEquals            *string        `json:"user_equals,omitempty"`
	PIDIn                 []uint32       `json:"pid_in,omitempty"`
	PPID                  *uint32        `json:"ppid,omitempty"`
	StateIn               []ProcessState `json:"state_in,omitempty"`
	CPUAbove              *float64       `json:"cpu_above,omitempty"`
	MemoryAboveKB         *uint64        `json:"memory_above_kb,omitempty"`
	RunningForAtLeastSecs *uint64        `json:"running_for_at_least_secs,omitempty"`
}

// Validate checks filter parameter bounds (e.g. cpu_above in [0.0, 100.0] and valid states).
func (f *ProcessFilter) Validate() error {
	if f == nil {
		return nil
	}
	if f.CPUAbove != nil {
		v := *f.CPUAbove
		if math.IsNaN(v) || math.IsInf(v, 0) || v < 0.0 || v > 100.0 {
			return syserror.InvalidArgument(fmt.Sprintf("cpu_above must be between 0.0 and 100.0, got %v", v))
		}
	}
	for _, st := range f.StateIn {
		if !isValidProcessState(st) {
			return syserror.InvalidArgument(fmt.Sprintf("invalid state in state_in: %q", st))
		}
	}
	return nil
}

// Matches returns true if the given ProcessInfo satisfies all criteria in the filter.
func (f *ProcessFilter) Matches(proc *ProcessInfo) bool {
	if f == nil {
		return true
	}
	if f.NameContains != nil {
		needle := strings.ToLower(*f.NameContains)
		if !strings.Contains(strings.ToLower(proc.Name), needle) {
			return false
		}
	}
	if f.NameEquals != nil {
		if proc.Name != *f.NameEquals {
			return false
		}
	}
	if f.UserEquals != nil {
		if proc.User == nil || *proc.User != *f.UserEquals {
			return false
		}
	}
	if len(f.PIDIn) > 0 {
		if !slices.Contains(f.PIDIn, proc.PID) {
			return false
		}
	}
	if f.PPID != nil {
		if proc.PPID != *f.PPID {
			return false
		}
	}
	if len(f.StateIn) > 0 {
		if !slices.Contains(f.StateIn, proc.State) {
			return false
		}
	}
	if f.CPUAbove != nil {
		if proc.CPUPercent <= *f.CPUAbove {
			return false
		}
	}
	if f.MemoryAboveKB != nil {
		if proc.MemoryKB <= *f.MemoryAboveKB {
			return false
		}
	}
	if f.RunningForAtLeastSecs != nil {
		if proc.ElapsedSeconds < *f.RunningForAtLeastSecs {
			return false
		}
	}
	return true
}

// GetProcess retrieves information for a single process by PID.
func GetProcess(pid uint32) (*ProcessInfo, error) {
	return GetProcessWithOptions(pid, nil)
}

// GetProcessWithOptions retrieves information for a single process by PID with optional enrichment.
func GetProcessWithOptions(pid uint32, options *ProcessOptions) (*ProcessInfo, error) {
	if err := ValidatePID(pid); err != nil {
		return nil, err
	}
	var opts ProcessOptions
	if options != nil {
		opts = *options
	}
	p, err := getProcessImpl(pid, opts)
	if err != nil {
		return nil, err
	}
	normalizeProcessInfo(p)
	return p, nil
}

// ProcessGet is an alias for GetProcess matching the legacy Go bindings API.
func ProcessGet(pid uint32) (*ProcessInfo, error) {
	return GetProcess(pid)
}

// ProcessGetWithOptions is an alias for GetProcessWithOptions matching the legacy Go bindings API.
func ProcessGetWithOptions(pid uint32, options *ProcessOptions) (*ProcessInfo, error) {
	return GetProcessWithOptions(pid, options)
}

// Snapshot returns a point-in-time snapshot of all accessible processes.
func Snapshot() (*ProcessSnapshot, error) {
	return SnapshotFilteredWithOptions(nil, nil)
}

// SnapshotWithOptions returns a point-in-time snapshot of all accessible processes with optional enrichment.
func SnapshotWithOptions(options *ProcessOptions) (*ProcessSnapshot, error) {
	return SnapshotFilteredWithOptions(nil, options)
}

// SnapshotFiltered returns a point-in-time snapshot of processes matching filter.
func SnapshotFiltered(filter *ProcessFilter) (*ProcessSnapshot, error) {
	return SnapshotFilteredWithOptions(filter, nil)
}

// SnapshotFilteredWithOptions returns a point-in-time snapshot of processes matching filter with optional enrichment.
func SnapshotFilteredWithOptions(filter *ProcessFilter, options *ProcessOptions) (*ProcessSnapshot, error) {
	if filter != nil {
		if err := filter.Validate(); err != nil {
			return nil, err
		}
	}
	var opts ProcessOptions
	if options != nil {
		opts = *options
	}
	all, err := snapshotAllImpl(opts)
	if err != nil {
		return nil, err
	}
	processes := make([]ProcessInfo, 0, len(all))
	for i := range all {
		normalizeProcessInfo(&all[i])
		if filter == nil || filter.Matches(&all[i]) {
			processes = append(processes, all[i])
		}
	}
	return &ProcessSnapshot{
		SchemaID:  ProcessInfoSchemaID,
		Timestamp: nowRFC3339(),
		Processes: processes,
	}, nil
}

// ProcessList is an alias for SnapshotFiltered matching the legacy Go bindings API.
func ProcessList(filter *ProcessFilter) (*ProcessSnapshot, error) {
	return SnapshotFiltered(filter)
}

// ProcessListWithOptions is an alias for SnapshotFilteredWithOptions matching the legacy Go bindings API.
func ProcessListWithOptions(filter *ProcessFilter, options *ProcessOptions) (*ProcessSnapshot, error) {
	return SnapshotFilteredWithOptions(filter, options)
}

// WaitPidResult holds the result of waiting for a PID to exit.
type WaitPidResult struct {
	SchemaID  string   `json:"schema_id"`
	Timestamp string   `json:"timestamp"`
	Platform  string   `json:"platform"`
	PID       uint32   `json:"pid"`
	Exited    bool     `json:"exited"`
	TimedOut  bool     `json:"timed_out"`
	ExitCode  *int32   `json:"exit_code,omitempty"`
	Warnings  []string `json:"warnings"`
}

// WaitPID waits for a process to exit up to timeout.
func WaitPID(pid uint32, timeout time.Duration) (*WaitPidResult, error) {
	if err := ValidatePID(pid); err != nil {
		return nil, err
	}
	if timeout < 0 {
		timeout = 0
	}
	timeoutMS := uint64(timeout / time.Millisecond)
	res, err := waitPIDImpl(pid, timeoutMS)
	if err != nil {
		return nil, err
	}
	if res.Warnings == nil {
		res.Warnings = []string{}
	}
	return res, nil
}

const maxTimeoutMS = uint64(math.MaxInt64 / int64(time.Millisecond))

func durationFromTimeoutMS(timeoutMS uint64) time.Duration {
	if timeoutMS > maxTimeoutMS {
		timeoutMS = maxTimeoutMS
	}
	return time.Duration(timeoutMS) * time.Millisecond // #nosec G115 -- clamped to maxTimeoutMS above
}

func safeUint32Len(n int) uint32 {
	if n <= 0 {
		return 0
	}
	if uint64(n) > math.MaxUint32 {
		return math.MaxUint32
	}
	return uint32(n) // #nosec G115 -- bounded to [0, math.MaxUint32]
}

func safeSelfAndParentPID() (uint32, uint32) {
	var selfPID, parentPID uint32
	if p := os.Getpid(); p > 0 && uint64(p) <= uint64(MaxSafePID) {
		selfPID = uint32(p) // #nosec G115 -- bounded to [1, MaxSafePID]
	}
	if pp := os.Getppid(); pp > 0 && uint64(pp) <= uint64(MaxSafePID) {
		parentPID = uint32(pp) // #nosec G115 -- bounded to [1, MaxSafePID]
	}
	return selfPID, parentPID
}

// ProcessWait is an alias for WaitPID taking timeout in milliseconds, matching the legacy Go bindings API.
func ProcessWait(pid uint32, timeoutMS uint64) (*WaitPidResult, error) {
	return WaitPID(pid, durationFromTimeoutMS(timeoutMS))
}

// Liveness captures whether a process is running (live) or still present as a zombie in the process table.
type Liveness struct {
	Live         bool
	TablePresent bool
}

// IsLive returns true if the process exists and is not a zombie.
func IsLive(pid uint32) bool {
	if err := ValidatePID(pid); err != nil {
		return false
	}
	return livenessImpl(pid).Live
}

// IsFullyGone returns true if the process has been completely reaped from the OS process table.
func IsFullyGone(pid uint32) bool {
	if err := ValidatePID(pid); err != nil {
		return true
	}
	return !livenessImpl(pid).TablePresent
}

// CPUTotalTimeNs returns cumulative user+system CPU time in nanoseconds for pid.
func CPUTotalTimeNs(pid uint32) (uint64, error) {
	if err := ValidatePID(pid); err != nil {
		return 0, err
	}
	return cpuTotalTimeNsImpl(pid)
}

// AncestorsResult holds the ancestor chain starting at PID and walking up via PPID.
type AncestorsResult struct {
	SchemaID  string        `json:"schema_id"`
	Timestamp string        `json:"timestamp"`
	Platform  string        `json:"platform"`
	PID       uint32        `json:"pid"`
	Chain     []ProcessInfo `json:"chain"`
	Warnings  []string      `json:"warnings"`
}

// Ancestors walks up the process tree starting at pid up to maxDepth parents.
// Chain[0] is the target process itself, Chain[1] is its parent, etc.
func Ancestors(pid uint32, maxDepth uint32, options *ProcessOptions) (*AncestorsResult, error) {
	if err := ValidatePID(pid); err != nil {
		return nil, err
	}
	snap, err := SnapshotWithOptions(options)
	if err != nil {
		return nil, err
	}
	byPID := make(map[uint32]ProcessInfo, len(snap.Processes))
	for _, p := range snap.Processes {
		byPID[p.PID] = p
	}
	start, ok := byPID[pid]
	if !ok {
		p, err := GetProcessWithOptions(pid, options)
		if err != nil {
			return nil, err
		}
		start = *p
		byPID[pid] = start
	}

	warnings := []string{}
	visited := make(map[uint32]bool)
	chain := []ProcessInfo{start}
	visited[pid] = true

	cur := start
	for depth := uint32(0); depth < maxDepth; depth++ {
		ppid := cur.PPID
		if ppid == 0 || ppid == cur.PID {
			break
		}
		if ppid > MaxSafePID {
			warnings = append(warnings, fmt.Sprintf("stopping at invalid parent pid %d", ppid))
			break
		}
		if visited[ppid] {
			warnings = append(warnings, fmt.Sprintf("cycle detected at pid %d", ppid))
			break
		}
		visited[ppid] = true

		next, found := byPID[ppid]
		if !found {
			p, err := GetProcessWithOptions(ppid, options)
			if err != nil {
				if syserror.IsCode(err, syserror.ErrNotFound) {
					warnings = append(warnings, fmt.Sprintf("parent pid %d not found (may have exited)", ppid))
					break
				}
				if syserror.IsCode(err, syserror.ErrPermissionDenied) {
					warnings = append(warnings, fmt.Sprintf("permission denied reading parent pid %d", ppid))
					break
				}
				return nil, err
			}
			next = *p
		}
		chain = append(chain, next)
		cur = next
	}

	return &AncestorsResult{
		SchemaID:  AncestorsResultSchemaID,
		Timestamp: nowRFC3339(),
		Platform:  CurrentPlatform(),
		PID:       pid,
		Chain:     chain,
		Warnings:  warnings,
	}, nil
}

// ProcessAncestors is an alias for Ancestors matching the legacy Go bindings API.
func ProcessAncestors(pid uint32, maxDepth uint32, options *ProcessOptions) (*AncestorsResult, error) {
	return Ancestors(pid, maxDepth, options)
}

// CpuMode selects how CPU percentage is computed during descendant traversal and guard evaluation.
type CpuMode string

const (
	// CpuModeLifetime computes average CPU usage over the process lifetime: (total_cpu_time / elapsed_time) * 100.
	CpuModeLifetime CpuMode = "lifetime"
	// CpuModeMonitor samples CPU time over a short interval (Activity Monitor / top style), where 100% = 1 full core.
	CpuModeMonitor CpuMode = "monitor"
)

func parseCpuMode(m CpuMode) (CpuMode, error) {
	switch m {
	case "", CpuModeLifetime:
		return CpuModeLifetime, nil
	case CpuModeMonitor:
		return CpuModeMonitor, nil
	default:
		return "", syserror.InvalidArgument(fmt.Sprintf("invalid cpu_mode: %q (expected 'lifetime' or 'monitor')", m))
	}
}

// DescendantsLevel groups processes at a specific depth below RootPID (Level 1 = direct children).
type DescendantsLevel struct {
	Level     uint32        `json:"level"`
	Processes []ProcessInfo `json:"processes"`
}

// DescendantsResult holds the hierarchical result of a descendant traversal.
type DescendantsResult struct {
	SchemaID        string             `json:"schema_id"`
	RootPID         uint32             `json:"root_pid"`
	MaxLevels       uint32             `json:"max_levels"`
	Levels          []DescendantsLevel `json:"levels"`
	TotalFound      int                `json:"total_found"`
	MatchedByFilter int                `json:"matched_by_filter"`
	Timestamp       string             `json:"timestamp"`
	Platform        string             `json:"platform"`
}

// DescendantsOptions configures DescendantsWithOptions.
type DescendantsOptions struct {
	MaxLevels      *uint32
	Filter         *ProcessFilter
	CpuMode        CpuMode
	SampleDuration time.Duration
	Options        *ProcessOptions
}

// DescendantsConfig is an alias for DescendantsOptions.
type DescendantsConfig = DescendantsOptions

// Descendants traverses the process tree downward from rootPID up to maxLevels.
func Descendants(rootPID uint32, maxLevels uint32, filter *ProcessFilter) (*DescendantsResult, error) {
	return DescendantsWithOptions(rootPID, &DescendantsOptions{
		MaxLevels: &maxLevels,
		Filter:    filter,
	})
}

// DescendantsWithOptions traverses the process tree downward from rootPID with CPU mode, sampling, and enrichment options.
func DescendantsWithOptions(rootPID uint32, opts *DescendantsOptions) (*DescendantsResult, error) {
	if err := ValidatePID(rootPID); err != nil {
		return nil, err
	}
	var cfg DescendantsOptions
	if opts != nil {
		cfg = *opts
	}
	if cfg.Filter != nil {
		if err := cfg.Filter.Validate(); err != nil {
			return nil, err
		}
	}
	if cfg.SampleDuration < 0 {
		return nil, syserror.InvalidArgument("sample duration must be >= 0")
	}
	mode, err := parseCpuMode(cfg.CpuMode)
	if err != nil {
		return nil, err
	}

	snap, err := SnapshotWithOptions(cfg.Options)
	if err != nil {
		return nil, err
	}

	rootExists := false
	for i := range snap.Processes {
		if snap.Processes[i].PID == rootPID {
			rootExists = true
			break
		}
	}
	if !rootExists {
		if _, err := GetProcessWithOptions(rootPID, cfg.Options); err != nil {
			return nil, err
		}
	}

	limit := uint32(math.MaxUint32)
	if cfg.MaxLevels != nil {
		limit = *cfg.MaxLevels
	}

	res := buildDescendantsFromSnapshot(rootPID, limit, nil, snap.Processes)
	schemaID := DescendantsResultSchemaID

	if mode == CpuModeMonitor {
		schemaID = DescendantsResultSampledSchemaID
		dur := cfg.SampleDuration
		if dur <= 0 {
			dur = 1000 * time.Millisecond
		}
		var descPIDs []uint32
		for _, lvl := range res.Levels {
			for _, p := range lvl.Processes {
				descPIDs = append(descPIDs, p.PID)
			}
		}
		if len(descPIDs) > 0 {
			t0 := make(map[uint32]uint64, len(descPIDs))
			for _, pid := range descPIDs {
				if ns, err := CPUTotalTimeNs(pid); err == nil {
					t0[pid] = ns
				}
			}
			time.Sleep(dur)
			t1 := make(map[uint32]uint64, len(descPIDs))
			for _, pid := range descPIDs {
				if ns, err := CPUTotalTimeNs(pid); err == nil {
					t1[pid] = ns
				}
			}
			sampleNs := float64(dur.Nanoseconds())
			if sampleNs > 0 {
				for li := range res.Levels {
					for pi := range res.Levels[li].Processes {
						p := &res.Levels[li].Processes[pi]
						c0, ok0 := t0[p.PID]
						c1, ok1 := t1[p.PID]
						if ok0 && ok1 {
							var delta uint64
							if c1 > c0 {
								delta = c1 - c0
							}
							pct := (float64(delta) / sampleNs) * 100.0
							p.CPUPercent = math.Round(pct*100.0) / 100.0
						} else {
							p.CPUPercent = 0.0
						}
					}
				}
			}
		}
	}

	if cfg.Filter != nil {
		res = filterDescendantsLevels(res, cfg.Filter)
	}
	res.SchemaID = schemaID
	return res, nil
}

// DescendantsWithConfig is an alias for DescendantsWithOptions taking a value struct.
func DescendantsWithConfig(rootPID uint32, cfg DescendantsConfig) (*DescendantsResult, error) {
	return DescendantsWithOptions(rootPID, &cfg)
}

// ProcessDescendants is an alias for Descendants matching the legacy Go bindings API.
func ProcessDescendants(rootPID uint32, maxLevels uint32, filter *ProcessFilter) (*DescendantsResult, error) {
	return Descendants(rootPID, maxLevels, filter)
}

func buildDescendantsFromSnapshot(rootPID uint32, maxLevels uint32, filter *ProcessFilter, processes []ProcessInfo) *DescendantsResult {
	childrenMap := make(map[uint32][]ProcessInfo)
	for _, p := range processes {
		if p.PID == p.PPID {
			continue
		}
		childrenMap[p.PPID] = append(childrenMap[p.PPID], p)
	}

	visited := make(map[uint32]bool)
	visited[rootPID] = true
	currentParentPIDs := []uint32{rootPID}

	levels := []DescendantsLevel{}
	totalFound := 0
	matchedByFilter := 0

	for level := uint32(1); level <= maxLevels; level++ {
		if len(currentParentPIDs) == 0 {
			break
		}
		var nextParentPIDs []uint32
		var levelProcesses []ProcessInfo

		for _, parentPID := range currentParentPIDs {
			children := childrenMap[parentPID]
			for _, child := range children {
				if visited[child.PID] {
					continue
				}
				visited[child.PID] = true
				totalFound++
				nextParentPIDs = append(nextParentPIDs, child.PID)

				if filter == nil || filter.Matches(&child) {
					matchedByFilter++
					cp := child
					normalizeProcessInfo(&cp)
					levelProcesses = append(levelProcesses, cp)
				}
			}
		}

		if len(levelProcesses) > 0 {
			levels = append(levels, DescendantsLevel{
				Level:     level,
				Processes: levelProcesses,
			})
		}
		currentParentPIDs = nextParentPIDs
		if level == math.MaxUint32 {
			break
		}
	}

	return &DescendantsResult{
		SchemaID:        DescendantsResultSchemaID,
		RootPID:         rootPID,
		MaxLevels:       maxLevels,
		Levels:          levels,
		TotalFound:      totalFound,
		MatchedByFilter: matchedByFilter,
		Timestamp:       nowRFC3339(),
		Platform:        CurrentPlatform(),
	}
}

func filterDescendantsLevels(result *DescendantsResult, filter *ProcessFilter) *DescendantsResult {
	matched := 0
	filteredLevels := []DescendantsLevel{}
	for _, lvl := range result.Levels {
		var procs []ProcessInfo
		for _, p := range lvl.Processes {
			if filter.Matches(&p) {
				matched++
				procs = append(procs, p)
			}
		}
		if len(procs) > 0 {
			filteredLevels = append(filteredLevels, DescendantsLevel{
				Level:     lvl.Level,
				Processes: procs,
			})
		}
	}
	out := *result
	out.Levels = filteredLevels
	out.MatchedByFilter = matched
	return &out
}

// SelectDescendantTargets selects target PIDs from a DescendantsResult for killing or guard remediation.
// When cascade is false, returns matched descendant PIDs.
// When cascade is true, expands each matched PID to include its descendant subtree within traversal.
// Returned PIDs are sorted ascending, deduplicated, and never include traversal.RootPID.
func SelectDescendantTargets(traversal *DescendantsResult, filter *ProcessFilter, cascade bool) ([]uint32, error) {
	if filter != nil {
		if err := filter.Validate(); err != nil {
			return nil, err
		}
	}
	if traversal == nil || len(traversal.Levels) == 0 {
		return []uint32{}, nil
	}

	var matchedRoots []uint32
	childrenMap := make(map[uint32][]uint32)

	for _, lvl := range traversal.Levels {
		for _, p := range lvl.Processes {
			childrenMap[p.PPID] = append(childrenMap[p.PPID], p.PID)
			if filter == nil || filter.Matches(&p) {
				matchedRoots = append(matchedRoots, p.PID)
			}
		}
	}

	selected := make(map[uint32]bool, len(matchedRoots))
	for _, pid := range matchedRoots {
		if pid != traversal.RootPID {
			selected[pid] = true
		}
	}

	if cascade {
		stack := make([]uint32, 0, len(matchedRoots))
		stack = append(stack, matchedRoots...)
		visited := make(map[uint32]bool, len(matchedRoots))
		for len(stack) > 0 {
			cur := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			if visited[cur] {
				continue
			}
			visited[cur] = true
			if cur != traversal.RootPID {
				selected[cur] = true
			}
			for _, child := range childrenMap[cur] {
				if !visited[child] {
					stack = append(stack, child)
				}
			}
		}
	}

	out := make([]uint32, 0, len(selected))
	for pid := range selected {
		out = append(out, pid)
	}
	sort.Slice(out, func(i, j int) bool { return out[i] < out[j] })
	return out, nil
}

// KillDescendantsFail records a single PID failure during KillDescendants.
type KillDescendantsFail struct {
	PID   uint32 `json:"pid"`
	Error string `json:"error"`
}

// KillDescendantsResult holds the result of killing descendants of a root process.
type KillDescendantsResult struct {
	SchemaID      string                `json:"schema_id"`
	SignalSent    int                   `json:"signal_sent"`
	RootPID       uint32                `json:"root_pid"`
	Succeeded     []uint32              `json:"succeeded"`
	Failed        []KillDescendantsFail `json:"failed"`
	SkippedSafety int                   `json:"skipped_safety"`
}

// KillDescendantsOptions configures KillDescendantsWithOptions.
type KillDescendantsOptions struct {
	Signal         int
	MaxLevels      *uint32
	Filter         *ProcessFilter
	CpuMode        CpuMode
	SampleDuration time.Duration
	Cascade        bool
}

// KillDescendantsConfig is an alias for KillDescendantsOptions.
type KillDescendantsConfig = KillDescendantsOptions

// KillDescendants traverses descendants of rootPID and sends sig to matching targets.
func KillDescendants(rootPID uint32, sig int, maxLevels uint32, filter *ProcessFilter) (*KillDescendantsResult, error) {
	return KillDescendantsWithOptions(rootPID, &KillDescendantsOptions{
		Signal:    sig,
		MaxLevels: &maxLevels,
		Filter:    filter,
	})
}

// KillDescendantsWithOptions traverses descendants of rootPID and sends opts.Signal to selected targets.
// Safety rules: never signals RootPID, current process (os.Getpid()), parent of current process, or PID 1.
func KillDescendantsWithOptions(rootPID uint32, opts *KillDescendantsOptions) (*KillDescendantsResult, error) {
	var cfg KillDescendantsOptions
	if opts != nil {
		cfg = *opts
	}
	sig := cfg.Signal
	if sig == 0 {
		sig = signal.SIGTERM
	}
	traversal, err := DescendantsWithOptions(rootPID, &DescendantsOptions{
		MaxLevels:      cfg.MaxLevels,
		Filter:         nil, // Keep full tree so Cascade can walk children of matched nodes
		CpuMode:        cfg.CpuMode,
		SampleDuration: cfg.SampleDuration,
	})
	if err != nil {
		return nil, err
	}
	targets, err := SelectDescendantTargets(traversal, cfg.Filter, cfg.Cascade)
	if err != nil {
		return nil, err
	}
	return executeKillDescendants(rootPID, sig, targets)
}

func executeKillDescendants(rootPID uint32, sig int, targets []uint32) (*KillDescendantsResult, error) {
	selfPID, parentPID := safeSelfAndParentPID()
	var toKill []uint32
	skippedSafety := 0

	for _, pid := range targets {
		if pid == rootPID || pid == selfPID || pid == 1 || (parentPID > 0 && pid == parentPID) {
			skippedSafety++
			continue
		}
		toKill = append(toKill, pid)
	}

	if len(toKill) == 0 {
		return &KillDescendantsResult{
			SchemaID:      BatchKillResultSchemaID,
			SignalSent:    sig,
			RootPID:       rootPID,
			Succeeded:     []uint32{},
			Failed:        []KillDescendantsFail{},
			SkippedSafety: skippedSafety,
		}, nil
	}

	batchRes, err := signal.KillMany(toKill, sig)
	if err != nil {
		return nil, err
	}

	succeeded := batchRes.Succeeded
	if succeeded == nil {
		succeeded = []uint32{}
	}
	failed := make([]KillDescendantsFail, len(batchRes.Failed))
	for i, f := range batchRes.Failed {
		errMsg := ""
		if f.Err != nil {
			errMsg = f.Err.Error()
		}
		failed[i] = KillDescendantsFail{
			PID:   f.PID,
			Error: errMsg,
		}
	}

	return &KillDescendantsResult{
		SchemaID:      BatchKillResultSchemaID,
		SignalSent:    sig,
		RootPID:       rootPID,
		Succeeded:     succeeded,
		Failed:        failed,
		SkippedSafety: skippedSafety,
	}, nil
}

// Protocol represents transport layer protocol for port bindings.
type Protocol string

const (
	ProtocolTCP Protocol = "tcp"
	ProtocolUDP Protocol = "udp"
)

// PortFilter filters listening port queries.
type PortFilter struct {
	Protocol  *Protocol `json:"protocol,omitempty"`
	LocalPort *uint16   `json:"local_port,omitempty"`
}

// Validate checks port filter invariants (LocalPort != 0, valid Protocol).
func (f *PortFilter) Validate() error {
	if f == nil {
		return nil
	}
	if f.LocalPort != nil && *f.LocalPort == 0 {
		return syserror.InvalidArgument("local_port must be in 1..=65535")
	}
	if f.Protocol != nil && *f.Protocol != ProtocolTCP && *f.Protocol != ProtocolUDP {
		return syserror.InvalidArgument(fmt.Sprintf("invalid protocol: %q (expected 'tcp' or 'udp')", *f.Protocol))
	}
	return nil
}

// Matches returns true if the binding matches the filter.
func (f *PortFilter) Matches(b *PortBinding) bool {
	if f == nil {
		return true
	}
	if f.Protocol != nil && b.Protocol != *f.Protocol {
		return false
	}
	if f.LocalPort != nil && b.LocalPort != *f.LocalPort {
		return false
	}
	return true
}

// PortBinding represents a single listening socket binding attributed to a process where permitted.
type PortBinding struct {
	Protocol  Protocol     `json:"protocol"`
	LocalAddr *string      `json:"local_addr,omitempty"`
	LocalPort uint16       `json:"local_port"`
	State     *string      `json:"state,omitempty"`
	PID       *uint32      `json:"pid,omitempty"`
	Process   *ProcessInfo `json:"process,omitempty"`
}

// PortBindingsSnapshot holds a snapshot of listening ports.
type PortBindingsSnapshot struct {
	SchemaID  string        `json:"schema_id"`
	Timestamp string        `json:"timestamp"`
	Platform  string        `json:"platform"`
	Bindings  []PortBinding `json:"bindings"`
	Warnings  []string      `json:"warnings"`
}

// ListeningPorts enumerates listening TCP (and bound UDP) ports matching filter.
func ListeningPorts(filter *PortFilter) (*PortBindingsSnapshot, error) {
	if filter != nil {
		if err := filter.Validate(); err != nil {
			return nil, err
		}
	}
	snap, err := listeningPortsImpl(filter)
	if err != nil {
		return nil, err
	}
	if snap.Bindings == nil {
		snap.Bindings = []PortBinding{}
	}
	for i := range snap.Bindings {
		if snap.Bindings[i].Process != nil {
			normalizeProcessInfo(snap.Bindings[i].Process)
		}
	}
	if snap.Warnings == nil {
		snap.Warnings = []string{}
	}
	return snap, nil
}

// ProcessByPort finds the process listening on the specified port and protocol.
func ProcessByPort(port uint16, protocol *Protocol) (*ProcessInfo, error) {
	if port == 0 {
		return nil, syserror.InvalidArgument("port must be in 1..=65535")
	}
	f := &PortFilter{
		Protocol:  protocol,
		LocalPort: &port,
	}
	snap, err := ListeningPorts(f)
	if err != nil {
		return nil, err
	}
	for _, b := range snap.Bindings {
		if b.LocalPort == port && b.PID != nil {
			if b.Process != nil {
				cp := *b.Process
				normalizeProcessInfo(&cp)
				return &cp, nil
			}
			return GetProcess(*b.PID)
		}
	}
	return nil, syserror.NotFound(uint32(port))
}

// FdKind classifies an open file descriptor.
type FdKind string

const (
	FdKindFile    FdKind = "file"
	FdKindSocket  FdKind = "socket"
	FdKindPipe    FdKind = "pipe"
	FdKindUnknown FdKind = "unknown"
)

func isValidFdKind(k FdKind) bool {
	switch k {
	case FdKindFile, FdKindSocket, FdKindPipe, FdKindUnknown:
		return true
	default:
		return false
	}
}

// FdFilter filters file descriptor snapshots.
type FdFilter struct {
	Kind *FdKind `json:"kind,omitempty"`
}

// Validate checks FdFilter fields.
func (f *FdFilter) Validate() error {
	if f == nil || f.Kind == nil {
		return nil
	}
	if !isValidFdKind(*f.Kind) {
		return syserror.InvalidArgument(fmt.Sprintf("invalid fd kind: %q", *f.Kind))
	}
	return nil
}

// Matches returns true if the FdInfo matches the filter.
func (f *FdFilter) Matches(info *FdInfo) bool {
	if f == nil || f.Kind == nil {
		return true
	}
	return info.Kind == *f.Kind
}

// FdInfo holds metadata about a single open file descriptor.
type FdInfo struct {
	Fd   uint32  `json:"fd"`
	Kind FdKind  `json:"kind"`
	Path *string `json:"path,omitempty"`
}

// IsDeleted returns true if the file descriptor points to an unlinked file on disk
// (conventionally suffixed with " (deleted)" on Linux procfs and macOS vnode check).
func (f FdInfo) IsDeleted() bool {
	return f.Path != nil && strings.HasSuffix(*f.Path, " (deleted)")
}

// FdSnapshot holds a point-in-time snapshot of open file descriptors for a process.
type FdSnapshot struct {
	SchemaID  string   `json:"schema_id"`
	Timestamp string   `json:"timestamp"`
	Platform  string   `json:"platform"`
	PID       uint32   `json:"pid"`
	Fds       []FdInfo `json:"fds"`
	Warnings  []string `json:"warnings"`
}

// ListFds enumerates open file descriptors for pid matching filter.
func ListFds(pid uint32, filter *FdFilter) (*FdSnapshot, error) {
	if err := ValidatePID(pid); err != nil {
		return nil, err
	}
	if filter != nil {
		if err := filter.Validate(); err != nil {
			return nil, err
		}
	}
	snap, err := listFdsImpl(pid, filter)
	if err != nil {
		return nil, err
	}
	if snap.Fds == nil {
		snap.Fds = []FdInfo{}
	}
	if snap.Warnings == nil {
		snap.Warnings = []string{}
	}
	return snap, nil
}

// ProcessListFds is an alias for ListFds matching the legacy Go bindings API.
func ProcessListFds(pid uint32, filter *FdFilter) (*FdSnapshot, error) {
	return ListFds(pid, filter)
}

// GuardRule specifies descendant traversal and filter rules for a GuardStep.
type GuardRule struct {
	RootPID        uint32         `json:"root_pid"`
	MaxLevels      *uint32        `json:"max_levels,omitempty"`
	Filter         *ProcessFilter `json:"filter,omitempty"`
	CpuMode        CpuMode        `json:"cpu_mode,omitempty"`
	SampleDuration time.Duration  `json:"sample_duration,omitempty"`
}

// GuardAction specifies what action to take on matched descendants.
type GuardAction struct {
	Kind    string `json:"kind,omitempty"`
	Signal  int    `json:"signal,omitempty"`
	Cascade bool   `json:"cascade,omitempty"`
}

// GuardConfig configures a single GuardStep evaluation.
type GuardConfig struct {
	Rule          GuardRule   `json:"rule"`
	Action        GuardAction `json:"action"`
	ActionEnabled bool        `json:"action_enabled"`
	MaxTargets    uint32      `json:"max_targets,omitempty"`
}

// GuardEvent records the evaluation and optional remediation results of a single GuardStep.
type GuardEvent struct {
	SchemaID      string   `json:"schema_id"`
	Timestamp     string   `json:"timestamp"`
	Platform      string   `json:"platform"`
	Matched       uint32   `json:"matched"`
	Targeted      uint32   `json:"targeted"`
	Killed        uint32   `json:"killed"`
	Failed        uint32   `json:"failed"`
	SkippedSafety uint32   `json:"skipped_safety"`
	Warnings      []string `json:"warnings"`
}

// GuardStep performs one evaluation (and optional remediation) cycle of a GuardConfig.
func GuardStep(cfg *GuardConfig) (*GuardEvent, error) {
	if cfg == nil {
		return nil, syserror.InvalidArgument("guard config is required")
	}
	if err := ValidatePID(cfg.Rule.RootPID); err != nil {
		return nil, err
	}
	if cfg.Rule.Filter != nil {
		if err := cfg.Rule.Filter.Validate(); err != nil {
			return nil, err
		}
	}
	if cfg.Rule.SampleDuration < 0 {
		return nil, syserror.InvalidArgument("sample duration must be >= 0")
	}
	mode, err := parseCpuMode(cfg.Rule.CpuMode)
	if err != nil {
		return nil, err
	}
	if cfg.Action.Kind != "" && cfg.Action.Kind != "kill_descendants" {
		return nil, syserror.InvalidArgument(fmt.Sprintf("unsupported guard action kind: %q", cfg.Action.Kind))
	}

	maxTargets := cfg.MaxTargets
	if maxTargets == 0 {
		maxTargets = 64
	}

	desc, err := DescendantsWithOptions(cfg.Rule.RootPID, &DescendantsOptions{
		MaxLevels:      cfg.Rule.MaxLevels,
		Filter:         nil, // Keep full tree so Cascade can traverse children of matched nodes
		CpuMode:        mode,
		SampleDuration: cfg.Rule.SampleDuration,
	})
	if err != nil {
		return nil, err
	}

	matchedPIDs, err := SelectDescendantTargets(desc, cfg.Rule.Filter, false)
	if err != nil {
		return nil, err
	}
	matched := safeUint32Len(len(matchedPIDs))

	warnings := []string{}
	if !cfg.ActionEnabled {
		warnings = append(warnings, "action disabled; no signals sent")
		return &GuardEvent{
			SchemaID:      GuardEventSchemaID,
			Timestamp:     nowRFC3339(),
			Platform:      CurrentPlatform(),
			Matched:       matched,
			Targeted:      0,
			Killed:        0,
			Failed:        0,
			SkippedSafety: 0,
			Warnings:      warnings,
		}, nil
	}

	targetPIDs, err := SelectDescendantTargets(desc, cfg.Rule.Filter, cfg.Action.Cascade)
	if err != nil {
		return nil, err
	}

	if safeUint32Len(len(targetPIDs)) > maxTargets {
		targetPIDs = targetPIDs[:maxTargets]
		warnings = append(warnings, fmt.Sprintf("target count capped at max_targets=%d for this cycle", maxTargets))
	}

	selfPID, parentPID := safeSelfAndParentPID()
	safeTargets := make([]uint32, 0, len(targetPIDs))
	skippedSafety := uint32(0)

	for _, pid := range targetPIDs {
		if pid == cfg.Rule.RootPID || pid == selfPID || pid == 1 || (parentPID > 0 && pid == parentPID) {
			skippedSafety++
			continue
		}
		safeTargets = append(safeTargets, pid)
	}
	if skippedSafety > 0 {
		warnings = append(warnings, fmt.Sprintf("skipped %d unsafe target(s) (self/PID1/parent/root)", skippedSafety))
	}

	if len(safeTargets) == 0 {
		return &GuardEvent{
			SchemaID:      GuardEventSchemaID,
			Timestamp:     nowRFC3339(),
			Platform:      CurrentPlatform(),
			Matched:       matched,
			Targeted:      0,
			Killed:        0,
			Failed:        0,
			SkippedSafety: skippedSafety,
			Warnings:      warnings,
		}, nil
	}

	sig := cfg.Action.Signal
	if sig == 0 {
		sig = signal.SIGTERM
	}
	batchRes, err := signal.KillMany(safeTargets, sig)
	if err != nil {
		return nil, err
	}

	return &GuardEvent{
		SchemaID:      GuardEventSchemaID,
		Timestamp:     nowRFC3339(),
		Platform:      CurrentPlatform(),
		Matched:       matched,
		Targeted:      safeUint32Len(len(safeTargets)),
		Killed:        safeUint32Len(len(batchRes.Succeeded)),
		Failed:        safeUint32Len(len(batchRes.Failed)),
		SkippedSafety: skippedSafety,
		Warnings:      warnings,
	}, nil
}

// GuardPreset defines standard polling and sampling intervals for GuardRunner.
type GuardPreset string

const (
	GuardPresetInteractive GuardPreset = "interactive"
	GuardPresetBackground  GuardPreset = "background"
	GuardPresetWatchdog    GuardPreset = "watchdog"
)

// Interval returns the polling interval for the preset (0 for unknown preset).
func (p GuardPreset) Interval() time.Duration {
	switch p {
	case GuardPresetInteractive:
		return 3 * time.Second
	case GuardPresetBackground:
		return 3 * time.Minute
	case GuardPresetWatchdog:
		return 5 * time.Minute
	default:
		return 0
	}
}

// SampleDuration returns the CPU sampling duration for the preset (0 for unknown preset).
func (p GuardPreset) SampleDuration() time.Duration {
	switch p {
	case GuardPresetInteractive:
		return 2 * time.Second
	case GuardPresetBackground:
		return 3 * time.Second
	case GuardPresetWatchdog:
		return 5 * time.Second
	default:
		return 0
	}
}

// GuardRunnerConfig configures a GuardRunner.
type GuardRunnerConfig struct {
	Guard         GuardConfig
	Interval      time.Duration
	MaxIterations *uint64
}

// GuardRunner manages repeated GuardStep evaluations on demand (Tick) or until stopped/closed.
type GuardRunner struct {
	mu         sync.Mutex
	cfg        GuardRunnerConfig
	iterations uint64
	stopped    bool
	closed     bool
}

// NewGuardRunner validates the configuration and creates a GuardRunner.
func NewGuardRunner(cfg *GuardRunnerConfig) (*GuardRunner, error) {
	if cfg == nil {
		return nil, syserror.InvalidArgument("guard runner config is required")
	}
	if cfg.Interval <= 0 {
		return nil, syserror.InvalidArgument("interval must be > 0")
	}
	if err := ValidatePID(cfg.Guard.Rule.RootPID); err != nil {
		return nil, err
	}
	if cfg.Guard.Rule.Filter != nil {
		if err := cfg.Guard.Rule.Filter.Validate(); err != nil {
			return nil, err
		}
	}
	if cfg.Guard.Rule.SampleDuration < 0 {
		return nil, syserror.InvalidArgument("sample duration must be >= 0")
	}
	if _, err := parseCpuMode(cfg.Guard.Rule.CpuMode); err != nil {
		return nil, err
	}
	return &GuardRunner{
		cfg: *cfg,
	}, nil
}

// Tick executes a single GuardStep evaluation unless the runner has been stopped, closed, or reached MaxIterations.
// Returns (nil, nil) when stopped or MaxIterations is reached; returns ErrInvalidArgument if called after Close().
func (r *GuardRunner) Tick() (*GuardEvent, error) {
	if r == nil {
		return nil, syserror.InvalidArgument("guard runner is nil")
	}
	r.mu.Lock()
	if r.closed {
		r.mu.Unlock()
		return nil, syserror.InvalidArgument("guard runner is closed")
	}
	if r.stopped {
		r.mu.Unlock()
		return nil, nil
	}
	if r.cfg.MaxIterations != nil && r.iterations >= *r.cfg.MaxIterations {
		r.stopped = true
		r.mu.Unlock()
		return nil, nil
	}
	r.iterations++
	guardCfg := r.cfg.Guard
	r.mu.Unlock()

	return GuardStep(&guardCfg)
}

// Stop signals the runner to stop further ticks (subsequent Tick calls return (nil, nil)).
func (r *GuardRunner) Stop() {
	if r == nil {
		return
	}
	r.mu.Lock()
	r.stopped = true
	r.mu.Unlock()
}

// Close marks the runner closed (subsequent Tick calls return ErrInvalidArgument).
func (r *GuardRunner) Close() {
	if r == nil {
		return
	}
	r.mu.Lock()
	r.stopped = true
	r.closed = true
	r.mu.Unlock()
}
