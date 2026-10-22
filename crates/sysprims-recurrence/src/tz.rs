//! The pinned tzdb bundle. Zones load only from these embedded bytes; there is
//! no host lookup path, so `TZ`, `ZONEINFO` and system zoneinfo never matter.

use std::collections::HashMap;
use std::sync::OnceLock;

static BUNDLE_BYTES: &[u8] = include_bytes!("../tzdata/tzif-bundle-2026e.bin");

struct Bundle {
    version: &'static str,
    index: HashMap<&'static str, (usize, usize)>,
    #[allow(dead_code)] // read by zone loading
    data: &'static [u8],
}

fn bundle() -> &'static Bundle {
    static BUNDLE: OnceLock<Bundle> = OnceLock::new();
    BUNDLE.get_or_init(|| parse_bundle(BUNDLE_BYTES).expect("embedded tzdb bundle is malformed"))
}

fn parse_bundle(bytes: &'static [u8]) -> Option<Bundle> {
    let split = bytes.windows(2).position(|w| w == b"\n\n")?;
    let header = std::str::from_utf8(&bytes[..split]).ok()?;
    let data = &bytes[split + 2..];
    let mut lines = header.lines();
    if lines.next()? != "SYSPRIMS-TZIF-BUNDLE 1" {
        return None;
    }
    let version = lines.next()?.strip_prefix("tzdb ")?;
    let count: usize = lines.next()?.strip_prefix("zones ")?.parse().ok()?;
    let mut index = HashMap::with_capacity(count);
    for line in lines {
        let mut parts = line.split(' ');
        let (name, off, len) = (parts.next()?, parts.next()?, parts.next()?);
        let (off, len): (usize, usize) = (off.parse().ok()?, len.parse().ok()?);
        if parts.next().is_some() || off.checked_add(len)? > data.len() {
            return None;
        }
        index.insert(name, (off, len));
    }
    (index.len() == count).then_some(Bundle {
        version,
        index,
        data,
    })
}

/// The tzdb release this build expands with (from the embedded bundle header).
pub fn tzdb_version() -> &'static str {
    bundle().version
}

pub(crate) fn is_accepted(name: &str) -> bool {
    bundle().index.contains_key(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn crate_file(rel: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
    }

    #[test]
    fn bundle_matches_its_pin() {
        let pin = String::from_utf8(crate_file("tzdata/tzif-bundle-2026e.sha256")).unwrap();
        let want = pin.split_whitespace().next().unwrap();
        let got: String = Sha256::digest(BUNDLE_BYTES)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(got, want);
        assert_eq!(tzdb_version(), "2026e");
    }

    #[test]
    fn roster_equals_the_corpus_accepted_names() {
        let ev: serde_json::Value =
            serde_json::from_slice(&crate_file("tests/recurrence-v0/corpus/tzdb-evidence.json"))
                .unwrap();
        let mut want: Vec<&str> = ev["accepted_names"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        want.sort_unstable();
        let mut got: Vec<&str> = bundle().index.keys().copied().collect();
        got.sort_unstable();
        assert_eq!(got, want);
        assert!(is_accepted("Europe/Oslo"));
        for alias in ["US/Eastern", "Asia/Calcutta", "UTC", "europe/berlin"] {
            assert!(!is_accepted(alias), "{alias}");
        }
    }
}
