//! Strict JSON decoding (duplicate member names refused) and RFC 8785 output.

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::collections::BTreeMap;
use std::fmt;

/// A decoded JSON value. Objects keep keys sorted; duplicates never reach here.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    /// An integer that fits i64.
    Int(i64),
    /// Any other number (fractional, exponent, or out of i64 range).
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Int(v))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(i64::try_from(v).map_or(Value::Float(v as f64), Value::Int))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Float(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::Str(v.to_owned()))
    }
    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::Str(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element::<Value>()? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if out.contains_key(&key) {
                return Err(de::Error::custom("duplicate member name"));
            }
            let value = map.next_value::<Value>()?;
            out.insert(key, value);
        }
        Ok(Value::Object(out))
    }
}

impl Value {
    /// JSON Schema `integer`: any number with a zero fractional part.
    pub(crate) fn as_integer(&self) -> Option<i64> {
        match *self {
            Value::Int(n) => Some(n),
            Value::Float(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 => {
                Some(f as i64)
            }
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}

/// Parses one JSON document; `None` on malformed input or duplicate member names.
pub(crate) fn parse(bytes: &[u8]) -> Option<Value> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let v = Value::deserialize(&mut de).ok()?;
    de.end().ok()?;
    Some(v)
}

/// Output model for canonical serialization: only what a rule contains.
pub(crate) enum Out {
    Int(i64),
    Str(String),
    Array(Vec<Out>),
    /// Keys are serialized in RFC 8785 order (UTF-16 code units); all ours are ASCII.
    Object(Vec<(&'static str, Out)>),
}

pub(crate) fn write_jcs(v: &Out, buf: &mut String) {
    match v {
        Out::Int(n) => buf.push_str(&n.to_string()),
        Out::Str(s) => write_string(s, buf),
        Out::Array(items) => {
            buf.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                write_jcs(item, buf);
            }
            buf.push(']');
        }
        Out::Object(members) => {
            let mut sorted: Vec<&(&'static str, Out)> = members.iter().collect();
            sorted.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            buf.push('{');
            for (i, (k, val)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                write_string(k, buf);
                buf.push(':');
                write_jcs(val, buf);
            }
            buf.push('}');
        }
    }
}

/// RFC 8785 string serialization (ECMAScript JSON.stringify rules).
fn write_string(s: &str, buf: &mut String) {
    buf.push('"');
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\u{08}' => buf.push_str("\\b"),
            '\u{0C}' => buf.push_str("\\f"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if (c as u32) < 0x20 => buf.push_str(&format!("\\u{:04x}", c as u32)),
            c => buf.push(c),
        }
    }
    buf.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_members_are_refused_at_any_depth() {
        assert!(parse(br#"{"a":1,"a":1}"#).is_none());
        assert!(parse(br#"{"a":{"b":1,"b":2}}"#).is_none());
        assert!(parse(br#"[{"x":1},{"x":1}]"#).is_some());
        assert!(parse(br#"{"a":1} x"#).is_none());
    }

    #[test]
    fn numbers_keep_integer_identity() {
        assert_eq!(parse(b"5").unwrap(), Value::Int(5));
        assert_eq!(parse(b"5.0").unwrap(), Value::Float(5.0));
        assert_eq!(parse(b"1e2").unwrap(), Value::Float(100.0));
        assert!(matches!(
            parse(b"99999999999999999999").unwrap(),
            Value::Float(_)
        ));
    }

    #[test]
    fn jcs_sorts_keys_and_escapes_controls() {
        let mut s = String::new();
        write_jcs(
            &Out::Object(vec![
                ("b", Out::Int(-1)),
                ("a", Out::Str("x\u{1}\"\\".into())),
            ]),
            &mut s,
        );
        assert_eq!(s, r#"{"a":"x\u0001\"\\","b":-1}"#);
    }
}
