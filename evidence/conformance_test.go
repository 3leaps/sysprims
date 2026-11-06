// SPDX-License-Identifier: MIT OR Apache-2.0

package evidence_test

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/3leaps/sysprims/evidence"
)

func corpusRoot(t *testing.T) string {
	t.Helper()
	if env := os.Getenv("EVIDENCE_CORPUS_DIR"); env != "" {
		return env
	}
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	p := filepath.Clean(filepath.Join(filepath.Dir(file), "..", "crates", "sysprims-evidence", "tests", "identity-evidence-v0"))
	if _, err := os.Stat(p); err != nil {
		t.Skipf("vendored conformance corpus not found at %s (set EVIDENCE_CORPUS_DIR for out-of-tree runs)", p)
	}
	return p
}

func sha256Hex(b []byte) string {
	sum := sha256.Sum256(b)
	return "sha256:" + hex.EncodeToString(sum[:])
}

func TestCorpusManifestAndProfilePinsMatch(t *testing.T) {
	if !strings.HasPrefix(evidence.CorpusDigest, "sha256:") {
		t.Fatalf("unexpected CorpusDigest: %s", evidence.CorpusDigest)
	}
	root := corpusRoot(t)
	rawManifest, err := os.ReadFile(filepath.Join(root, "MANIFEST.json"))
	if err != nil {
		t.Fatalf("read MANIFEST.json: %v", err)
	}
	var manifest struct {
		FileCount int               `json:"file_count"`
		Files     map[string]string `json:"files"`
	}
	if err := json.Unmarshal(rawManifest, &manifest); err != nil {
		t.Fatalf("unmarshal MANIFEST.json: %v", err)
	}
	if len(manifest.Files) != manifest.FileCount {
		t.Fatalf("file count mismatch: got %d, manifest says %d", len(manifest.Files), manifest.FileCount)
	}
	for rel, expectedSHA := range manifest.Files {
		b, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(rel)))
		if err != nil {
			t.Fatalf("read %s: %v", rel, err)
		}
		if got := sha256Hex(b); got != expectedSHA {
			t.Fatalf("SHA-256 drift on %s: got %s, want %s", rel, got, expectedSHA)
		}
	}

	obsDir := filepath.Join(root, "profiles", "native-process-observation")
	profBytes, err := os.ReadFile(filepath.Join(obsDir, "profile.json"))
	if err != nil {
		t.Fatal(err)
	}
	if got := sha256Hex(profBytes); got != evidence.NativeProcessObservationProfileRevision {
		t.Fatalf("profile.json revision mismatch: got %s", got)
	}
	readmeBytes, err := os.ReadFile(filepath.Join(obsDir, "README.md"))
	if err != nil {
		t.Fatal(err)
	}
	if got := sha256Hex(readmeBytes); got != evidence.NativeProcessObservationReadmeRevision {
		t.Fatalf("README.md revision mismatch: got %s", got)
	}
	schemaBytes, err := os.ReadFile(filepath.Join(obsDir, "evidence.schema.json"))
	if err != nil {
		t.Fatal(err)
	}
	if got := sha256Hex(schemaBytes); got != evidence.NativeProcessObservationSchemaRevision {
		t.Fatalf("evidence.schema.json revision mismatch: got %s", got)
	}
}

func TestStreamingReaderCapsOversizedInput(t *testing.T) {
	exact := bytes.Repeat([]byte{' '}, evidence.MaxReportBytes)
	exact[0] = '{'
	exact[evidence.MaxReportBytes-1] = '}'
	okWire, err := evidence.AdmitReportReader(bytes.NewReader(exact))
	if err != nil {
		t.Fatalf("exact 256 KiB reader failed: %v", err)
	}
	if string(okWire.CanonicalBytes) != "{}" {
		t.Fatalf("unexpected canonical bytes: %q", okWire.CanonicalBytes)
	}

	over := bytes.Repeat([]byte{' '}, evidence.MaxReportBytes+4096)
	_, err = evidence.AdmitReportReader(bytes.NewReader(over))
	code, ok := evidence.RefusalCode(err)
	if !ok || code != "report-too-large" {
		t.Fatalf("expected report-too-large, got %v (%q)", err, code)
	}
}

func TestStrictUTCAndJSONEqualInvariants(t *testing.T) {
	if ts, ok := evidence.ParseStrictUTCUnix("2026-10-29T12:00:00Z"); !ok || ts != 1_793_275_200 {
		t.Fatalf("unexpected unix timestamp: %d, %v", ts, ok)
	}
	for _, bad := range []string{
		"2026-10-29T12:00:00.000Z",
		"2026-10-29T12:00:00+00:00",
		"2026-10-29t12:00:00z",
		"2026-02-29T12:00:00Z",
		"2026-10-29T12:00:60Z",
	} {
		if _, ok := evidence.ParseStrictUTCUnix(bad); ok {
			t.Fatalf("expected %q to be rejected", bad)
		}
	}

	negZero := math.Copysign(0, -1)
	if !evidence.JSONEqual(
		map[string]any{"a": []any{1, negZero}},
		map[string]any{"a": []any{1.0, 0}},
	) {
		t.Fatal("expected numeric equality across int/float64 and -0.0/0")
	}
	if evidence.JSONEqual(map[string]any{"a": []any{true}}, map[string]any{"a": []any{1}}) {
		t.Fatal("bool true must not equal number 1")
	}
	if evidence.JSONEqual([]any{false}, []any{0}) {
		t.Fatal("bool false must not equal number 0")
	}
	if evidence.JSONEqual([]any{1, 2}, []any{2, 1}) {
		t.Fatal("array order must be significant")
	}
	if !evidence.JSONEqual(
		map[string]any{"a": 1, "b": 2},
		map[string]any{"b": 2.0, "a": 1.0},
	) {
		t.Fatal("object member order must be immaterial")
	}
}

func TestPreimageGoldensAndNumericBounds(t *testing.T) {
	if got := evidence.DeriveArgumentsDigest([][]byte{[]byte("/usr/bin/sleep"), []byte("300")}); got != "sha256:3846a65c6cdc97a8388e79fad49c2837d9e0abacec2bdeef7f4b54d361b970f1" {
		t.Fatalf("argv digest mismatch: %s", got)
	}
	if got := evidence.DeriveExecutableDigest([]byte("\x7fELF-synthetic-sleep-image-v0\n")); got != "sha256:58b9da5ab4aede636708b24b04ccfa6946746ab58058ec2e15ebda26b0a6d909" {
		t.Fatalf("exe digest mismatch: %s", got)
	}

	nsMap := map[string]string{
		"cgroup":            "ns:0:4:4026531835",
		"ipc":               "ns:0:4:4026531839",
		"mnt":               "ns:0:4:4026531840",
		"net":               "ns:0:4:4026531992",
		"pid":               "ns:0:4:4026531836",
		"time":              "ns:0:4:4026531834",
		"time_for_children": "ns:0:4:4026531834",
		"user":              "ns:0:4:4026531837",
		"uts":               "ns:0:4:4026531838",
	}
	nsDig, err := evidence.DeriveLinuxNamespaceContextDigest(nsMap)
	if err != nil || nsDig != "sha256:c74496ded4d8d5b0f0e11bfed4c3643e2496880b26502f885880355ec117bded" {
		t.Fatalf("ns digest mismatch: %s, err=%v", nsDig, err)
	}

	maxNS := make(map[string]string, len(nsMap))
	for k, v := range nsMap {
		maxNS[k] = v
	}
	maxNS["user"] = "ns:4294967295:4294967295:18446744073709551615"
	if _, err := evidence.DeriveLinuxNamespaceContextDigest(maxNS); err != nil {
		t.Fatalf("u32/u64 max ns rejected: %v", err)
	}

	for _, badEntry := range []string{
		"ns:4294967296:0:1",
		"ns:0:4294967296:1",
		"ns:0:4:18446744073709551616",
	} {
		bad := make(map[string]string, len(nsMap))
		for k, v := range nsMap {
			bad[k] = v
		}
		bad["user"] = badEntry
		_, err := evidence.DeriveLinuxNamespaceContextDigest(bad)
		if code, _ := evidence.RefusalCode(err); code != "invalid-linux-ns-entry" {
			t.Fatalf("expected invalid-linux-ns-entry for %s, got %q", badEntry, code)
		}
	}

	partial := make(map[string]string, len(nsMap))
	for k, v := range nsMap {
		if k != "time" {
			partial[k] = v
		}
	}
	_, err = evidence.DeriveLinuxNamespaceContextDigest(partial)
	if code, _ := evidence.RefusalCode(err); code != "incomplete-linux-namespace-set" {
		t.Fatalf("expected incomplete-linux-namespace-set, got %q", code)
	}

	secDig, err := evidence.DeriveLinuxSecurityContextDigest(
		[]byte("observer_user_ns=userns:0:4:4026531837;groups=27,1000;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"),
	)
	if err != nil || secDig != "sha256:954107c2e47b6b4406093ee4e6cf14a66ee9c7350d6550f606b9995104d44976" {
		t.Fatalf("sec digest mismatch: %s, err=%v", secDig, err)
	}
	if _, err := evidence.DeriveLinuxSecurityContextDigest(
		[]byte("observer_user_ns=userns:4294967295:4294967295:18446744073709551615;groups=0,4294967295;no_new_privs=1;seccomp=4294967295;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"),
	); err != nil {
		t.Fatalf("u32/u64 max secctx rejected: %v", err)
	}

	for _, badSec := range []string{
		"observer_user_ns=userns:0:4:4026531837;groups=4294967296;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000",
		"observer_user_ns=userns:0:4:4026531837;groups=1000,27;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000",
		"observer_user_ns=userns:0:4:4026531837;groups=27;no_new_privs=1;seccomp=4294967296;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000",
		"observer_user_ns=userns:4294967296:4:1;groups=27;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000",
	} {
		_, err := evidence.DeriveLinuxSecurityContextDigest([]byte(badSec))
		if code, _ := evidence.RefusalCode(err); code != "invalid-linux-security-context" {
			t.Fatalf("expected invalid-linux-security-context, got %q", code)
		}
	}

	if !evidence.LinuxIDIsUnambiguouslyMapped(1000, []evidence.IDMapExtent{{First: 0, Count: math.MaxUint32}}, 65534) {
		t.Fatal("expected initial ns 1000 mapped")
	}
	if !evidence.LinuxIDIsUnambiguouslyMapped(65534, []evidence.IDMapExtent{{First: 0, Count: math.MaxUint32}}, 65534) {
		t.Fatal("expected initial ns 65534 mapped")
	}
	if evidence.LinuxIDIsUnambiguouslyMapped(math.MaxUint32, []evidence.IDMapExtent{{First: 0, Count: math.MaxUint32}}, 65534) {
		t.Fatal("expected u32::MAX overflow sentinel unmapped")
	}
	if !evidence.LinuxIDIsUnambiguouslyMapped(1000, []evidence.IDMapExtent{{First: 0, Count: 65536}}, 65534) {
		t.Fatal("expected non-initial 1000 mapped")
	}
	if evidence.LinuxIDIsUnambiguouslyMapped(65534, []evidence.IDMapExtent{{First: 0, Count: 65536}}, 65534) {
		t.Fatal("expected non-initial overflowuid 65534 fail-closed")
	}
	if evidence.LinuxIDIsUnambiguouslyMapped(70000, []evidence.IDMapExtent{{First: 0, Count: 65536}}, 65534) {
		t.Fatal("expected out-of-extent 70000 unmapped")
	}

	root := corpusRoot(t)
	rawObs, err := os.ReadFile(filepath.Join(root, "profiles", "native-process-observation", "evidence.linux.example.json"))
	if err != nil {
		t.Fatal(err)
	}
	var linuxObs map[string]any
	if err := json.Unmarshal(rawObs, &linuxObs); err != nil {
		t.Fatal(err)
	}
	urn, err := evidence.ProjectHostScopeURN(linuxObs["process"].(map[string]any)["host_scope"])
	if err != nil || urn != "urn:3leaps:identity-evidence:host-scope:v0:sha256:8e3d1953b6494fc7c0399bf1487b90e57917b9a0651670dd83bc62a0af5d0a8d" {
		t.Fatalf("unexpected host_scope URN: %s, err=%v", urn, err)
	}
}

func TestFullConformanceAndParitySuitePasses(t *testing.T) {
	root := corpusRoot(t)
	parityPath := os.Getenv("EVIDENCE_PARITY_CASES")
	if parityPath == "" {
		parityPath = filepath.Join(filepath.Dir(root), "parity-cases.json")
	}
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	cmd := exec.Command("go", "run", "./internal/difftool")
	cmd.Dir = filepath.Dir(file)
	cmd.Env = append(
		os.Environ(),
		"GOWORK=off",
		"CGO_ENABLED=0",
		"EVIDENCE_CORPUS_DIR="+root,
		"EVIDENCE_PARITY_CASES="+parityPath,
	)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("difftool failed: %v\noutput:\n%s", err, string(out))
	}
	lines := strings.Split(strings.TrimSpace(string(out)), "\n")
	if len(lines) < 280 {
		t.Fatalf("expected >= 280 NDJSON parity lines, got %d", len(lines))
	}
}
