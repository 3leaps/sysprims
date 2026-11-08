// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build darwin || linux

package procobserve_test

import (
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/3leaps/sysprims/evidence"
	"github.com/3leaps/sysprims/procobserve"
)

type ownedSleepChild struct {
	cmd      *exec.Cmd
	reaped   bool
	execPath string
}

func spawnOwnedSleep(t *testing.T) *ownedSleepChild {
	t.Helper()
	var sleepPath string
	for _, candidate := range []string{"/bin/sleep", "/usr/bin/sleep"} {
		if _, err := os.Stat(candidate); err == nil {
			sleepPath = candidate
			break
		}
	}
	if sleepPath == "" {
		t.Fatal("neither /bin/sleep nor /usr/bin/sleep exists")
	}

	// #nosec G204 -- sleepPath is selected from a fixed two-element constant allow-list
	cmd := exec.Command(sleepPath, "30")
	if err := cmd.Start(); err != nil {
		t.Fatalf("start sleep child: %v", err)
	}
	c := &ownedSleepChild{
		cmd:      cmd,
		execPath: sleepPath,
	}
	t.Cleanup(func() {
		if !c.reaped {
			_ = c.cmd.Process.Kill()
			_ = c.cmd.Wait()
			c.reaped = true
		}
	})
	return c
}

func (c *ownedSleepChild) pid(t *testing.T) uint32 {
	t.Helper()
	rawPID := c.cmd.Process.Pid
	if rawPID <= 0 || uint64(rawPID) > uint64(procobserve.MaxObservablePID) {
		t.Fatalf("spawned child PID out of observable bounds: %d", rawPID)
	}
	return uint32(rawPID) // #nosec G115 -- bounds checked immediately above
}

func (c *ownedSleepChild) killUnreaped() {
	_ = c.cmd.Process.Kill()
}

func (c *ownedSleepChild) reap() {
	if !c.reaped {
		_ = c.cmd.Wait()
		c.reaped = true
	}
}

func supportedContextFactsForOS() []string {
	if runtime.GOOS == "linux" {
		return []string{
			"security_principal",
			"namespace_context_digest",
			"executable_digest",
			"arguments_digest",
			"security_context_digest",
		}
	}
	return []string{
		"security_principal",
		"executable_digest",
		"arguments_digest",
	}
}

func mustMap(t *testing.T, v any, label string) map[string]any {
	t.Helper()
	m, ok := v.(map[string]any)
	if !ok {
		t.Fatalf("%s: expected map[string]any, got %T", label, v)
	}
	return m
}

func TestDefaultAndOptInDisclosureMatchSpawnedChildImageAndArgv(t *testing.T) {
	child := spawnOwnedSleep(t)
	pid := child.pid(t)
	hs := sampleHostScope()

	defaultOpts := procobserve.DefaultObservationOptions()
	defaultOpts.ObservedAt = "2026-11-06T12:00:00Z"

	obsDefault, err := procobserve.ObserveProcess(pid, hs, defaultOpts)
	if err != nil {
		t.Fatalf("ObserveProcess default: %v", err)
	}
	procDefault := mustMap(t, obsDefault["process"], "process")
	stateDefault := mustMap(t, procDefault["process_state"], "process_state")
	if stateDefault["status"] != "observed" || stateDefault["value"] != "live" {
		t.Fatalf("unexpected default process_state: %v", stateDefault)
	}
	ecDefault := mustMap(t, procDefault["execution_context"], "execution_context")
	spDefault := mustMap(t, ecDefault["security_principal"], "security_principal")
	if spDefault["status"] != "observed" {
		t.Fatalf("expected default security_principal observed, got %v", spDefault)
	}
	for _, omitted := range []string{
		"namespace_context_digest",
		"executable_digest",
		"arguments_digest",
		"security_context_digest",
	} {
		f := mustMap(t, ecDefault[omitted], omitted)
		if f["status"] != "not-probed" {
			t.Fatalf("expected default %s status=not-probed, got %v", omitted, f)
		}
	}

	fullOpts := defaultOpts.WithAllContextFacts()
	obsFull, err := procobserve.ObserveProcess(pid, hs, fullOpts)
	if err != nil {
		t.Fatalf("ObserveProcess full: %v", err)
	}
	procFull := mustMap(t, obsFull["process"], "process")
	ecFull := mustMap(t, procFull["execution_context"], "execution_context")

	// #nosec G304 -- child.execPath is selected from constant ["/bin/sleep", "/usr/bin/sleep"]
	sleepBytes, err := os.ReadFile(child.execPath)
	if err != nil {
		t.Fatalf("read %s: %v", child.execPath, err)
	}
	wantExeDigest := evidence.DeriveExecutableDigest(sleepBytes)
	exeFact := mustMap(t, ecFull["executable_digest"], "executable_digest")
	if exeFact["status"] != "observed" || exeFact["digest"] != wantExeDigest {
		t.Fatalf("executable_digest mismatch: got %v, want %s", exeFact, wantExeDigest)
	}

	wantArgsDigest := evidence.DeriveArgumentsDigest([][]byte{[]byte(child.execPath), []byte("30")})
	argsFact := mustMap(t, ecFull["arguments_digest"], "arguments_digest")
	if argsFact["status"] != "observed" || argsFact["digest"] != wantArgsDigest {
		t.Fatalf("arguments_digest mismatch: got %v, want %s", argsFact, wantArgsDigest)
	}

	nsFact := mustMap(t, ecFull["namespace_context_digest"], "namespace_context_digest")
	secFact := mustMap(t, ecFull["security_context_digest"], "security_context_digest")
	if runtime.GOOS == "linux" {
		if nsFact["status"] != "observed" || secFact["status"] != "observed" {
			t.Fatalf("expected Linux ns/secctx observed, got ns=%v sec=%v", nsFact, secFact)
		}
	} else {
		if nsFact["status"] != "unsupported" || secFact["status"] != "unsupported" {
			t.Fatalf("expected macOS ns/secctx unsupported when opted in, got ns=%v sec=%v", nsFact, secFact)
		}
	}
}

func TestContinuityHandleLiveZombieReapedAndPIDReuse(t *testing.T) {
	child := spawnOwnedSleep(t)
	pid := child.pid(t)
	hs := sampleHostScope()
	opts := procobserve.DefaultObservationOptions().WithAllContextFacts()
	opts.ObservedAt = "2026-11-06T12:00:00Z"

	handle, err := procobserve.OpenContinuityHandle(pid, hs, opts)
	if err != nil {
		t.Fatalf("OpenContinuityHandle: %v", err)
	}
	defer func() {
		_ = handle.Close()
	}()

	if handle.PID() != pid {
		t.Fatalf("PID() = %d, want %d", handle.PID(), pid)
	}
	// Live value/pointer JSON and text serialization/deserialization must refuse without clobbering state.
	_, mValErr := json.Marshal(*handle)
	assertRefusalCode(t, mValErr, "continuity-handle-not-serializable")
	_, mPtrErr := json.Marshal(handle)
	assertRefusalCode(t, mPtrErr, "continuity-handle-not-serializable")
	assertRefusalCode(t, json.Unmarshal([]byte(`{}`), handle), "continuity-handle-not-serializable")
	_, mtErr := (*handle).MarshalText()
	assertRefusalCode(t, mtErr, "continuity-handle-not-serializable")
	assertRefusalCode(t, handle.UnmarshalText([]byte("bad")), "continuity-handle-not-serializable")

	exited, err := handle.PollExited()
	if err != nil || exited {
		t.Fatalf("alive child PollExited() = (%v, %v), want (false, nil)", exited, err)
	}
	exited, err = handle.WaitExit(20 * time.Millisecond)
	if err != nil || exited {
		t.Fatalf("alive child WaitExit(20ms) = (%v, %v), want (false, nil)", exited, err)
	}

	strictPolicy := evidence.ComparisonPolicy{
		BaselineHostScopeTrusted:   true,
		CandidateHostScopeTrusted:  true,
		RequireLive:                true,
		RequiredContextFacts:       supportedContextFactsForOS(),
		DetectObservedContextDrift: true,
	}

	_, outLive, err := handle.Reverify(strictPolicy, "2026-11-06T12:00:01Z")
	if err != nil || outLive.Code() != "match" {
		t.Fatalf("alive Reverify = (%v, %v), want match", outLive, err)
	}

	// Kill without reaping so child transitions to zombie.
	child.killUnreaped()
	exited, err = handle.WaitExit(3 * time.Second)
	if err != nil || !exited {
		t.Fatalf("WaitExit after kill = (%v, %v), want (true, nil)", exited, err)
	}
	exited, err = handle.PollExited()
	if err != nil || !exited {
		t.Fatalf("idempotent PollExited = (%v, %v), want (true, nil)", exited, err)
	}

	sawZombie := false
	for range 40 {
		snap, rErr := handle.Reobserve("2026-11-06T12:00:02Z")
		if rErr != nil {
			t.Fatalf("Reobserve unreaped: %v", rErr)
		}
		pObj := mustMap(t, snap["process"], "process")
		psObj := mustMap(t, pObj["process_state"], "process_state")
		if psObj["value"] == "zombie" {
			sawZombie = true
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if !sawZombie {
		t.Fatal("unreaped terminated child never reported zombie state")
	}

	_, outZombieStrict, err := handle.Reverify(strictPolicy, "2026-11-06T12:00:02Z")
	if err != nil {
		t.Fatalf("zombie Reverify(require_live=true) error: %v", err)
	}
	relaxedPolicy := evidence.ComparisonPolicy{
		BaselineHostScopeTrusted:   true,
		CandidateHostScopeTrusted:  true,
		RequireLive:                false,
		RequiredContextFacts:       []string{},
		DetectObservedContextDrift: false,
	}
	_, outZombieRelaxed, err := handle.Reverify(relaxedPolicy, "2026-11-06T12:00:02Z")
	if err != nil {
		t.Fatalf("zombie Reverify(require_live=false) error: %v", err)
	}
	if runtime.GOOS == "linux" {
		if outZombieStrict.Code() != "insufficient:missing-target-pid-namespace" {
			t.Fatalf("Linux zombie Reverify(require_live=true) = %v, want insufficient:missing-target-pid-namespace", outZombieStrict)
		}
		if outZombieRelaxed.Code() != "insufficient:missing-target-pid-namespace" {
			t.Fatalf("Linux zombie Reverify(require_live=false) = %v, want insufficient:missing-target-pid-namespace", outZombieRelaxed)
		}
	} else {
		if outZombieStrict.Code() != "mismatch:process-not-live" {
			t.Fatalf("zombie Reverify(require_live=true) = %v, want mismatch:process-not-live", outZombieStrict)
		}
		if outZombieRelaxed.Code() != "match" {
			t.Fatalf("zombie Reverify(require_live=false) = %v, want match", outZombieRelaxed)
		}
	}

	// Reap child so PID disappears.
	child.reap()
	_, outReaped, err := handle.Reverify(strictPolicy, "2026-11-06T12:00:03Z")
	if err != nil || outReaped.Code() != "mismatch:process-exited" {
		t.Fatalf("reaped Reverify = (%v, %v), want mismatch:process-exited", outReaped, err)
	}

	// Opening a handle on reaped PID must refuse with Status == "not-present".
	_, reopenErr := procobserve.OpenContinuityHandle(pid, hs, opts)
	var ref *procobserve.CollectorRefusal
	if !errors.As(reopenErr, &ref) || ref.Status != "not-present" {
		t.Fatalf("expected not-present refusal reopening reaped PID, got %v", reopenErr)
	}

	// Close is idempotent; WaitExit, Reobserve, and Reverify after Close refuse.
	if err := handle.Close(); err != nil {
		t.Fatalf("first Close: %v", err)
	}
	if err := handle.Close(); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	_, closedWaitErr := handle.WaitExit(0)
	assertRefusalCode(t, closedWaitErr, "continuity-handle-closed")
	_, closedReobsErr := handle.Reobserve("2026-11-06T12:00:04Z")
	assertRefusalCode(t, closedReobsErr, "continuity-handle-closed")
	_, _, closedRevErr := handle.Reverify(strictPolicy, "2026-11-06T12:00:04Z")
	assertRefusalCode(t, closedRevErr, "continuity-handle-closed")

	// PID reuse check using a distinct live child incarnation placed at the baseline PID.
	child2 := spawnOwnedSleep(t)
	reusedCand, err := procobserve.ObserveProcess(child2.pid(t), hs, opts)
	if err != nil {
		t.Fatalf("observe second child: %v", err)
	}
	baseObs := handle.BaselineObservation()
	baseStart := mustMap(t, mustMap(t, baseObs["process"], "base process")["start_identity"], "base start")["value"]
	candStart := mustMap(t, mustMap(t, reusedCand["process"], "cand process")["start_identity"], "cand start")["value"]
	if baseStart == candStart {
		time.Sleep(25 * time.Millisecond)
		child3 := spawnOwnedSleep(t)
		reusedCand, err = procobserve.ObserveProcess(child3.pid(t), hs, opts)
		if err != nil {
			t.Fatalf("observe third child: %v", err)
		}
	}
	mustMap(t, reusedCand["process"], "cand process")["pid"] = strconv.FormatUint(uint64(pid), 10)
	reuseOut := evidence.CompareNativeObservations(baseObs, reusedCand, strictPolicy, procobserve.AcceptedObservationProfile())
	if reuseOut.Code() != "mismatch:start-identity-mismatch" {
		t.Fatalf("PID reuse outcome = %s, want mismatch:start-identity-mismatch", reuseOut.Code())
	}
}

func TestBuildObservationReportFreshnessAndSessionProjection(t *testing.T) {
	child := spawnOwnedSleep(t)
	pid := child.pid(t)
	hs := sampleHostScope()
	opts := procobserve.DefaultObservationOptions().WithAllContextFacts()
	opts.ObservedAt = "2026-11-06T12:00:00Z"

	obs, err := procobserve.ObserveProcess(pid, hs, opts)
	if err != nil {
		t.Fatalf("ObserveProcess: %v", err)
	}

	provider := "macos-kqueue-proc"
	if runtime.GOOS == "linux" {
		provider = "linux-pidfd"
	}
	meta := procobserve.DefaultReportMetadata()
	meta.ExitHandleCapability = &procobserve.CapabilityProbe{
		Provider: provider,
		Status:   "available",
	}

	admittedReport, err := procobserve.BuildObservationReport(obs, meta)
	if err != nil {
		t.Fatalf("BuildObservationReport: %v", err)
	}
	reparsed, err := evidence.AdmitAndValidateReport(admittedReport.CanonicalBytes, nil)
	if err != nil {
		t.Fatalf("AdmitAndValidateReport: %v", err)
	}
	if reparsed.Digest != admittedReport.Digest {
		t.Fatalf("digest mismatch: %s vs %s", reparsed.Digest, admittedReport.Digest)
	}

	accepted := procobserve.AcceptedObservationProfile()
	now5 := "2026-11-06T12:00:05Z"
	if err := evidence.AdmitLiveObservation(obs, &now5, accepted, admittedReport.Value, nil); err != nil {
		t.Fatalf("+5s boundary must pass freshness gate: %v", err)
	}

	now6 := "2026-11-06T12:00:06Z"
	staleErr := evidence.AdmitLiveObservation(obs, &now6, accepted, admittedReport.Value, nil)
	staleCode, _ := evidence.RefusalCode(staleErr)
	if staleCode != "stale-process-observation" {
		t.Fatalf("expected stale-process-observation at +6s, got %v (code=%q)", staleErr, staleCode)
	}

	now1 := "2026-11-06T12:00:01Z"
	projected, projErr := evidence.ProjectToLocalProcessSessionProcess(
		obs,
		true,
		&now1,
		accepted,
		admittedReport.Value,
		nil,
		nil,
	)
	if runtime.GOOS == "linux" {
		if projErr != nil {
			t.Fatalf("Linux full-context observation must project to session process: %v", projErr)
		}
		hsURN, _ := projected["host_scope"].(string)
		if !strings.HasPrefix(hsURN, "urn:3leaps:identity-evidence:host-scope:v0:sha256:") {
			t.Fatalf("unexpected projected host_scope: %q", hsURN)
		}
		si := mustMap(t, projected["start_identity"], "projected start_identity")
		siProf := mustMap(t, si["profile"], "projected start_identity.profile")
		if siProf["id"] != evidence.LinuxProcfsStatTimensScopedProfileID {
			t.Fatalf("unexpected projected profile ID: %v", siProf["id"])
		}
		siVal, _ := si["value"].(string)
		if !strings.HasPrefix(siVal, "timens:") || !strings.Contains(siVal, ";ticks=") {
			t.Fatalf("unexpected projected start_identity value: %q", siVal)
		}
	} else if projErr == nil {
		t.Fatal("macOS observation without Linux namespace/secctx digests must fail session projection closed")
	}
}
