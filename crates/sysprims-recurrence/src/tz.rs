//! The pinned tzdb bundle. Zones load only from these embedded bytes; there is
//! no host lookup path, so `TZ`, `ZONEINFO` and system zoneinfo never matter.

use jiff::tz::{AmbiguousOffset, TimeZone};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::civil::CivilDateTime;

static BUNDLE_BYTES: &[u8] = include_bytes!("../tzdata/tzif-bundle-2026e.bin");

struct Bundle {
    version: &'static str,
    index: HashMap<&'static str, (usize, usize)>,
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

/// Loads a roster zone from the bundle. `None` for names outside the roster.
pub(crate) fn zone(name: &str) -> Option<TimeZone> {
    static CACHE: OnceLock<Mutex<HashMap<String, TimeZone>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(tz) = cache.lock().ok()?.get(name) {
        return Some(tz.clone());
    }
    let b = bundle();
    let &(off, len) = b.index.get(name)?;
    let tz = TimeZone::tzif(name, &b.data[off..off + len]).ok()?;
    cache.lock().ok()?.insert(name.to_owned(), tz.clone());
    Some(tz)
}

/// Maps a local label to UTC seconds.
///
/// Generated candidates (`explicit == false`) in a gap do not exist and give
/// `None`. Explicit values in a gap use the offset before the gap (RFC 5545
/// section 3.3.5). An overlap always selects the earlier instant.
pub(crate) fn resolve(tz: &TimeZone, label: CivilDateTime, explicit: bool) -> Option<i64> {
    let dt = jiff::civil::DateTime::new(
        i16::try_from(label.year()).ok()?,
        label.month() as i8,
        label.day() as i8,
        label.hour() as i8,
        label.minute() as i8,
        label.second() as i8,
        0,
    )
    .ok()?;
    let local = label.label_seconds();
    let offset = match tz.to_ambiguous_timestamp(dt).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset,
        AmbiguousOffset::Gap { before, .. } if explicit => before,
        AmbiguousOffset::Gap { .. } => return None,
        AmbiguousOffset::Fold { before, after } => before.max(after),
    };
    Some(local - i64::from(offset.seconds()))
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
        for name in got {
            assert!(zone(name).is_some(), "{name} does not load");
        }
    }

    /// Every recorded transition in the crucible evidence holds in the bundle:
    /// the new offset at the transition instant and the prior offset one
    /// second before it.
    #[test]
    fn bundle_agrees_with_the_tzdb_evidence() {
        let ev: serde_json::Value =
            serde_json::from_slice(&crate_file("tests/recurrence-v0/corpus/tzdb-evidence.json"))
                .unwrap();
        let parse_offset = |s: &str| -> i32 {
            let sign = if s.starts_with('-') { -1 } else { 1 };
            sign * (s[1..3].parse::<i32>().unwrap() * 3600 + s[4..6].parse::<i32>().unwrap() * 60)
        };
        let window_from =
            crate::civil::Instant::parse(ev["window"]["from"].as_str().unwrap()).unwrap();
        let offset_at = |tz: &TimeZone, secs: i64| -> i32 {
            tz.to_offset(jiff::Timestamp::from_second(secs).unwrap())
                .seconds()
        };
        let mut checked = 0;
        let zones = ev["zones"].as_object().unwrap();
        let links = ev["links"].as_object().unwrap();
        let named = zones.iter().map(|(n, z)| (n.as_str(), z)).chain(
            links
                .iter()
                .map(|(n, l)| (n.as_str(), &zones[l["target"].as_str().unwrap()])),
        );
        for (name, z) in named {
            let tz = zone(name).unwrap_or_else(|| panic!("{name} missing"));
            let mut prior = parse_offset(z["initial_offset"].as_str().unwrap());
            assert_eq!(
                offset_at(&tz, window_from.unix_seconds()),
                prior,
                "{name} initial"
            );
            for t in z["transitions"].as_array().unwrap() {
                let at = crate::civil::Instant::parse(t["at"].as_str().unwrap())
                    .unwrap()
                    .unix_seconds();
                let off = parse_offset(t["offset"].as_str().unwrap());
                assert_eq!(offset_at(&tz, at - 1), prior, "{name} before {at}");
                assert_eq!(offset_at(&tz, at), off, "{name} at {at}");
                prior = off;
                checked += 1;
            }
        }
        assert!(checked > 40, "only {checked} transitions checked");
    }

    #[test]
    fn host_time_zone_settings_do_not_matter() {
        // No host lookup is compiled in; this documents the property.
        std::env::set_var("TZ", "Asia/Tokyo");
        std::env::set_var("ZONEINFO", "/nonexistent");
        let tz = zone("America/New_York").unwrap();
        let label = CivilDateTime::new(2026, 7, 1, 12, 0, 0).unwrap();
        assert_eq!(
            resolve(&tz, label, false),
            Some(label.label_seconds() + 4 * 3600)
        );
    }

    #[test]
    fn gaps_and_folds() {
        let ny = zone("America/New_York").unwrap();
        let gap = CivilDateTime::new(2026, 3, 8, 2, 30, 0).unwrap();
        assert_eq!(resolve(&ny, gap, false), None);
        assert_eq!(
            resolve(&ny, gap, true),
            Some(gap.label_seconds() + 5 * 3600)
        );
        let fold = CivilDateTime::new(2026, 11, 1, 1, 30, 0).unwrap();
        assert_eq!(
            resolve(&ny, fold, false),
            Some(fold.label_seconds() + 4 * 3600)
        );
    }
}
