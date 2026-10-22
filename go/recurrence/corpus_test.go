package recurrence

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

// corpusDir locates the recurrence/v0 corpus. In this repository it is the
// copy vendored for the Rust crate; RECURRENCE_CORPUS_DIR points elsewhere
// (for example at a crucible checkout) when this module is verified on its
// own. A missing corpus fails the test; only `go test -short` skips it.
func corpusDir(t *testing.T) string {
	t.Helper()
	d := os.Getenv("RECURRENCE_CORPUS_DIR")
	if d == "" {
		d = filepath.Join("..", "..", "crates", "sysprims-recurrence", "tests", "recurrence-v0")
	}
	if _, err := os.Stat(filepath.Join(d, "corpus", "MANIFEST.json")); err != nil {
		if testing.Short() {
			t.Skip("recurrence/v0 corpus not found (-short)")
		}
		t.Fatalf("recurrence/v0 corpus not found at %s; set RECURRENCE_CORPUS_DIR", d)
	}
	return d
}

func readJSON(t *testing.T, path string) map[string]interface{} {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var v map[string]interface{}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	if err := dec.Decode(&v); err != nil {
		t.Fatalf("%s: %v", path, err)
	}
	return v
}

func listJSON(t *testing.T, dir string) []string {
	t.Helper()
	m, err := filepath.Glob(filepath.Join(dir, "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	sort.Strings(m)
	return m
}

// referenceJCS serializes decoded test JSON (string, json.Number, arrays,
// objects; the corpus uses no other kinds) independently of the package.
func referenceJCS(v interface{}) string {
	switch x := v.(type) {
	case map[string]interface{}:
		keys := make([]string, 0, len(x))
		for k := range x {
			keys = append(keys, k)
		}
		sort.Strings(keys) // corpus keys are ASCII, where UTF-16 and byte order agree
		parts := make([]string, len(keys))
		for i, k := range keys {
			kb, _ := json.Marshal(k)
			parts[i] = string(kb) + ":" + referenceJCS(x[k])
		}
		return "{" + strings.Join(parts, ",") + "}"
	case []interface{}:
		parts := make([]string, len(x))
		for i, e := range x {
			parts[i] = referenceJCS(e)
		}
		return "[" + strings.Join(parts, ",") + "]"
	case json.Number:
		return x.String()
	case nil:
		return "null"
	default:
		b, _ := json.Marshal(x)
		return string(b)
	}
}

func inputRule(v map[string]interface{}) (*Rule, error) {
	if rule, ok := v["rule"]; ok {
		raw, _ := json.Marshal(rule)
		return ParseJSON(raw)
	}
	var lines []string
	for _, l := range v["input_text"].([]interface{}) {
		lines = append(lines, l.(string))
	}
	return ParseText(lines)
}

func TestManifestPinsTheCorpus(t *testing.T) {
	dir := filepath.Join(corpusDir(t), "corpus")
	raw, err := os.ReadFile(filepath.Join(dir, "MANIFEST.json"))
	if err != nil {
		t.Fatal(err)
	}
	sum := sha256.Sum256(raw)
	if got := "sha256:" + hex.EncodeToString(sum[:]); got != CorpusDigest {
		t.Fatalf("corpus digest %s, want %s", got, CorpusDigest)
	}
	var m struct {
		TZDB  string            `json:"tzdb_version"`
		Files map[string]string `json:"files"`
	}
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatal(err)
	}
	if m.TZDB != TZDBVersion() {
		t.Fatalf("corpus tzdb %s, bundle %s", m.TZDB, TZDBVersion())
	}
	for rel, want := range m.Files {
		b, err := os.ReadFile(filepath.Join(dir, filepath.FromSlash(rel)))
		if err != nil {
			t.Fatal(err)
		}
		s := sha256.Sum256(b)
		if got := "sha256:" + hex.EncodeToString(s[:]); got != want {
			t.Errorf("%s: %s != manifest %s", rel, got, want)
		}
	}
	if n := len(listJSON(t, filepath.Join(dir, "vectors"))) + len(listJSON(t, filepath.Join(dir, "rejects"))) + 1; n != len(m.Files) {
		t.Fatalf("%d corpus files, manifest lists %d", n, len(m.Files))
	}
}

// TestCorpus runs every accept and reject vector. With
// RECURRENCE_NDJSON_OUT set it also writes the cross-language comparison
// stream, one line per vector in file-name order.
func TestCorpus(t *testing.T) {
	dir := filepath.Join(corpusDir(t), "corpus")
	var lines []string
	vectors := listJSON(t, filepath.Join(dir, "vectors"))
	rejects := listJSON(t, filepath.Join(dir, "rejects"))
	if len(vectors) != 34 || len(rejects) != 38 {
		t.Fatalf("expected 34 vectors and 38 rejects, found %d and %d", len(vectors), len(rejects))
	}
	for _, path := range vectors {
		name := strings.TrimSuffix(filepath.Base(path), ".json")
		v := readJSON(t, path)
		r, err := inputRule(v)
		if err != nil {
			t.Errorf("%s: refused: %v", name, err)
			continue
		}
		canonical := string(r.CanonicalJSON())
		if want := referenceJCS(v["canonical"]); canonical != want {
			t.Errorf("%s: canonical\n got %s\nwant %s", name, canonical, want)
		}
		if r.Digest() != v["digest"] {
			t.Errorf("%s: digest %s", name, r.Digest())
		}
		var wantText []string
		for _, l := range v["text"].([]interface{}) {
			wantText = append(wantText, l.(string))
		}
		if fmt.Sprint(r.Text()) != fmt.Sprint(wantText) {
			t.Errorf("%s: text %q", name, r.Text())
		}
		if back, err := ParseText(r.Text()); err != nil || back.Digest() != r.Digest() {
			t.Errorf("%s: text round trip: %v", name, err)
		}
		if back, err := ParseJSON(r.CanonicalJSON()); err != nil || back.Digest() != r.Digest() {
			t.Errorf("%s: JSON round trip: %v", name, err)
		}
		var results []interface{}
		for _, qv := range v["queries"].([]interface{}) {
			q := qv.(map[string]interface{})
			from, _ := ParseInstant(q["from"].(string))
			to, _ := ParseInstant(q["to"].(string))
			limit, _ := q["limit"].(json.Number).Int64()
			got := []interface{}{}
			res, err := r.Expand(from, to, int(limit))
			if err != nil {
				got = append(got, "error: "+err.Error())
			}
			for _, at := range res {
				got = append(got, FormatInstant(at))
			}
			if want := referenceJCS(q["expected"]); referenceJCS(got) != want {
				t.Errorf("%s: expand %s..%s limit %d\n got %s\nwant %s", name, q["from"], q["to"], limit, referenceJCS(got), want)
			}
			results = append(results, map[string]interface{}{
				"kind": "expand", "from": FormatInstant(from), "to": FormatInstant(to),
				"limit": json.Number(fmt.Sprint(limit)), "result": got,
			})
		}
		for _, nv := range v["next_after"].([]interface{}) {
			n := nv.(map[string]interface{})
			after, _ := ParseInstant(n["after"].(string))
			var got interface{}
			next, found, err := r.NextAfter(after)
			switch {
			case err != nil:
				got = "error: " + err.Error()
			case found:
				got = FormatInstant(next)
			}
			if referenceJCS(got) != referenceJCS(n["expected"]) {
				t.Errorf("%s: next_after %s: got %v want %v", name, n["after"], got, n["expected"])
			}
			results = append(results, map[string]interface{}{"kind": "next_after", "after": FormatInstant(after), "result": got})
		}
		if results == nil {
			results = []interface{}{}
		}
		var text []interface{}
		for _, l := range r.Text() {
			text = append(text, l)
		}
		lines = append(lines, referenceJCS(map[string]interface{}{
			"vector": name, "canonical": canonical, "digest": r.Digest(), "text": text, "results": results,
		}))
	}
	for _, path := range rejects {
		name := strings.TrimSuffix(filepath.Base(path), ".json")
		v := readJSON(t, path)
		want := v["reason"].(string)
		if !strings.HasPrefix(name, want+"--") {
			t.Fatalf("%s: file name and reason %s disagree", name, want)
		}
		got := "accepted"
		if _, err := inputRule(v); err != nil {
			got = string(ReasonOf(err))
		}
		if got != want {
			t.Errorf("%s: %s, want %s", name, got, want)
		}
		lines = append(lines, referenceJCS(map[string]interface{}{"vector": name, "reason": got}))
	}
	if out := os.Getenv("RECURRENCE_NDJSON_OUT"); out != "" {
		if err := os.WriteFile(out, []byte(strings.Join(lines, "\n")+"\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}

func TestExamplesAndDocuments(t *testing.T) {
	base := corpusDir(t)
	readme, err := os.ReadFile(filepath.Join(base, "README.md"))
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range listJSON(t, filepath.Join(base, "examples")) {
		raw, _ := os.ReadFile(path)
		r, err := ParseJSON(raw)
		if err != nil {
			t.Fatalf("%s: %v", path, err)
		}
		if !bytes.Contains(readme, []byte(r.Digest())) {
			t.Errorf("%s: digest %s not recorded in the README", filepath.Base(path), r.Digest())
		}
	}
	good, _ := os.ReadFile(filepath.Join(base, "documents", "first-thursday-monthly.json"))
	if _, err := ParseDocument(good); err != nil {
		t.Errorf("document: %v", err)
	}
	bad, _ := os.ReadFile(filepath.Join(base, "documents", "reject-missing-capability.json"))
	if _, err := ParseDocument(bad); err == nil {
		t.Error("document without the capability was accepted")
	}
}
