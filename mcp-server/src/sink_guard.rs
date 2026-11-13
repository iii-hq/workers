//! Who may deliver to `mcp-server::observed`.
//!
//! The sink is a public function (the engine refuses a private observer
//! target), so any worker could call it with a forged `observer_id` and put
//! data in a session's buffer. The engine tells the two apart: it stamps
//! `_caller_worker_id` on every call a worker makes, and a delivery the
//! engine makes itself carries no stamp. A stamped delivery is accepted only
//! from the worker hosting the observed function (a source that delivers its
//! own observations); the stamp is compared by worker name.

use serde_json::Value;

use crate::bus::Bus;

const CALLER: &str = "_caller_worker_id";

/// Whether `delivery` may reach a session. Removes the engine's stamp.
pub async fn admit<B: Bus>(bus: &B, delivery: &mut Value) -> bool {
    let Some(stamp) = delivery.as_object_mut().and_then(|o| o.remove(CALLER)) else {
        return true;
    };
    let Some(caller_id) = stamp.as_str() else {
        return false;
    };
    let Some(function_id) = delivery.get("function_id").and_then(Value::as_str) else {
        return false;
    };
    let (Some(caller), Some(host)) = (
        worker_name(bus, caller_id).await,
        host_of(bus, function_id).await,
    ) else {
        return false;
    };
    caller == host
}

async fn worker_name<B: Bus>(bus: &B, worker_id: &str) -> Option<String> {
    let list = bus
        .call("engine::workers::list", serde_json::json!({}), None)
        .await
        .ok()?;
    list.get("workers")?
        .as_array()?
        .iter()
        .find(|w| w.get("id").and_then(Value::as_str) == Some(worker_id))?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

async fn host_of<B: Bus>(bus: &B, function_id: &str) -> Option<String> {
    let list = bus
        .call(
            "engine::functions::list",
            serde_json::json!({ "prefix": function_id }),
            None,
        )
        .await
        .ok()?;
    list.get("functions")?
        .as_array()?
        .iter()
        .find(|f| f.get("function_id").and_then(Value::as_str) == Some(function_id))?
        .get("worker_name")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::fake::FakeBus;
    use crate::bus::BusError;
    use serde_json::json;

    fn engine() -> FakeBus {
        FakeBus::new()
            .on("engine::workers::list", |_| {
                Ok(json!({ "workers": [
                    { "id": "w-http", "name": "http", "namespace": "default" },
                    { "id": "w-evil", "name": "evil", "namespace": "default" }
                ] }))
            })
            .on("engine::functions::list", |p| {
                assert_eq!(p["prefix"], "http::on-request");
                Ok(json!({ "functions": [
                    { "function_id": "http::on-request", "worker_name": "http" },
                    { "function_id": "http::on-request-x", "worker_name": "evil" }
                ] }))
            })
    }

    fn delivery(caller: Option<&str>) -> Value {
        let mut d = json!({ "observer_id": "o-1", "function_id": "http::on-request", "input": {} });
        if let Some(c) = caller {
            d["_caller_worker_id"] = json!(c);
        }
        d
    }

    #[tokio::test]
    async fn an_unstamped_delivery_comes_from_the_engine() {
        let bus = FakeBus::new();
        let mut d = delivery(None);
        assert!(admit(&bus, &mut d).await);
        // No lookup needed.
        assert!(bus.calls().is_empty());
    }

    #[tokio::test]
    async fn the_host_of_the_observed_function_may_deliver_its_own() {
        let bus = engine();
        let mut d = delivery(Some("w-http"));
        assert!(admit(&bus, &mut d).await);
        assert!(
            d.get("_caller_worker_id").is_none(),
            "the stamp is not an observation field"
        );
    }

    #[tokio::test]
    async fn any_other_worker_is_refused() {
        let bus = engine();
        for caller in ["w-evil", "w-unknown"] {
            let mut d = delivery(Some(caller));
            assert!(!admit(&bus, &mut d).await, "{caller}");
        }
    }

    #[tokio::test]
    async fn a_delivery_without_a_function_or_a_failed_lookup_is_refused() {
        let mut d = json!({ "observer_id": "o-1", "_caller_worker_id": "w-http" });
        assert!(!admit(&engine(), &mut d).await);
        let failing = FakeBus::new().on("engine::workers::list", |_| {
            Err(BusError::new("FORBIDDEN", "no"))
        });
        let mut d = delivery(Some("w-http"));
        assert!(!admit(&failing, &mut d).await);
        let mut stamped_oddly = delivery(None);
        stamped_oddly["_caller_worker_id"] = json!(7);
        assert!(!admit(&engine(), &mut stamped_oddly).await);
    }
}
