//! JSON ↔ SQL value coercion shared across drivers.
//!
//! `JsonParam` is the driver-agnostic representation of a parameter sent in
//! by a caller. Each driver translates `JsonParam` to its native bind type.
//!
//! `RowValue` is the driver-agnostic representation of a returned cell, which
//! `to_json` flattens back to `serde_json::Value` for transport.

use crate::error::DbError;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Driver-agnostic input parameter. Each driver translates this to its
/// native bind type.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonParam {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Json(Value),
}

impl JsonParam {
    pub fn from_json(v: &Value) -> Result<Self, DbError> {
        Ok(match v {
            Value::Null => JsonParam::Null,
            Value::Bool(b) => JsonParam::Bool(*b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    JsonParam::Int(i)
                } else if let Some(f) = n.as_f64() {
                    JsonParam::Float(f)
                } else {
                    return Err(DbError::InvalidParam {
                        index: 0,
                        reason: format!("number {n} not representable as i64 or f64"),
                    });
                }
            }
            Value::String(s) => JsonParam::Text(s.clone()),
            Value::Array(_) | Value::Object(_) => JsonParam::Json(v.clone()),
        })
    }

    /// Convenience: coerce a slice of JSON values, tagging each error with its index.
    pub fn from_json_slice(values: &[Value]) -> Result<Vec<JsonParam>, DbError> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Self::from_json(v).map_err(|e| match e {
                    DbError::InvalidParam { reason, .. } => {
                        DbError::InvalidParam { index: i, reason }
                    }
                    other => other,
                })
            })
            .collect()
    }
}

/// Driver-agnostic returned cell. Each driver maps its row types into this
/// enum; `to_json` flattens it for transport.
#[derive(Debug, Clone, PartialEq)]
pub enum RowValue {
    Null,
    Bool(bool),
    Int(i64),
    /// 64-bit identities. Serialized as JSON string to preserve precision in JS.
    BigInt(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Timestamp(DateTime<Utc>),
    /// Numeric / decimal values preserved as string.
    Decimal(String),
    /// JSON / JSONB columns surfaced as a JSON value.
    Json(Value),
}

impl RowValue {
    pub fn to_json(&self) -> Value {
        match self {
            RowValue::Null => Value::Null,
            RowValue::Bool(b) => Value::Bool(*b),
            RowValue::Int(i) => Value::from(*i),
            RowValue::BigInt(i) => Value::String(i.to_string()),
            RowValue::Float(f) => serde_json::Number::from_f64(*f)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            RowValue::Text(s) => Value::String(s.clone()),
            RowValue::Bytes(b) => Value::String(B64.encode(b)),
            RowValue::Timestamp(t) => {
                Value::String(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            }
            RowValue::Decimal(s) => Value::String(s.clone()),
            RowValue::Json(v) => v.clone(),
        }
    }

    /// Consuming variant of `to_json` that moves heap-allocated payloads
    /// (`Text`, `Decimal`, `Json`) instead of cloning. On row-heavy SELECTs
    /// this eliminates one allocation per text/json cell.
    pub fn into_json(self) -> Value {
        match self {
            RowValue::Null => Value::Null,
            RowValue::Bool(b) => Value::Bool(b),
            RowValue::Int(i) => Value::from(i),
            RowValue::BigInt(i) => Value::String(i.to_string()),
            RowValue::Float(f) => serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            RowValue::Text(s) => Value::String(s),
            RowValue::Bytes(b) => Value::String(B64.encode(&b)),
            RowValue::Timestamp(t) => {
                Value::String(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            }
            RowValue::Decimal(s) => Value::String(s),
            RowValue::Json(v) => v,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_null_becomes_null_param() {
        assert_eq!(JsonParam::from_json(&json!(null)).unwrap(), JsonParam::Null);
    }

    #[test]
    fn json_int_becomes_int_param() {
        assert_eq!(
            JsonParam::from_json(&json!(42)).unwrap(),
            JsonParam::Int(42)
        );
    }

    #[test]
    fn json_negative_int_preserves_sign() {
        assert_eq!(
            JsonParam::from_json(&json!(-7)).unwrap(),
            JsonParam::Int(-7)
        );
    }

    #[test]
    fn json_float_becomes_float_param() {
        // 2.5 chosen as a clean fraction that avoids clippy::approx_constant
        // (which flags numbers like 3.14 / 2.71 as imprecise math constants).
        match JsonParam::from_json(&json!(2.5)).unwrap() {
            JsonParam::Float(f) => assert!((f - 2.5).abs() < 1e-9),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn json_bool_becomes_bool_param() {
        assert_eq!(
            JsonParam::from_json(&json!(true)).unwrap(),
            JsonParam::Bool(true)
        );
        assert_eq!(
            JsonParam::from_json(&json!(false)).unwrap(),
            JsonParam::Bool(false)
        );
    }

    #[test]
    fn json_string_becomes_text_param() {
        assert_eq!(
            JsonParam::from_json(&json!("hello")).unwrap(),
            JsonParam::Text("hello".into())
        );
    }

    #[test]
    fn json_object_becomes_json_param() {
        let v = json!({"a": 1});
        match JsonParam::from_json(&v).unwrap() {
            JsonParam::Json(inner) => assert_eq!(inner, v),
            other => panic!("expected Json, got {other:?}"),
        }
    }

    #[test]
    fn json_array_becomes_json_param() {
        let v = json!([1, 2, 3]);
        match JsonParam::from_json(&v).unwrap() {
            JsonParam::Json(inner) => assert_eq!(inner, v),
            other => panic!("expected Json, got {other:?}"),
        }
    }

    #[test]
    fn row_value_int_to_json() {
        assert_eq!(RowValue::Int(42).to_json(), json!(42));
    }

    #[test]
    fn row_value_bigint_to_json_is_string() {
        // BIGINT identities serialize as string to preserve precision in JS clients.
        assert_eq!(
            RowValue::BigInt(9_007_199_254_740_993).to_json(),
            json!("9007199254740993")
        );
    }

    #[test]
    fn row_value_bytes_to_json_is_base64() {
        let v = RowValue::Bytes(vec![0xff, 0x00, 0x10]);
        assert_eq!(v.to_json(), json!("/wAQ"));
    }

    #[test]
    fn row_value_bytes_base64_includes_padding() {
        // 1 byte → 2 base64 chars + "==" padding
        assert_eq!(RowValue::Bytes(vec![0xff]).to_json(), json!("/w=="));
        // 2 bytes → 3 base64 chars + "=" padding
        assert_eq!(RowValue::Bytes(vec![0xff, 0x00]).to_json(), json!("/wA="));
        // 3 bytes → 4 base64 chars, no padding (already covered by row_value_bytes_to_json_is_base64)
    }

    #[test]
    fn row_value_decimal_to_json_is_string() {
        let v = RowValue::Decimal("123.456000".into());
        assert_eq!(v.to_json(), json!("123.456000"));
    }

    #[test]
    fn row_value_timestamp_to_json_is_iso8601() {
        use chrono::{TimeZone, Utc};
        let ts = Utc.with_ymd_and_hms(2026, 4, 29, 12, 0, 0).unwrap();
        let v = RowValue::Timestamp(ts);
        assert_eq!(v.to_json(), json!("2026-04-29T12:00:00Z"));
    }

    #[test]
    fn row_value_json_passes_through() {
        let v = RowValue::Json(json!({"k": "v"}));
        assert_eq!(v.to_json(), json!({"k": "v"}));
    }

    #[test]
    fn row_value_null_to_json() {
        assert_eq!(RowValue::Null.to_json(), json!(null));
    }

    #[test]
    fn row_value_into_json_text_moves_string() {
        // Smoke: same value as to_json but the consuming variant.
        assert_eq!(
            RowValue::Text("hello".into()).into_json(),
            json!("hello")
        );
    }

    #[test]
    fn row_value_into_json_json_moves_value() {
        let inner = json!({"k": [1, 2, 3]});
        assert_eq!(RowValue::Json(inner.clone()).into_json(), inner);
    }

    #[test]
    fn row_value_into_json_decimal_moves_string() {
        assert_eq!(
            RowValue::Decimal("123.456".into()).into_json(),
            json!("123.456")
        );
    }

    #[test]
    fn row_value_into_json_matches_to_json_across_variants() {
        // Equivalence sweep: `into_json` must produce the exact same JSON as
        // `to_json` for every variant; the only difference is allocation.
        use chrono::{TimeZone, Utc};
        let cases: Vec<RowValue> = vec![
            RowValue::Null,
            RowValue::Bool(true),
            RowValue::Int(-7),
            RowValue::BigInt(9_007_199_254_740_993),
            RowValue::Float(2.5),
            RowValue::Text("x".into()),
            RowValue::Bytes(vec![0xff, 0x00]),
            RowValue::Timestamp(Utc.with_ymd_and_hms(2026, 4, 29, 12, 0, 0).unwrap()),
            RowValue::Decimal("1.0".into()),
            RowValue::Json(json!([1, "two", null])),
        ];
        for v in cases {
            assert_eq!(v.clone().into_json(), v.to_json());
        }
    }
}
