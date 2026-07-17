//! `fp::*` — pure lodash-style value transforms
//! (<https://lodash.com/docs/4.18.1>). Registered for discovery and direct
//! calls; as `fp::pipe` steps they execute inline in this worker (no bus
//! round-trip).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const GET_ID: &str = "fp::get";
pub const GET_DESC: &str =
    "Extract the single value at a JSON pointer (lodash _.get): { value, path: \"/content\" }. \
     A miss names the keys that were available. Mainly useful as a fp::pipe step.";

pub const PICK_ID: &str = "fp::pick";
pub const PICK_DESC: &str =
    "Subset an object by top-level keys (lodash _.pick): { value, paths: [\"a\",\"b\"] } -> \
     {a, b}; missing keys are omitted. Mainly useful as a fp::pipe step.";

pub const OMIT_ID: &str = "fp::omit";
pub const OMIT_DESC: &str =
    "Drop top-level keys from an object (lodash _.omit): { value, paths: [\"a\",\"b\"] }. \
     Mainly useful as a fp::pipe step.";

pub const TAKE_ID: &str = "fp::take";
pub const TAKE_DESC: &str =
    "Keep the first n elements of an array or the first n characters of a string (lodash \
     _.take): { value, n }. Mainly useful as a fp::pipe step.";

pub const DROP_ID: &str = "fp::drop";
pub const DROP_DESC: &str =
    "Skip the first n elements of an array or the first n characters of a string (lodash \
     _.drop): { value, n }. Mainly useful as a fp::pipe step.";

pub const MAP_ID: &str = "fp::map";
pub const MAP_DESC: &str =
    "Pluck the value at a JSON pointer from each array element (lodash _.map with a property \
     iteratee): { value, path: \"/id\" }. Sporadic misses become null (fp::compact drops \
     them), but a path matching NO element errors — pointers pluck stored fields, not \
     computed properties like /length (fp::size counts). Mainly useful as a fp::pipe step.";

pub const FILTER_ID: &str = "fp::filter";
pub const FILTER_DESC: &str =
    "Keep array elements whose properties equal a partial object (lodash _.filter with a \
     matches iteratee): { value, matches: { status: \"active\" } }. matches is a partial \
     OBJECT (never a bare string/number) and only object elements can match. Mainly useful \
     as a fp::pipe step.";

pub const SPLIT_ID: &str = "fp::split";
pub const SPLIT_DESC: &str =
    "Split a string into an array of strings (lodash _.split): { value, separator: \"\\n\" }. \
     Mainly useful as a fp::pipe step.";

pub const JOIN_ID: &str = "fp::join";
pub const JOIN_DESC: &str =
    "Join array elements into one string (lodash _.join): { value, separator } (separator \
     defaults to \",\"). Mainly useful as a fp::pipe step.";

pub const UNIQ_ID: &str = "fp::uniq";
pub const UNIQ_DESC: &str =
    "Deduplicate an array, keeping first occurrences (lodash _.uniq): { value }. Mainly useful \
     as a fp::pipe step.";

pub const SIZE_ID: &str = "fp::size";
pub const SIZE_DESC: &str =
    "Count a collection (lodash _.size): array length, string chars, or object key count: \
     { value }. Non-collections error instead of returning 0. Mainly useful as a fp::pipe step.";

pub const COMPACT_ID: &str = "fp::compact";
pub const COMPACT_DESC: &str =
    "Remove null elements from an array (lodash _.compact, null-only: 0, false and \"\" are \
     KEPT so plucked data survives): { value }. Pairs with fp::map, whose sporadic misses \
     become null. Mainly useful as a fp::pipe step.";

pub const NTH_ID: &str = "fp::nth";
pub const NTH_DESC: &str =
    "Element at index n; negative n counts from the end (lodash _.nth): { value, n: -1 } is \
     the last element. Out of bounds errors naming the length. Mainly useful as a fp::pipe \
     step.";

pub const GET_OR_ID: &str = "fp::getOr";
pub const GET_OR_DESC: &str =
    "Extract the value at a JSON pointer, or `default` when the path misses (lodash fp.getOr): \
     { value, path, default }. A stored null is a hit, not a miss. Mainly useful as a fp::pipe \
     step.";

pub const FLATTEN_ID: &str = "fp::flatten";
pub const FLATTEN_DESC: &str =
    "Flatten an array one level (lodash _.flatten): [1,[2],[[3]]] -> [1,2,[3]]. Mainly useful \
     as a fp::pipe step.";

pub const SORT_BY_ID: &str = "fp::sortBy";
pub const SORT_BY_DESC: &str =
    "Sort array elements ascending by the value at a JSON pointer (lodash _.sortBy, stable): \
     { value, path } — path \"\" sorts the elements themselves (primitives). Keys must all be \
     numbers, strings, or booleans; an element missing the path errors. Pair with fp::reverse \
     for descending. Mainly useful as a fp::pipe step.";

pub const REVERSE_ID: &str = "fp::reverse";
pub const REVERSE_DESC: &str =
    "Reverse an array (lodash _.reverse, immutable like lodash/fp): { value }. Mainly useful \
     as a fp::pipe step.";

pub const WHEN_ID: &str = "fp::when";
pub const WHEN_DESC: &str =
    "Guard: test the value at a JSON pointer ({ value, path?, op, to? }; ops ==, !=, >, >=, <, \
     <=, exists, not_empty; path defaults to the whole value; a pointer miss FAILS the guard, \
     it never errors). Direct calls return { passed, value }. As a fp::pipe step, a passing \
     guard threads the ORIGINAL value onward unchanged and a failing one STOPS the pipe \
     (`short_circuited: true`), so a trailing write step (e.g. state::set) runs only when the \
     condition holds.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// JSON pointer selecting the value to return (e.g. `/content`).
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PickRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// Top-level keys to keep; missing keys are silently omitted.
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OmitRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// Top-level keys to drop; missing keys are ignored.
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TakeRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// How many array elements / string characters to keep.
    pub n: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DropRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// How many array elements / string characters to skip.
    pub n: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MapRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// JSON pointer plucked from each element (e.g. `/id`). Sporadic misses
    /// become null; a path matching NO element is an error (stored fields
    /// only — no computed properties like `/length`).
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FilterRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// Partial object an element's top-level properties must equal.
    pub matches: Map<String, Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SplitRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// Non-empty separator to split on (e.g. `"\n"`).
    pub separator: String,
}

fn default_separator() -> String {
    ",".into()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct JoinRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// Separator between elements (default `","`).
    #[serde(default = "default_separator")]
    pub separator: String,
}

/// Shared request for the value-only ops (uniq/size/compact/flatten/reverse).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValueRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NthRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// Index to pluck; negative counts from the end (`-1` = last element).
    pub n: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetOrRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// JSON pointer selecting the value to return (e.g. `/content`).
    pub path: String,
    /// Returned when the path matches nothing (a stored null is a hit).
    pub default: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SortByRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// JSON pointer plucked from each element as the sort key (e.g. `/score`).
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UtilResponse {
    pub value: Value,
}

/// The comparison a `fp::when` guard runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum WhenOp {
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "exists")]
    Exists,
    #[serde(rename = "not_empty")]
    NotEmpty,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WhenRequest {
    /// Input value (a pipe lands the previous step's value here).
    pub value: Value,
    /// JSON pointer selecting what to test (default: the whole value).
    #[serde(default)]
    pub path: Option<String>,
    /// One of "==", "!=", ">", ">=", "<", "<=", "exists", "not_empty".
    pub op: WhenOp,
    /// Right-hand side for the comparison ops; meaningless (and rejected) for
    /// exists / not_empty.
    #[serde(default)]
    pub to: Option<Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WhenResponse {
    pub passed: bool,
    /// The ORIGINAL input value, untouched — guards test, they never reshape.
    pub value: Value,
}

/// Evaluate a guard. A pointer miss FAILS the guard (Ok(false)) instead of
/// erroring: "the thing I am waiting for is not there yet" is exactly what a
/// guard exists to say. `exists` counts a stored null as a hit (same rule as
/// `fp::getOr`); `not_empty` does not.
pub fn when(
    value: &Value,
    path: Option<&str>,
    op: WhenOp,
    to: Option<&Value>,
) -> Result<bool, String> {
    let needs_to = matches!(
        op,
        WhenOp::Eq | WhenOp::Ne | WhenOp::Gt | WhenOp::Ge | WhenOp::Lt | WhenOp::Le
    );
    if needs_to && to.is_none() {
        return Err("this op needs `to` (the right-hand side to compare against)".into());
    }
    if !needs_to && to.is_some() {
        return Err("`to` is meaningless for exists/not_empty — drop it".into());
    }
    let Some(target) = value.pointer(path.unwrap_or("")) else {
        return Ok(false);
    };
    Ok(match op {
        WhenOp::Exists => true,
        WhenOp::NotEmpty => match target {
            Value::Array(a) => !a.is_empty(),
            Value::String(s) => !s.is_empty(),
            Value::Object(m) => !m.is_empty(),
            Value::Null => false,
            other => {
                return Err(format!(
                    "not_empty needs an array/string/object value (got {})",
                    kind_of(other)
                ))
            }
        },
        WhenOp::Eq => Some(target) == to,
        WhenOp::Ne => Some(target) != to,
        WhenOp::Gt | WhenOp::Ge | WhenOp::Lt | WhenOp::Le => {
            let (Some(l), Some(r)) = (target.as_f64(), to.and_then(Value::as_f64)) else {
                return Err(format!(
                    "ordering ops compare numbers; got {} vs {}",
                    kind_of(target),
                    to.map(kind_of).unwrap_or("nothing")
                ));
            };
            match op {
                WhenOp::Gt => l > r,
                WhenOp::Ge => l >= r,
                WhenOp::Lt => l < r,
                WhenOp::Le => l <= r,
                _ => unreachable!(),
            }
        }
    })
}

/// Parse + evaluate a guard step's args for the pipe loop: `(passed, the
/// original value to thread onward)`.
pub(crate) fn eval_when(args: &Value) -> Result<(bool, Value), String> {
    let req: WhenRequest =
        serde_json::from_value(args.clone()).map_err(|e| format!("invalid arguments: {e}"))?;
    let passed = when(&req.value, req.path.as_deref(), req.op, req.to.as_ref())?;
    Ok((passed, req.value))
}

pub fn get(value: &Value, path: &str) -> Result<Value, String> {
    match value.pointer(path) {
        Some(v) => Ok(v.clone()),
        None => Err(format!(
            "path {path:?} matched nothing; available top-level keys: {}",
            top_level_keys(value)
        )),
    }
}

// Deviates from lodash on non-objects: erroring beats silently threading `{}`
// through the rest of a pipe.
// ponytail: top-level keys only; deep paths when a real pipeline needs them.
pub fn pick(value: Value, paths: &[String]) -> Result<Value, String> {
    match value {
        Value::Object(mut map) => Ok(Value::Object(
            paths
                .iter()
                .filter_map(|k| map.remove(k).map(|v| (k.clone(), v)))
                .collect::<Map<String, Value>>(),
        )),
        other => Err(format!(
            "pick needs an object value (got {})",
            kind_of(&other)
        )),
    }
}

pub fn omit(value: Value, paths: &[String]) -> Result<Value, String> {
    match value {
        Value::Object(mut map) => {
            for k in paths {
                map.remove(k);
            }
            Ok(Value::Object(map))
        }
        other => Err(format!(
            "omit needs an object value (got {})",
            kind_of(&other)
        )),
    }
}

pub fn take(value: Value, n: usize) -> Result<Value, String> {
    match value {
        Value::String(s) => Ok(Value::String(match s.char_indices().nth(n) {
            Some((byte_idx, _)) => s[..byte_idx].to_string(),
            None => s,
        })),
        Value::Array(mut items) => {
            items.truncate(n);
            Ok(Value::Array(items))
        }
        other => Err(format!(
            "take needs a string or array value (got {}); use fp::get to select one first",
            kind_of(&other)
        )),
    }
}

pub fn drop(value: Value, n: usize) -> Result<Value, String> {
    match value {
        Value::String(s) => Ok(Value::String(match s.char_indices().nth(n) {
            Some((byte_idx, _)) => s[byte_idx..].to_string(),
            None => String::new(),
        })),
        Value::Array(items) => Ok(Value::Array(items.into_iter().skip(n).collect())),
        other => Err(format!(
            "drop needs a string or array value (got {}); use fp::get to select one first",
            kind_of(&other)
        )),
    }
}

// Misses become null, mirroring lodash's undefined — a few holes in a mapped
// list are recoverable downstream; failing the whole pipe on one is not.
// A path matching NO element is different: that is a wrong path (live-tested:
// a model tried lodash's computed `/length` on strings and would have threaded
// all-nulls through the rest of the pipe), so it errors teachably instead.
pub fn map(value: Value, path: &str) -> Result<Value, String> {
    match value {
        Value::Array(items) => {
            let mut misses = 0usize;
            let plucked: Vec<Value> = items
                .iter()
                .map(|el| match el.pointer(path) {
                    Some(v) => v.clone(),
                    None => {
                        misses += 1;
                        Value::Null
                    }
                })
                .collect();
            if misses == items.len() && !items.is_empty() {
                let hint = match &items[0] {
                    Value::Object(_) => {
                        format!("first element keys: {}", top_level_keys(&items[0]))
                    }
                    other => format!(
                        "elements are {}s — JSON pointers pluck stored fields, not computed \
                         properties like /length",
                        kind_of(other)
                    ),
                };
                return Err(format!(
                    "path {path:?} matched nothing in any of the {} elements; {hint}",
                    items.len()
                ));
            }
            Ok(Value::Array(plucked))
        }
        other => Err(format!(
            "map needs an array value (got {})",
            kind_of(&other)
        )),
    }
}

pub fn filter(value: Value, matches: &Map<String, Value>) -> Result<Value, String> {
    match value {
        Value::Array(items) => Ok(Value::Array(
            items
                .into_iter()
                .filter(|el| matches.iter().all(|(k, want)| el.get(k) == Some(want)))
                .collect(),
        )),
        other => Err(format!(
            "filter needs an array value (got {})",
            kind_of(&other)
        )),
    }
}

pub fn split(value: Value, separator: &str) -> Result<Value, String> {
    if separator.is_empty() {
        return Err("split needs a non-empty separator".into());
    }
    match value {
        Value::String(s) => Ok(Value::Array(
            s.split(separator)
                .map(|p| Value::String(p.to_string()))
                .collect(),
        )),
        other => Err(format!(
            "split needs a string value (got {})",
            kind_of(&other)
        )),
    }
}

pub fn join(value: Value, separator: &str) -> Result<Value, String> {
    match value {
        Value::Array(items) => Ok(Value::String(
            items
                .iter()
                .map(|el| match el {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(separator),
        )),
        other => Err(format!(
            "join needs an array value (got {})",
            kind_of(&other)
        )),
    }
}

// ponytail: serialized-form equality (O(n·size)); fine at pipe scale.
pub fn uniq(value: Value) -> Result<Value, String> {
    match value {
        Value::Array(items) => {
            let mut seen = std::collections::HashSet::new();
            Ok(Value::Array(
                items
                    .into_iter()
                    .filter(|el| seen.insert(el.to_string()))
                    .collect(),
            ))
        }
        other => Err(format!(
            "uniq needs an array value (got {})",
            kind_of(&other)
        )),
    }
}

pub fn size(value: &Value) -> Result<Value, String> {
    match value {
        Value::Array(a) => Ok(json_len(a.len())),
        Value::String(s) => Ok(json_len(s.chars().count())),
        Value::Object(m) => Ok(json_len(m.len())),
        // Deviates from lodash (_.size(5) == 0): a 0 for a non-collection
        // would thread garbage into a take/drop downstream.
        other => Err(format!(
            "size needs an array, string, or object value (got {})",
            kind_of(other)
        )),
    }
}

// Deviates from lodash: null-only, NOT full falsey — compacting away a
// legitimate 0, false, or "" plucked by map would silently corrupt data.
pub fn compact(value: Value) -> Result<Value, String> {
    match value {
        Value::Array(items) => Ok(Value::Array(
            items.into_iter().filter(|el| !el.is_null()).collect(),
        )),
        other => Err(format!(
            "compact needs an array value (got {})",
            kind_of(&other)
        )),
    }
}

pub fn nth(value: Value, n: i64) -> Result<Value, String> {
    match value {
        Value::Array(items) => {
            let len = items.len() as i64;
            let idx = if n < 0 { len + n } else { n };
            if idx < 0 || idx >= len {
                // Deviates from lodash's silent undefined: out of bounds is
                // a wrong assumption about the data, so it errors teachably.
                return Err(format!(
                    "nth index {n} is out of bounds for an array of {len} elements"
                ));
            }
            Ok(items
                .into_iter()
                .nth(idx as usize)
                .expect("index bounds checked"))
        }
        other => Err(format!(
            "nth needs an array value (got {})",
            kind_of(&other)
        )),
    }
}

pub fn get_or(value: &Value, path: &str, default: Value) -> Result<Value, String> {
    Ok(value.pointer(path).cloned().unwrap_or(default))
}

pub fn flatten(value: Value) -> Result<Value, String> {
    match value {
        Value::Array(items) => Ok(Value::Array(
            items
                .into_iter()
                .flat_map(|el| match el {
                    Value::Array(inner) => inner,
                    other => vec![other],
                })
                .collect(),
        )),
        other => Err(format!(
            "flatten needs an array value (got {})",
            kind_of(&other)
        )),
    }
}

// Deviates from lodash's coercing comparator: keys must all be one
// comparable type — a mixed or missing key means the sort order would be
// garbage, so it errors naming what it found.
pub fn sort_by(value: Value, path: &str) -> Result<Value, String> {
    let Value::Array(items) = value else {
        return Err(format!(
            "sortBy needs an array value (got {})",
            kind_of(&value)
        ));
    };
    let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(items.len());
    for (i, el) in items.into_iter().enumerate() {
        let Some(key) = el.pointer(path).cloned() else {
            // live-tested (haiku-4.5): "." is the common wrong guess for
            // "the element itself" — the JSON pointer for that is ""
            let hint = if path == "." {
                " (the JSON pointer for the element itself is \"\")"
            } else {
                ""
            };
            return Err(format!(
                "path {path:?} matched nothing in element {i}; sorting with holes would \
                 scramble the order{hint}"
            ));
        };
        keyed.push((key, el));
    }
    let Some(kind) = keyed.first().map(|(k, _)| kind_of(k)) else {
        return Ok(Value::Array(Vec::new()));
    };
    if let Some((other, _)) = keyed.iter().find(|(k, _)| kind_of(k) != kind) {
        return Err(format!(
            "sortBy keys mix {kind} and {} — sort one type at a time",
            kind_of(other)
        ));
    }
    match kind {
        "number" => keyed.sort_by(|a, b| {
            let (x, y) = (
                a.0.as_f64().unwrap_or(f64::NAN),
                b.0.as_f64().unwrap_or(f64::NAN),
            );
            x.total_cmp(&y)
        }),
        "string" | "boolean" => keyed.sort_by(|a, b| cmp_scalar(&a.0, &b.0)),
        other => {
            return Err(format!(
                "sortBy keys must be numbers, strings, or booleans (got {other})"
            ))
        }
    }
    Ok(Value::Array(keyed.into_iter().map(|(_, el)| el).collect()))
}

pub fn reverse(value: Value) -> Result<Value, String> {
    match value {
        Value::Array(mut items) => {
            items.reverse();
            Ok(Value::Array(items))
        }
        other => Err(format!(
            "reverse needs an array value (got {})",
            kind_of(&other)
        )),
    }
}

fn json_len(n: usize) -> Value {
    Value::Number(serde_json::Number::from(n as u64))
}

/// Same-type scalar sort keys only (sort_by guarantees homogeneity upstream).
fn cmp_scalar(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

/// True when `function_id` names one of the transforms. Every transform
/// takes its input at `value`, which the pipe uses to fail fast on an
/// unseeded first step. `fp::when` is listed here (same seeding rule) but is
/// dispatched by the pipe loop itself, never through [`apply`] — a failing
/// guard must be able to STOP the pipe, which `apply`'s value-or-error shape
/// cannot express.
pub(crate) fn is_transform(function_id: &str) -> bool {
    matches!(
        function_id,
        GET_ID
            | PICK_ID
            | OMIT_ID
            | TAKE_ID
            | DROP_ID
            | MAP_ID
            | FILTER_ID
            | SPLIT_ID
            | JOIN_ID
            | UNIQ_ID
            | SIZE_ID
            | COMPACT_ID
            | NTH_ID
            | GET_OR_ID
            | FLATTEN_ID
            | SORT_BY_ID
            | REVERSE_ID
            | WHEN_ID
    )
}

/// Inline dispatch for pipe steps: `None` when `function_id` is not a
/// transform, otherwise the transformed value (or a teachable error).
pub(crate) fn apply(function_id: &str, args: &Value) -> Option<Result<Value, String>> {
    fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, String> {
        serde_json::from_value(args.clone()).map_err(|e| format!("invalid arguments: {e}"))
    }
    match function_id {
        GET_ID => Some(parse::<GetRequest>(args).and_then(|r| get(&r.value, &r.path))),
        PICK_ID => Some(parse::<PickRequest>(args).and_then(|r| pick(r.value, &r.paths))),
        OMIT_ID => Some(parse::<OmitRequest>(args).and_then(|r| omit(r.value, &r.paths))),
        TAKE_ID => Some(parse::<TakeRequest>(args).and_then(|r| take(r.value, r.n as usize))),
        DROP_ID => Some(parse::<DropRequest>(args).and_then(|r| drop(r.value, r.n as usize))),
        MAP_ID => Some(parse::<MapRequest>(args).and_then(|r| map(r.value, &r.path))),
        FILTER_ID => Some(parse::<FilterRequest>(args).and_then(|r| filter(r.value, &r.matches))),
        SPLIT_ID => Some(parse::<SplitRequest>(args).and_then(|r| split(r.value, &r.separator))),
        JOIN_ID => Some(parse::<JoinRequest>(args).and_then(|r| join(r.value, &r.separator))),
        UNIQ_ID => Some(parse::<ValueRequest>(args).and_then(|r| uniq(r.value))),
        SIZE_ID => Some(parse::<ValueRequest>(args).and_then(|r| size(&r.value))),
        COMPACT_ID => Some(parse::<ValueRequest>(args).and_then(|r| compact(r.value))),
        NTH_ID => Some(parse::<NthRequest>(args).and_then(|r| nth(r.value, r.n))),
        GET_OR_ID => {
            Some(parse::<GetOrRequest>(args).and_then(|r| get_or(&r.value, &r.path, r.default)))
        }
        FLATTEN_ID => Some(parse::<ValueRequest>(args).and_then(|r| flatten(r.value))),
        SORT_BY_ID => Some(parse::<SortByRequest>(args).and_then(|r| sort_by(r.value, &r.path))),
        REVERSE_ID => Some(parse::<ValueRequest>(args).and_then(|r| reverse(r.value))),
        _ => None,
    }
}

pub(crate) fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn top_level_keys(v: &Value) -> String {
    match v {
        Value::Object(m) => m.keys().cloned().collect::<Vec<_>>().join(", "),
        other => kind_of(other).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn get_extracts_by_pointer_with_teachable_misses() {
        let v = json!({ "content": "abcdef", "status": 200 });
        assert_eq!(get(&v, "/content").unwrap(), json!("abcdef"));
        // root pointer is the identity
        assert_eq!(get(&v, "").unwrap(), v);
        // a miss names what WAS available
        let err = get(&v, "/body").unwrap_err();
        assert!(err.contains("content") && err.contains("status"));
    }

    #[test]
    fn pick_and_omit_subset_objects_and_refuse_the_rest() {
        let v = json!({ "a": 1, "b": 2, "c": 3 });
        assert_eq!(
            pick(v.clone(), &["a".into(), "c".into(), "missing".into()]).unwrap(),
            json!({ "a": 1, "c": 3 })
        );
        assert_eq!(
            omit(v, &["b".into(), "missing".into()]).unwrap(),
            json!({ "a": 1, "c": 3 })
        );
        assert!(pick(json!("s"), &["a".into()])
            .unwrap_err()
            .contains("object"));
        assert!(omit(json!([1]), &["a".into()])
            .unwrap_err()
            .contains("object"));
    }

    #[test]
    fn take_and_drop_slice_strings_and_arrays() {
        // char boundaries, not bytes
        assert_eq!(take(json!("héllo"), 2).unwrap(), json!("hé"));
        assert_eq!(take(json!("hi"), 10).unwrap(), json!("hi"));
        assert_eq!(take(json!("hi"), 0).unwrap(), json!(""));
        assert_eq!(take(json!([1, 2, 3]), 2).unwrap(), json!([1, 2]));
        assert_eq!(take(json!([]), 3).unwrap(), json!([]));
        assert!(take(json!({ "a": 1 }), 2).unwrap_err().contains("string"));

        assert_eq!(drop(json!("héllo"), 2).unwrap(), json!("llo"));
        assert_eq!(drop(json!("hi"), 10).unwrap(), json!(""));
        assert_eq!(drop(json!([1, 2, 3]), 2).unwrap(), json!([3]));
        assert!(drop(json!(5), 1).unwrap_err().contains("string"));
    }

    #[test]
    fn map_plucks_with_null_misses_but_rejects_a_total_miss() {
        let v = json!([{ "id": 1 }, { "id": 2 }, { "other": 3 }]);
        assert_eq!(map(v, "/id").unwrap(), json!([1, 2, null]));
        assert!(map(json!("s"), "/id").unwrap_err().contains("array"));

        // a stored null is a hit, not a miss
        assert_eq!(map(json!([{ "id": null }]), "/id").unwrap(), json!([null]));
        // empty input has no wrong path to detect
        assert_eq!(map(json!([]), "/id").unwrap(), json!([]));

        // a path matching NOTHING errors instead of threading all-nulls —
        // naming the keys that exist...
        let err = map(json!([{ "id": 1 }, { "id": 2 }]), "/uuid").unwrap_err();
        assert!(err.contains("matched nothing") && err.contains("id"));
        // ...and teaching the computed-property trap on primitives
        // (the live session tried lodash's `/length` on strings)
        let err = map(json!(["apple", "banana"]), "/length").unwrap_err();
        assert!(err.contains("string") && err.contains("computed"));
    }

    #[test]
    fn is_transform_matches_exactly_the_seventeen_ops() {
        for id in [
            GET_ID, PICK_ID, OMIT_ID, TAKE_ID, DROP_ID, MAP_ID, FILTER_ID, SPLIT_ID, JOIN_ID,
            UNIQ_ID, SIZE_ID, COMPACT_ID, NTH_ID, GET_OR_ID, FLATTEN_ID, SORT_BY_ID, REVERSE_ID,
        ] {
            assert!(is_transform(id));
        }
        assert!(!is_transform("fp::pipe"));
        assert!(!is_transform("state::set"));
    }

    #[test]
    fn size_counts_collections_and_refuses_the_rest() {
        assert_eq!(size(&json!([1, 2, 3])).unwrap(), json!(3));
        // chars, not bytes
        assert_eq!(size(&json!("héllo")).unwrap(), json!(5));
        assert_eq!(size(&json!({"a": 1, "b": 2})).unwrap(), json!(2));
        assert_eq!(size(&json!([])).unwrap(), json!(0));
        // lodash returns 0 here; a silent 0 threads garbage, so we error
        assert!(size(&json!(5)).unwrap_err().contains("array"));
        assert!(size(&json!(null)).unwrap_err().contains("array"));
    }

    #[test]
    fn compact_removes_null_only() {
        // NOT lodash falsey: 0, false and "" are data and survive
        assert_eq!(
            compact(json!([0, null, false, "", 1, null, "x"])).unwrap(),
            json!([0, false, "", 1, "x"])
        );
        assert!(compact(json!("s")).unwrap_err().contains("array"));
    }

    #[test]
    fn nth_indexes_both_ends_and_errors_out_of_bounds() {
        let v = json!(["a", "b", "c"]);
        assert_eq!(nth(v.clone(), 0).unwrap(), json!("a"));
        assert_eq!(nth(v.clone(), 2).unwrap(), json!("c"));
        // the hole JSON Pointer cannot express: the last element
        assert_eq!(nth(v.clone(), -1).unwrap(), json!("c"));
        assert_eq!(nth(v.clone(), -3).unwrap(), json!("a"));
        let err = nth(v.clone(), 3).unwrap_err();
        assert!(err.contains("3 elements"));
        assert!(nth(v, -4).unwrap_err().contains("out of bounds"));
        assert!(nth(json!({}), 0).unwrap_err().contains("array"));
    }

    #[test]
    fn get_or_defaults_only_on_a_true_miss() {
        let v = json!({ "a": 1, "b": null });
        assert_eq!(get_or(&v, "/a", json!("dflt")).unwrap(), json!(1));
        assert_eq!(
            get_or(&v, "/missing", json!("dflt")).unwrap(),
            json!("dflt")
        );
        // a stored null is a hit, mirroring map's rule
        assert_eq!(get_or(&v, "/b", json!("dflt")).unwrap(), json!(null));
    }

    #[test]
    fn flatten_unnests_one_level() {
        assert_eq!(
            flatten(json!([1, [2, 3], [[4]]])).unwrap(),
            json!([1, 2, 3, [4]])
        );
        assert_eq!(flatten(json!([])).unwrap(), json!([]));
        assert!(flatten(json!(1)).unwrap_err().contains("array"));
    }

    #[test]
    fn sort_by_is_stable_typed_and_loud() {
        let v = json!([
            { "s": 3, "id": "c" },
            { "s": 1, "id": "a" },
            { "s": 3, "id": "b" },
        ]);
        // ascending, stable: the two s=3 keep their relative order
        assert_eq!(
            sort_by(v, "/s").unwrap(),
            json!([
                { "s": 1, "id": "a" },
                { "s": 3, "id": "c" },
                { "s": 3, "id": "b" },
            ])
        );
        assert_eq!(
            sort_by(json!([{ "n": "b" }, { "n": "a" }]), "/n").unwrap(),
            json!([{ "n": "a" }, { "n": "b" }])
        );
        assert_eq!(sort_by(json!([]), "/x").unwrap(), json!([]));

        // path "" sorts primitives by themselves; "." gets the redirect hint
        assert_eq!(sort_by(json!([3, 1, 2]), "").unwrap(), json!([1, 2, 3]));
        assert!(sort_by(json!([2, 1]), ".")
            .unwrap_err()
            .contains("element itself"));

        // holes and mixed types would scramble the order — loud, not coerced
        let err = sort_by(json!([{ "s": 1 }, {}]), "/s").unwrap_err();
        assert!(err.contains("element 1"));
        let err = sort_by(json!([{ "s": 1 }, { "s": "x" }]), "/s").unwrap_err();
        assert!(err.contains("number") && err.contains("string"));
        let err = sort_by(json!([{ "s": {} }]), "/s").unwrap_err();
        assert!(err.contains("numbers, strings, or booleans"));
        assert!(sort_by(json!("s"), "/x").unwrap_err().contains("array"));
    }

    #[test]
    fn reverse_flips_arrays_immutably() {
        assert_eq!(reverse(json!([1, 2, 3])).unwrap(), json!([3, 2, 1]));
        assert_eq!(reverse(json!([])).unwrap(), json!([]));
        assert!(reverse(json!("abc")).unwrap_err().contains("array"));
    }

    #[test]
    fn filter_keeps_matching_elements() {
        let v = json!([
            { "status": "active", "id": 1 },
            { "status": "gone", "id": 2 },
            "not-an-object",
            { "status": "active", "id": 3 },
        ]);
        let matches = json!({ "status": "active" });
        let Value::Object(matches) = matches else {
            unreachable!()
        };
        assert_eq!(
            filter(v.clone(), &matches).unwrap(),
            json!([{ "status": "active", "id": 1 }, { "status": "active", "id": 3 }])
        );
        // empty matches keeps everything (vacuous truth, lodash-compatible)
        assert_eq!(filter(v.clone(), &Map::new()).unwrap(), v);
        assert!(filter(json!({}), &Map::new())
            .unwrap_err()
            .contains("array"));
    }

    #[test]
    fn split_and_join_round_trip_strings() {
        assert_eq!(
            split(json!("a\nb\nc"), "\n").unwrap(),
            json!(["a", "b", "c"])
        );
        assert!(split(json!("x"), "").unwrap_err().contains("non-empty"));
        assert!(split(json!(1), ",").unwrap_err().contains("string"));

        assert_eq!(join(json!(["a", "b"]), "\n").unwrap(), json!("a\nb"));
        // non-strings serialize
        assert_eq!(join(json!([1, "x", null]), ",").unwrap(), json!("1,x,null"));
        assert!(join(json!("s"), ",").unwrap_err().contains("array"));
    }

    #[test]
    fn uniq_keeps_first_occurrences() {
        assert_eq!(
            uniq(json!([1, 2, 1, { "a": 1 }, { "a": 1 }, "1"])).unwrap(),
            json!([1, 2, { "a": 1 }, "1"])
        );
        assert!(uniq(json!("s")).unwrap_err().contains("array"));
    }

    #[test]
    fn when_guards_compare_exist_and_fail_on_misses() {
        let v = json!({ "rows": [{ "n": 3 }], "empty": [], "name": "x", "nil": null });
        let w = |path: Option<&str>, op: WhenOp, to: Option<Value>| {
            when(&v, path, op, to.as_ref())
        };
        // Ordering over a pointer.
        assert_eq!(w(Some("/rows/0/n"), WhenOp::Ge, Some(json!(3))), Ok(true));
        assert_eq!(w(Some("/rows/0/n"), WhenOp::Gt, Some(json!(3))), Ok(false));
        assert_eq!(w(Some("/rows/0/n"), WhenOp::Lt, Some(json!(10))), Ok(true));
        // Structural equality, any JSON type.
        assert_eq!(w(Some("/name"), WhenOp::Eq, Some(json!("x"))), Ok(true));
        assert_eq!(w(Some("/name"), WhenOp::Ne, Some(json!("y"))), Ok(true));
        // A pointer miss FAILS the guard for every op — never errors.
        assert_eq!(w(Some("/missing"), WhenOp::Ge, Some(json!(1))), Ok(false));
        assert_eq!(w(Some("/missing"), WhenOp::Ne, Some(json!(1))), Ok(false));
        assert_eq!(w(Some("/missing"), WhenOp::Exists, None), Ok(false));
        // exists counts a stored null as a hit (getOr rule); not_empty doesn't.
        assert_eq!(w(Some("/nil"), WhenOp::Exists, None), Ok(true));
        assert_eq!(w(Some("/nil"), WhenOp::NotEmpty, None), Ok(false));
        assert_eq!(w(Some("/rows"), WhenOp::NotEmpty, None), Ok(true));
        assert_eq!(w(Some("/empty"), WhenOp::NotEmpty, None), Ok(false));
        // Default path tests the whole value.
        assert_eq!(w(None, WhenOp::Exists, None), Ok(true));
        // Teachable errors: missing/extra `to`, non-number ordering.
        assert!(w(Some("/name"), WhenOp::Ge, None).unwrap_err().contains("needs `to`"));
        assert!(w(Some("/name"), WhenOp::Exists, Some(json!(1)))
            .unwrap_err()
            .contains("meaningless"));
        assert!(w(Some("/name"), WhenOp::Ge, Some(json!(1)))
            .unwrap_err()
            .contains("numbers"));
        assert!(w(Some("/rows/0/n"), WhenOp::NotEmpty, None)
            .unwrap_err()
            .contains("array/string/object"));
    }

    #[test]
    fn eval_when_threads_the_original_value() {
        let args = json!({ "value": { "n": 5 }, "path": "/n", "op": ">=", "to": 3 });
        assert_eq!(eval_when(&args), Ok((true, json!({ "n": 5 }))));
        let args = json!({ "value": { "n": 1 }, "path": "/n", "op": ">=", "to": 3 });
        assert_eq!(eval_when(&args), Ok((false, json!({ "n": 1 }))));
        assert!(eval_when(&json!({ "op": ">=" }))
            .unwrap_err()
            .contains("invalid arguments"));
    }

    #[test]
    fn apply_dispatches_transform_ids_only() {
        assert!(apply("state::set", &json!({})).is_none());
        assert!(apply("fp::pipe", &json!({})).is_none());
        assert_eq!(
            apply(GET_ID, &json!({ "value": { "a": 1 }, "path": "/a" })),
            Some(Ok(json!(1)))
        );
        assert_eq!(
            apply(TAKE_ID, &json!({ "value": "héllo", "n": 2 })),
            Some(Ok(json!("hé")))
        );
        // join's separator defaults like lodash
        assert_eq!(
            apply(JOIN_ID, &json!({ "value": ["a", "b"] })),
            Some(Ok(json!("a,b")))
        );
        assert_eq!(
            apply(
                FILTER_ID,
                &json!({ "value": [{"s": 1}, {"s": 2}], "matches": {"s": 2} })
            ),
            Some(Ok(json!([{"s": 2}])))
        );
        // bad shape is a step error, not a panic
        assert!(apply(TAKE_ID, &json!({ "value": "x" }))
            .unwrap()
            .unwrap_err()
            .contains("invalid arguments"));
    }
}
