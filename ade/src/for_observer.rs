//! Calls for one observer (iii decision 52).
//!
//! The console sends one observer something of its own (a tab's served
//! asset set, the first workspace ring, an asset's content fetch) by calling
//! its own function with metadata `{ "for_observer": <observer id> }` once
//! the observer is accepted. The function's match hook gives that call to
//! that observer alone, and iii delivers it like every call of the function,
//! with the observer's `deliver`, filters and policy, so a target receives
//! every event of the function in one shape.

use iii_sdk::{Call, CallAction, CallRequest, CallRequestWithMetadata, Match};
use serde_json::{json, Value};

/// The metadata key that names the observer a call is for.
pub const KEY: &str = "for_observer";

/// The observer `call` is for, when it is for one.
pub fn of(call: &Call) -> Option<&str> {
    call.metadata
        .as_ref()
        .and_then(|metadata| metadata.get(KEY))
        .and_then(Value::as_str)
}

/// A call of `function_id` with `payload` that only the observer
/// `observer_id` matches (under [`every_call`] or [`own_calls_only`]).
pub fn call(function_id: &str, payload: Value, observer_id: &str) -> CallRequestWithMetadata {
    CallRequest::new(function_id, payload).metadata(json!({ KEY: observer_id }))
}

/// The same call, fire and forget.
pub fn void_call(function_id: &str, payload: Value, observer_id: &str) -> CallRequestWithMetadata {
    call(function_id, payload, observer_id).action(CallAction::Void)
}

/// Every observer wants every call, except a call for another observer.
pub fn every_call() -> Match {
    Match::predicate(|obs, call| of(call).is_none_or(|id| id == obs.id()))
}

/// An observer wants only the calls made for it.
pub fn own_calls_only() -> Match {
    Match::predicate(|obs, call| of(call) == Some(obs.id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_for(observer: Option<&str>) -> Call {
        Call {
            metadata: observer.map(|id| json!({ KEY: id })),
            ..Call::default()
        }
    }

    #[test]
    fn of_reads_the_observer_a_call_is_for() {
        assert_eq!(of(&call_for(Some("obs-1"))), Some("obs-1"));
        assert_eq!(of(&call_for(None)), None);
        let other_metadata = Call {
            metadata: Some(json!({ "trace": "t" })),
            ..Call::default()
        };
        assert_eq!(of(&other_metadata), None);
        let not_a_string = Call {
            metadata: Some(json!({ KEY: 7 })),
            ..Call::default()
        };
        assert_eq!(of(&not_a_string), None);
    }

    #[test]
    fn the_call_names_its_observer_in_metadata() {
        let request = call("console::ui::asset", json!({ "path": "a/b.js" }), "obs-1");
        let debug = format!("{request:?}");
        assert!(debug.contains("for_observer"), "{debug}");
        assert!(debug.contains("obs-1"), "{debug}");
    }
}
