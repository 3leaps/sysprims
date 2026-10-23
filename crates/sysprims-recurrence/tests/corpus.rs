//! recurrence/v0 conformance corpus: every accept and reject vector.
//!
//! Set `RECURRENCE_NDJSON_OUT=<path>` to also write the cross-language
//! comparison stream (one line per vector, sorted by file name).

use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sysprims_recurrence::{Instant, RecurrenceRule, CORPUS_DIGEST};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/recurrence-v0")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    v.sort();
    v
}

fn load(p: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
}

/// Rebuilds a vector's JSON input exactly as written (duplicate-free per the corpus).
fn input_rule(v: &serde_json::Value) -> Result<RecurrenceRule, sysprims_recurrence::Refusal> {
    if let Some(rule) = v.get("rule") {
        RecurrenceRule::from_json(serde_json::to_string(rule).unwrap().as_bytes())
    } else {
        let lines: Vec<String> = v["input_text"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_owned())
            .collect();
        RecurrenceRule::from_text(&lines)
    }
}

/// RFC 8785 bytes of an expected `canonical` object, computed independently
/// of the crate (keys sorted, integers and ASCII strings only in this corpus).
fn jcs(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let body: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), jcs(&m[k])))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        serde_json::Value::Array(a) => {
            format!("[{}]", a.iter().map(jcs).collect::<Vec<_>>().join(","))
        }
        other => serde_json::to_string(other).unwrap(),
    }
}

#[test]
fn manifest_pins_every_corpus_file() {
    let corpus = root().join("corpus");
    let manifest_bytes = fs::read(corpus.join("MANIFEST.json")).unwrap();
    assert_eq!(
        format!("sha256:{}", sha256_hex(&manifest_bytes)),
        CORPUS_DIGEST
    );
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(
        manifest["tzdb_version"],
        sysprims_recurrence::tzdb_version()
    );
    let files = manifest["files"].as_object().unwrap();
    for (rel, want) in files {
        let got = format!(
            "sha256:{}",
            sha256_hex(&fs::read(corpus.join(rel)).unwrap())
        );
        assert_eq!(&got, want.as_str().unwrap(), "{rel}");
    }
    let on_disk =
        json_files(&corpus.join("vectors")).len() + json_files(&corpus.join("rejects")).len() + 1;
    assert_eq!(files.len(), on_disk, "corpus files not listed in MANIFEST");
}

#[test]
fn accept_vectors() {
    let mut lines = Vec::new();
    let mut failures = Vec::new();
    let vectors = json_files(&root().join("corpus/vectors"));
    assert!(!vectors.is_empty());
    for path in &vectors {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let v = load(path);
        let mut fail = |what: String| failures.push(format!("{name}: {what}"));
        let rule = match input_rule(&v) {
            Ok(r) => r,
            Err(e) => {
                fail(format!("refused: {e}"));
                continue;
            }
        };
        let canonical = String::from_utf8(rule.to_canonical_json()).unwrap();
        if canonical != jcs(&v["canonical"]) {
            fail(format!("canonical {canonical}"));
        }
        if rule.digest() != v["digest"].as_str().unwrap() {
            fail(format!("digest {}", rule.digest()));
        }
        let text = rule.to_text();
        let want_text: Vec<&str> = v["text"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap())
            .collect();
        if text != want_text {
            fail(format!("text {text:?}"));
        }
        // Text and canonical JSON both round-trip to the same rule.
        match RecurrenceRule::from_text(&text) {
            Ok(r) if r == rule => {}
            other => fail(format!("text round trip {other:?}")),
        }
        match RecurrenceRule::from_json(&rule.to_canonical_json()) {
            Ok(r) if r == rule => {}
            other => fail(format!("json round trip {other:?}")),
        }
        let mut results = Vec::new();
        for q in v["queries"].as_array().unwrap() {
            let from = Instant::parse(q["from"].as_str().unwrap()).unwrap();
            let to = Instant::parse(q["to"].as_str().unwrap()).unwrap();
            let limit = NonZeroU32::new(q["limit"].as_u64().unwrap() as u32).unwrap();
            let got: Vec<String> = match rule.expand(from, to, limit) {
                Ok(v) => v.iter().map(Instant::to_string).collect(),
                Err(e) => vec![format!("error: {e:?}")],
            };
            let want: Vec<&str> = q["expected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect();
            if got != want {
                fail(format!(
                    "expand {from}..{to} limit {limit}: {got:?} != {want:?}"
                ));
            }
            results.push(serde_json::json!({"kind": "expand", "from": from.to_string(), "to": to.to_string(), "limit": limit.get(), "result": got}));
        }
        for na in v["next_after"].as_array().unwrap() {
            let after = Instant::parse(na["after"].as_str().unwrap()).unwrap();
            let got = match rule.next_after(after) {
                Ok(Some(i)) => serde_json::Value::String(i.to_string()),
                Ok(None) => serde_json::Value::Null,
                Err(e) => serde_json::Value::String(format!("error: {e:?}")),
            };
            if got != na["expected"] {
                fail(format!("next_after {after}: {got} != {}", na["expected"]));
            }
            results.push(serde_json::json!({"kind": "next_after", "after": after.to_string(), "result": got}));
        }
        lines.push(serde_json::json!({
            "vector": name,
            "canonical": canonical,
            "digest": rule.digest(),
            "text": text,
            "results": results,
        }));
    }
    for path in json_files(&root().join("corpus/rejects")) {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let v = load(&path);
        let reason = match input_rule(&v) {
            Ok(_) => "accepted".to_owned(),
            Err(e) => e.reason().as_str().to_owned(),
        };
        lines.push(serde_json::json!({"vector": name, "reason": reason}));
    }
    if let Ok(out) = std::env::var("RECURRENCE_NDJSON_OUT") {
        let body: String = lines.iter().map(|l| format!("{}\n", jcs(l))).collect();
        fs::write(out, body).unwrap();
    }
    assert!(
        failures.is_empty(),
        "{} vector failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(vectors.len(), 34);
}

#[test]
fn reject_vectors() {
    let mut failures = Vec::new();
    let rejects = json_files(&root().join("corpus/rejects"));
    for path in &rejects {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let v = load(path);
        let want = v["reason"].as_str().unwrap();
        assert_eq!(
            name.split("--").next().unwrap(),
            want,
            "{name}: file name and reason disagree"
        );
        match input_rule(&v) {
            Ok(r) => failures.push(format!(
                "{name}: accepted as {}",
                String::from_utf8_lossy(&r.to_canonical_json())
            )),
            Err(e) if e.reason().as_str() != want => {
                failures.push(format!("{name}: {} != {want}", e.reason()))
            }
            Err(_) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} reject failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(rejects.len(), 38);
}

#[test]
fn examples_and_documents() {
    let base = root();
    let readme = fs::read_to_string(base.join("README.md")).unwrap();
    for path in json_files(&base.join("examples")) {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(&path).unwrap();
        let rule = RecurrenceRule::from_json(&bytes).unwrap_or_else(|e| panic!("{file}: {e}"));
        let row = readme
            .lines()
            .find(|l| l.contains(&format!("`{file}`")))
            .unwrap();
        assert!(
            row.contains(&rule.digest()),
            "{file}: {} not in README row",
            rule.digest()
        );
        // Examples are already in normal form.
        assert_eq!(
            String::from_utf8(rule.to_canonical_json()).unwrap(),
            jcs(&serde_json::from_slice(&bytes).unwrap()),
            "{file}"
        );
    }
    let ok = fs::read(base.join("documents/first-thursday-monthly.json")).unwrap();
    let doc_rule = RecurrenceRule::from_document_json(&ok).unwrap();
    let inner =
        serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(&ok).unwrap()["rule"])
            .unwrap();
    assert_eq!(
        doc_rule.digest(),
        RecurrenceRule::from_json(&inner).unwrap().digest(),
        "envelope never changes the digest"
    );
    let bad = fs::read(base.join("documents/reject-missing-capability.json")).unwrap();
    assert!(RecurrenceRule::from_document_json(&bad).is_err());
}
