//! Cross-cutting coercion tests — every JSON shape, both directions.

use iii_database::value::{JsonParam, RowValue};
use serde_json::json;

#[test]
fn from_json_slice_happy_path() {
    // exercises the slice helper's happy path; the InvalidParam branch is hard
    // to trigger from JSON in serde_json 1.x and is left for direct unit testing.
    let values = vec![json!(1), json!("ok"), json!(null)];
    let out = JsonParam::from_json_slice(&values).unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(out[0], JsonParam::Int(1));
    assert_eq!(out[1], JsonParam::Text("ok".into()));
    assert_eq!(out[2], JsonParam::Null);
}

#[test]
fn row_value_round_trip_text() {
    assert_eq!(RowValue::Text("hi".into()).to_json(), json!("hi"));
}

#[test]
fn row_value_float_nan_becomes_null() {
    // serde_json::Number cannot represent NaN; we surface it as JSON null
    // rather than failing.
    let v = RowValue::Float(f64::NAN);
    assert_eq!(v.to_json(), json!(null));
}
