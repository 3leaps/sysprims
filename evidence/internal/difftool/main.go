// SPDX-License-Identifier: MIT OR Apache-2.0

// Command difftool emits deterministic RFC 8785 NDJSON evaluation records over
// the identity-evidence/v0 conformance corpus and parity-cases.json for
// cross-language parity verification against crates/sysprims-evidence.
package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"

	"github.com/3leaps/canonprims/canon"
	"github.com/3leaps/sysprims/evidence"
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

func deepCloneJSON(v any) any {
	switch x := v.(type) {
	case map[string]any:
		out := make(map[string]any, len(x))
		for k, val := range x {
			out[k] = deepCloneJSON(val)
		}
		return out
	case []any:
		out := make([]any, len(x))
		for i, val := range x {
			out[i] = deepCloneJSON(val)
		}
		return out
	default:
		return x
	}
}

func asInt(v any) int {
	switch n := v.(type) {
	case float64:
		return int(n)
	case int:
		return n
	default:
		panic(fmt.Sprintf("expected integer number, got %T (%v)", v, v))
	}
}

func materializeWireCase(c map[string]any) []byte {
	kind := c["kind"].(string)
	switch kind {
	case "raw":
		return []byte(c["raw"].(string))
	case "hex":
		b, err := hex.DecodeString(c["hex"].(string))
		if err != nil {
			panic(fmt.Sprintf("decode hex wire case: %v", err))
		}
		return b
	case "padded-object":
		total := asInt(c["bytes"])
		out := make([]byte, total)
		out[0] = '{'
		for i := 1; i < total-1; i++ {
			out[i] = ' '
		}
		out[total-1] = '}'
		return out
	case "nested-array":
		depth := asInt(c["depth"])
		return []byte(strings.Repeat("[", depth) + "0" + strings.Repeat("]", depth))
	case "number-array":
		count := asInt(c["count"])
		elems := make([]string, count)
		for i := range elems {
			elems[i] = "1e20"
		}
		return []byte("[" + strings.Join(elems, ",") + "]")
	default:
		panic("unknown wire case kind: " + kind)
	}
}

func applyJSONChanges(doc any, changes []any) {
	for _, chItem := range changes {
		ch := chItem.(map[string]any)
		path := ch["path"].([]any)
		val := deepCloneJSON(ch["value"])
		cur := doc
		for _, part := range path[:len(path)-1] {
			switch p := part.(type) {
			case string:
				cur = cur.(map[string]any)[p]
			case float64:
				cur = cur.([]any)[int(p)]
			default:
				panic(fmt.Sprintf("unsupported path segment: %T (%v)", part, part))
			}
		}
		leaf := path[len(path)-1]
		if leafStr, ok := leaf.(string); ok && leafStr == "-" {
			parentPath := path[:len(path)-1]
			if len(parentPath) == 0 {
				panic("cannot append '-' at document root")
			}
			// Re-walk to parent of target array so slice growth writes back cleanly.
			parent := doc
			for _, part := range parentPath[:len(parentPath)-1] {
				switch p := part.(type) {
				case string:
					parent = parent.(map[string]any)[p]
				case float64:
					parent = parent.([]any)[int(p)]
				}
			}
			arrSeg := parentPath[len(parentPath)-1]
			switch ap := arrSeg.(type) {
			case string:
				m := parent.(map[string]any)
				m[ap] = append(m[ap].([]any), val)
			case float64:
				a := parent.([]any)
				a[int(ap)] = append(a[int(ap)].([]any), val)
			}
		} else if leafStr, ok := leaf.(string); ok {
			cur.(map[string]any)[leafStr] = val
		} else if leafIdx, ok := leaf.(float64); ok {
			cur.([]any)[int(leafIdx)] = val
		} else {
			panic(fmt.Sprintf("unsupported leaf segment: %T (%v)", leaf, leaf))
		}
	}
}

func parseInputsMap(v any) map[string]any {
	out := make(map[string]any)
	if m, ok := v.(map[string]any); ok {
		for k, val := range m {
			out[k] = deepCloneJSON(val)
		}
	}
	return out
}

func assertEq(got, expected, msg string) {
	if got != expected {
		panic(fmt.Sprintf("%s: got %q, expected %q", msg, got, expected))
	}
}

func mustReadJSON(path string) any {
	raw, err := os.ReadFile(path)
	if err != nil {
		panic(fmt.Sprintf("read %s: %v", path, err))
	}
	var out any
	if err := json.Unmarshal(raw, &out); err != nil {
		panic(fmt.Sprintf("unmarshal %s: %v", path, err))
	}
	return out
}

func defaultTestRoot() string {
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		panic("runtime.Caller failed")
	}
	// evidence/internal/difftool/main.go -> repo root is 3 dirs up
	repoRoot := filepath.Clean(filepath.Join(filepath.Dir(file), "..", "..", ".."))
	return filepath.Join(repoRoot, "crates", "sysprims-evidence", "tests")
}

func main() {
	testRoot := defaultTestRoot()
	corpusDir := os.Getenv("EVIDENCE_CORPUS_DIR")
	if corpusDir == "" {
		corpusDir = filepath.Join(testRoot, "identity-evidence-v0")
	}
	parityPath := os.Getenv("EVIDENCE_PARITY_CASES")
	if parityPath == "" {
		parityPath = filepath.Join(testRoot, "parity-cases.json")
	}
	runAll(corpusDir, parityPath)
}

func runAll(corpusDir, parityPath string) {
	parity := mustReadJSON(parityPath).(map[string]any)

	// 1. Wire admission cases
	wireDoc := mustReadJSON(filepath.Join(corpusDir, "vectors", "wire-admission.json")).(map[string]any)
	wireCases := append([]any{}, wireDoc["cases"].([]any)...)
	wireCases = append(wireCases, parity["extra_wire_cases"].([]any)...)
	for _, item := range wireCases {
		c := item.(map[string]any)
		id := c["id"].(string)
		raw := materializeWireCase(c)
		expected := c["expected"].(string)
		admitted, err := evidence.AdmitReportBytes(raw)
		if err == nil {
			assertEq("accept", expected, "wire case "+id)
			if expCan, ok := c["canonical"].(string); ok {
				assertEq(string(admitted.CanonicalBytes), expCan, "wire canonical "+id)
			}
			emitLine(map[string]any{
				"suite":         "wire",
				"id":            id,
				"outcome":       "accept",
				"digest":        admitted.Digest,
				"canonical_len": len(admitted.CanonicalBytes),
			})
		} else {
			code, _ := evidence.RefusalCode(err)
			assertEq(code, expected, "wire refusal "+id)
			emitLine(map[string]any{
				"suite":   "wire",
				"id":      id,
				"outcome": code,
			})
		}
	}

	// 2. Example reports
	examplesByStem := make(map[string]any)
	reportInputs := make(map[string]any)
	entries, err := os.ReadDir(filepath.Join(corpusDir, "examples"))
	if err != nil {
		panic(err)
	}
	var examplePaths []string
	for _, ent := range entries {
		if !ent.IsDir() && strings.HasSuffix(ent.Name(), ".json") {
			examplePaths = append(examplePaths, filepath.Join(corpusDir, "examples", ent.Name()))
		}
	}
	sort.Strings(examplePaths)
	for _, p := range examplePaths {
		raw, err := os.ReadFile(p)
		if err != nil {
			panic(err)
		}
		admitted, err := evidence.AdmitReportBytes(raw)
		if err != nil {
			panic(fmt.Sprintf("admit example %s: %v", p, err))
		}
		stem := strings.TrimSuffix(filepath.Base(p), ".json")
		repID := admitted.Value.(map[string]any)["report_id"].(string)
		reportInputs[repID] = deepCloneJSON(admitted.Value)
		examplesByStem[stem] = deepCloneJSON(admitted.Value)
	}
	for _, p := range examplePaths {
		stem := strings.TrimSuffix(filepath.Base(p), ".json")
		raw, err := os.ReadFile(p)
		if err != nil {
			panic(err)
		}
		admitted, err := evidence.AdmitAndValidateReport(raw, reportInputs)
		if err != nil {
			panic(fmt.Sprintf("validate example %s: %v", p, err))
		}
		emitLine(map[string]any{
			"suite":   "example-report",
			"id":      stem,
			"outcome": "accept",
			"digest":  admitted.Digest,
		})
	}

	// 3. Two-producer import collision
	collision := mustReadJSON(filepath.Join(corpusDir, "fixtures", "two-producer-collision.json")).(map[string]any)
	colInputs := collision["inputs"].([]any)
	colErr := evidence.CheckImportCollisions(colInputs)
	if colErr == nil {
		panic("two-producer-collision must refuse")
	}
	colCode, _ := evidence.RefusalCode(colErr)
	assertEq(colCode, collision["expected"].(string), "two-producer-collision")
	emitLine(map[string]any{
		"suite":   "import-collision",
		"id":      "two-producer-collision",
		"outcome": colCode,
	})
	if err := evidence.CheckImportCollisions([]any{deepCloneJSON(colInputs[0]), deepCloneJSON(colInputs[0])}); err != nil {
		panic(fmt.Sprintf("identical duplicate report import refused: %v", err))
	}
	emitLine(map[string]any{
		"suite":   "import-collision",
		"id":      "identical-self-duplicate",
		"outcome": "accept",
	})

	// 4. Report shape, semantic & provenance DAG cases
	for _, item := range parity["report_shape_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		got := "accept"
		if err := evidence.ValidateReportShape(c["report"]); err != nil {
			got, _ = evidence.RefusalCode(err)
		}
		assertEq(got, expected, "report shape "+id)
		emitLine(map[string]any{"suite": "report-shape", "id": id, "outcome": got})
	}

	for _, item := range parity["report_semantic_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		if err := evidence.ValidateReportShape(c["report"]); err != nil {
			panic(fmt.Sprintf("semantic case %s failed shape: %v", id, err))
		}
		inputs := parseInputsMap(c["inputs"])
		got := "accept"
		if err := evidence.NecessaryConditions(c["report"], inputs); err != nil {
			got, _ = evidence.RefusalCode(err)
		}
		assertEq(got, expected, "report semantic "+id)
		emitLine(map[string]any{"suite": "report-semantic", "id": id, "outcome": got})
	}

	for _, item := range parity["provenance_dag_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		maxDepth := asInt(c["max_depth"])
		inputs := parseInputsMap(c["inputs"])
		got := "accept"
		if err := evidence.ValidateProvenanceDAG(c["report"], inputs, maxDepth); err != nil {
			got, _ = evidence.RefusalCode(err)
		}
		assertEq(got, expected, "provenance dag "+id)
		emitLine(map[string]any{"suite": "provenance-dag", "id": id, "outcome": got})
	}

	// 5. Local-process-session fit cases
	sessDir := filepath.Join(corpusDir, "profiles", "local-process-session")
	sessBundle := mustReadJSON(filepath.Join(sessDir, "evidence.example.json"))
	if err := evidence.ValidateSessionBundleShape(sessBundle); err != nil {
		panic(fmt.Sprintf("valid session bundle example failed shape: %v", err))
	}
	origCtx := mustReadJSON(filepath.Join(sessDir, "evaluation-context.example.json"))
	sessProf := evidence.ProfileRef{
		ID:       evidence.LocalProcessSessionProfileID,
		Revision: evidence.LocalProcessSessionProfileRevision,
	}
	sessCasesA := mustReadJSON(filepath.Join(sessDir, "cases.json")).([]any)
	sessCasesB := mustReadJSON(filepath.Join(sessDir, "report-cases.json")).([]any)
	allSessCases := append(append([]any{}, sessCasesA...), sessCasesB...)
	for _, item := range allSessCases {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		ctx := deepCloneJSON(origCtx)
		rep := deepCloneJSON(examplesByStem["local-process-session-launch"])
		origRep := deepCloneJSON(rep)
		var ctxChanges []any
		if ch, ok := c["change"]; ok && ch != nil {
			ctxChanges = append(ctxChanges, ch)
		}
		if cc, ok := c["context_changes"].([]any); ok {
			ctxChanges = append(ctxChanges, cc...)
		}
		applyJSONChanges(ctx, ctxChanges)
		if rc, ok := c["report_changes"].([]any); ok {
			applyJSONChanges(rep, rc)
		}
		if irc, ok := c["input_report_changes"].([]any); ok {
			applyJSONChanges(origRep, irc)
		}
		if err := evidence.ValidateReportShape(rep); err != nil {
			panic(fmt.Sprintf("session case %s report shape: %v", id, err))
		}
		inMap := map[string]any{
			origRep.(map[string]any)["report_id"].(string): origRep,
		}
		if err := evidence.NecessaryConditions(rep, inMap); err != nil {
			panic(fmt.Sprintf("session case %s necessary conditions: %v", id, err))
		}
		got := "accept"
		if err := evidence.ProcessSessionFit(rep, sessBundle, ctx, sessProf); err != nil {
			got, _ = evidence.RefusalCode(err)
		}
		assertEq(got, expected, "session fit "+id)
		emitLine(map[string]any{"suite": "session-fit", "id": id, "outcome": got})
	}

	for _, item := range parity["extra_session_fit_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		if err := evidence.ValidateSessionBundleShape(c["bundle"]); err != nil {
			panic(fmt.Sprintf("extra session fit %s bundle shape: %v", id, err))
		}
		got := "accept"
		if err := evidence.ProcessSessionFit(c["report"], c["bundle"], c["context"], sessProf); err != nil {
			got, _ = evidence.RefusalCode(err)
		}
		assertEq(got, expected, "extra session fit "+id)
		emitLine(map[string]any{"suite": "session-fit-extra", "id": id, "outcome": got})
	}

	// 6. Native-process-observation suites
	obsProf := evidence.ProfileRef{
		ID:       evidence.NativeProcessObservationProfileID,
		Revision: evidence.NativeProcessObservationProfileRevision,
	}

	for _, item := range parity["observation_shape_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		got := "accept"
		if err := evidence.ValidateObservationShape(c["obs"]); err != nil {
			got, _ = evidence.RefusalCode(err)
		}
		assertEq(got, expected, "obs shape "+id)
		emitLine(map[string]any{"suite": "obs-shape", "id": id, "outcome": got})
	}

	for _, item := range parity["observation_admit_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		var nowPtr *string
		if s, ok := c["now_utc"].(string); ok {
			nowPtr = &s
		}
		var repOpt any
		if r, ok := c["report"]; ok && r != nil {
			repOpt = r
		}
		inputs := parseInputsMap(c["inputs"])
		got := "accept"
		if err := evidence.AdmitLiveObservation(c["obs"], nowPtr, obsProf, repOpt, inputs); err != nil {
			got, _ = evidence.RefusalCode(err)
		}
		assertEq(got, expected, "obs admit "+id)
		emitLine(map[string]any{"suite": "obs-admit", "id": id, "outcome": got})
	}

	for _, item := range parity["observation_compare_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		got := evidence.CompareNativeObservationsJSON(c["baseline"], c["candidate"], c["policy"], obsProf).Code()
		assertEq(got, expected, "obs compare "+id)
		emitLine(map[string]any{"suite": "obs-compare", "id": id, "outcome": got})
	}

	for _, item := range parity["observation_projection_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		var repOpt any
		if r, ok := c["report"]; ok && r != nil {
			repOpt = r
		}
		inputs := parseInputsMap(c["inputs"])
		proj, err := evidence.ProjectToLocalProcessSessionProcessJSON(
			c["obs"],
			c["host_scope_trusted"],
			c["now_utc"],
			obsProf,
			repOpt,
			inputs,
			c["allowed_clone_behaviors"],
		)
		if err == nil {
			assertEq("accept", expected, "obs projection "+id)
			if !evidence.JSONEqual(proj, c["projected"]) {
				panic("projected payload mismatch on " + id)
			}
			digest, err := evidence.CanonicalSHA256Value(proj)
			if err != nil {
				panic(err)
			}
			emitLine(map[string]any{
				"suite":   "obs-projection",
				"id":      id,
				"outcome": "accept",
				"digest":  digest,
			})
		} else {
			code, _ := evidence.RefusalCode(err)
			assertEq(code, expected, "obs projection refusal "+id)
			emitLine(map[string]any{
				"suite":   "obs-projection",
				"id":      id,
				"outcome": code,
			})
		}
	}

	for _, item := range parity["observation_bind_cases"].([]any) {
		c := item.(map[string]any)
		id := c["id"].(string)
		expected := c["expected"].(string)
		got := "accept"
		if err := evidence.BindNativeObservationReport(c["report"], c["obs"], obsProf); err != nil {
			got, _ = evidence.RefusalCode(err)
		}
		assertEq(got, expected, "obs bind "+id)
		emitLine(map[string]any{"suite": "obs-bind", "id": id, "outcome": got})
	}

	// Golden preimages & overflow-mapping parity lines
	linuxObs := mustReadJSON(filepath.Join(corpusDir, "profiles", "native-process-observation", "evidence.linux.example.json")).(map[string]any)
	argvDig := evidence.DeriveArgumentsDigest([][]byte{[]byte("/usr/bin/sleep"), []byte("300")})
	exeDig := evidence.DeriveExecutableDigest([]byte("\x7fELF-synthetic-sleep-image-v0\n"))
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
	if err != nil {
		panic(err)
	}
	secDig, err := evidence.DeriveLinuxSecurityContextDigest(
		[]byte("observer_user_ns=userns:0:4:4026531837;groups=27,1000;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"),
	)
	if err != nil {
		panic(err)
	}
	hsURN, err := evidence.ProjectHostScopeURN(linuxObs["process"].(map[string]any)["host_scope"])
	if err != nil {
		panic(err)
	}
	idChecks := []bool{
		evidence.LinuxIDIsUnambiguouslyMapped(1000, []evidence.IDMapExtent{{First: 0, Count: math.MaxUint32}}, 65534),
		evidence.LinuxIDIsUnambiguouslyMapped(65534, []evidence.IDMapExtent{{First: 0, Count: math.MaxUint32}}, 65534),
		evidence.LinuxIDIsUnambiguouslyMapped(math.MaxUint32, []evidence.IDMapExtent{{First: 0, Count: math.MaxUint32}}, 65534),
		evidence.LinuxIDIsUnambiguouslyMapped(1000, []evidence.IDMapExtent{{First: 0, Count: 65536}}, 65534),
		evidence.LinuxIDIsUnambiguouslyMapped(65534, []evidence.IDMapExtent{{First: 0, Count: 65536}}, 65534),
		evidence.LinuxIDIsUnambiguouslyMapped(70000, []evidence.IDMapExtent{{First: 0, Count: 65536}}, 65534),
	}
	emitLine(map[string]any{
		"suite":                    "preimage-goldens",
		"id":                       "linux-goldens",
		"arguments_digest":         argvDig,
		"executable_digest":        exeDig,
		"namespace_context_digest": nsDig,
		"security_context_digest":  secDig,
		"host_scope_urn":           hsURN,
		"id_map_checks":            idChecks,
	})

	if extraPath := os.Getenv("EVIDENCE_EXTRA_OBSERVATION_CASES"); extraPath != "" {
		runExtraObservationPack(extraPath, obsProf)
	}
}

func runExtraObservationPack(path string, obsProf evidence.ProfileRef) {
	doc := mustReadJSON(path)
	var cases []any
	if arr, ok := doc.([]any); ok {
		cases = arr
	} else if m, ok := doc.(map[string]any); ok {
		if arr, ok := m["cases"].([]any); ok {
			cases = arr
		}
	}
	for _, item := range cases {
		c := item.(map[string]any)
		id, _ := c["id"].(string)
		if id == "" {
			id = "unnamed"
		}
		kind, _ := c["kind"].(string)
		if kind == "" {
			kind, _ = c["operation"].(string)
		}
		if kind == "" {
			kind = "compare"
		}
		switch kind {
		case "compare":
			expected := c["expected"].(string)
			got := evidence.CompareNativeObservationsJSON(c["baseline"], c["candidate"], c["policy"], obsProf).Code()
			assertEq(got, expected, "extra compare "+id)
			emitLine(map[string]any{"suite": "extra-obs", "id": id, "outcome": got})
		case "admit":
			expected := c["expected"].(string)
			var nowPtr *string
			if s, ok := c["now_utc"].(string); ok {
				nowPtr = &s
			} else if s, ok := c["now"].(string); ok {
				nowPtr = &s
			}
			var repOpt any
			if r, ok := c["report"]; ok && r != nil {
				repOpt = r
			}
			inRaw := c["inputs"]
			if inRaw == nil {
				inRaw = c["input_reports"]
			}
			inputs := parseInputsMap(inRaw)
			got := "accept"
			if err := evidence.AdmitLiveObservation(c["obs"], nowPtr, obsProf, repOpt, inputs); err != nil {
				got, _ = evidence.RefusalCode(err)
			}
			assertEq(got, expected, "extra admit "+id)
			emitLine(map[string]any{"suite": "extra-obs", "id": id, "outcome": got})
		case "project", "projection":
			expected := c["expected"].(string)
			nowVal, ok := c["now_utc"]
			if !ok {
				nowVal = c["now"]
			}
			var repOpt any
			if r, ok := c["report"]; ok && r != nil {
				repOpt = r
			}
			inRaw := c["inputs"]
			if inRaw == nil {
				inRaw = c["input_reports"]
			}
			inputs := parseInputsMap(inRaw)
			proj, err := evidence.ProjectToLocalProcessSessionProcessJSON(
				c["obs"],
				c["host_scope_trusted"],
				nowVal,
				obsProf,
				repOpt,
				inputs,
				c["allowed_clone_behaviors"],
			)
			if err == nil {
				assertEq("accept", expected, "extra projection "+id)
				if expProj, hasExp := c["projected"]; hasExp {
					if !evidence.JSONEqual(proj, expProj) {
						panic("extra projected mismatch on " + id)
					}
				}
				digest, err := evidence.CanonicalSHA256Value(proj)
				if err != nil {
					panic(err)
				}
				emitLine(map[string]any{
					"suite":   "extra-obs",
					"id":      id,
					"outcome": "accept",
					"digest":  digest,
				})
			} else {
				code, _ := evidence.RefusalCode(err)
				assertEq(code, expected, "extra projection refusal "+id)
				emitLine(map[string]any{
					"suite":   "extra-obs",
					"id":      id,
					"outcome": code,
				})
			}
		case "shape":
			expected := c["expected"].(string)
			got := "accept"
			if err := evidence.ValidateObservationShape(c["obs"]); err != nil {
				got, _ = evidence.RefusalCode(err)
			}
			assertEq(got, expected, "extra shape "+id)
			emitLine(map[string]any{"suite": "extra-obs", "id": id, "outcome": got})
		default:
			panic("unsupported extra case kind " + kind + " on " + id)
		}
	}
}
