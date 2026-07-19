use serde_json::{Map, Value};

pub(super) fn flag(params: &Map<String, Value>, key: &str) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(super) fn literal_subset(expected: &Value, actual: &Value) -> bool {
    crate::matcher::subset_with_array_policy(expected, actual, crate::matcher::ArrayPolicy::Exact)
        .is_none()
}
