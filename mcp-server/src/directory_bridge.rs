//! MCP resources and the gateway's profile/skill tools, backed by the
//! `iii-directory` worker (`directory::skills::*`, `directory::agents::*`).
//!
//! Replaces the bridge to the `skills` and `prompts` workers, which no longer
//! exist. Skills are resources at `iii://<id>`; agent profiles are reached
//! through the gateway tools (the iii Claude plugin turns them into
//! `/iii:*` commands), so the bridge serves no MCP prompts.

use serde_json::{Map, Value, json};

use crate::bus::{Bus, BusError};

pub const SKILL_URI_PREFIX: &str = "iii://";

/// `{ skills: [...] }` or a bare array, as rows.
fn rows(v: Value, key: &str) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        Value::Object(mut o) => match o.remove(key) {
            Some(Value::Array(a)) => a,
            _ => vec![],
        },
        _ => vec![],
    }
}

pub async fn resources_list<B: Bus>(bus: &B) -> Result<Value, BusError> {
    let listed = match bus
        .call(
            "directory::skills::list",
            json!({ "include_description": true }),
            None,
        )
        .await
    {
        Ok(v) => v,
        Err(e) if e.is_missing_function() => return Ok(json!({ "resources": [] })),
        Err(e) => return Err(e),
    };
    let resources: Vec<Value> = rows(listed, "skills")
        .into_iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?.to_string();
            let mut r = Map::new();
            r.insert("uri".into(), json!(format!("{SKILL_URI_PREFIX}{id}")));
            r.insert("name".into(), json!(id));
            r.insert(
                "title".into(),
                row.get("title")
                    .filter(|t| t.is_string())
                    .cloned()
                    .unwrap_or(json!(id)),
            );
            if let Some(d) = row.get("description").filter(|d| d.is_string()) {
                r.insert("description".into(), d.clone());
            }
            r.insert("mimeType".into(), json!("text/markdown"));
            Some(Value::Object(r))
        })
        .collect();
    Ok(json!({ "resources": resources }))
}

fn is_unknown_skill(e: &BusError) -> bool {
    e.is_missing_function() || e.message.contains("D110") || e.message.contains("not_found")
}

pub async fn resources_read<B: Bus>(bus: &B, params: &Value) -> Result<Value, BusError> {
    let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
    let Some(id) = uri
        .strip_prefix(SKILL_URI_PREFIX)
        .filter(|id| !id.is_empty())
    else {
        return Ok(json!({ "contents": [] }));
    };
    match bus
        .call("directory::skills::get", json!({ "id": id }), None)
        .await
    {
        Ok(skill) => {
            let body = skill.get("body").and_then(Value::as_str).unwrap_or("");
            Ok(json!({ "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": body }] }))
        }
        Err(e) if is_unknown_skill(&e) => Ok(json!({ "contents": [] })),
        Err(e) => Err(e),
    }
}

pub fn resource_templates() -> Value {
    json!({ "resourceTemplates": [{
        "uriTemplate": "iii://{id}",
        "name": "iii-skill",
        "title": "iii skill",
        "description": "A skill from the engine's iii-directory, by id (e.g. iii://kanban).",
        "mimeType": "text/markdown"
    }]})
}

pub async fn get_agent<B: Bus>(bus: &B, id: &str) -> Result<Value, BusError> {
    bus.call("directory::agents::get", json!({ "id": id }), None)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{BusError, fake::FakeBus};
    use serde_json::json;

    fn skills_rows() -> Value {
        json!({ "skills": [
            { "id": "kanban", "title": "kanban", "description": "Boards." },
            { "id": "harness/orchestration", "title": "orchestration", "description": null }
        ]})
    }

    #[tokio::test]
    async fn resources_list_maps_directory_skills_to_iii_uris() {
        let bus = FakeBus::new().on("directory::skills::list", |_| Ok(skills_rows()));
        let r = resources_list(&bus).await.unwrap();
        let res = r["resources"].as_array().unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(
            res[0],
            json!({
                "uri": "iii://kanban", "name": "kanban", "title": "kanban",
                "description": "Boards.", "mimeType": "text/markdown"
            })
        );
        assert_eq!(res[1]["uri"], "iii://harness/orchestration");
        assert!(res[1].get("description").is_none());
        assert_eq!(bus.calls()[0].1, json!({ "include_description": true }));
    }

    #[tokio::test]
    async fn resources_list_is_empty_without_the_directory() {
        let bus = FakeBus::new();
        assert_eq!(
            resources_list(&bus).await.unwrap(),
            json!({ "resources": [] })
        );
    }

    #[tokio::test]
    async fn resources_list_propagates_other_errors() {
        let bus = FakeBus::new().on("directory::skills::list", |_| {
            Err(BusError::new("FORBIDDEN", "not allowed"))
        });
        assert_eq!(resources_list(&bus).await.unwrap_err().code, "FORBIDDEN");
    }

    #[tokio::test]
    async fn resources_read_returns_the_skill_body() {
        let bus = FakeBus::new().on("directory::skills::get", |p| {
            assert_eq!(p, json!({ "id": "harness/orchestration" }));
            Ok(json!({ "id": "harness/orchestration", "body": "# Orchestration" }))
        });
        let r = resources_read(&bus, &json!({ "uri": "iii://harness/orchestration" }))
            .await
            .unwrap();
        assert_eq!(
            r,
            json!({ "contents": [{
                "uri": "iii://harness/orchestration", "mimeType": "text/markdown", "text": "# Orchestration"
            }]})
        );
    }

    #[tokio::test]
    async fn resources_read_of_an_unknown_skill_is_empty() {
        let bus = FakeBus::new().on("directory::skills::get", |_| {
            Err(BusError::new(
                "invocation_failed",
                "D110 not_found: skill \"x\" does not exist",
            ))
        });
        let r = resources_read(&bus, &json!({ "uri": "iii://x" }))
            .await
            .unwrap();
        assert_eq!(r, json!({ "contents": [] }));
    }

    #[tokio::test]
    async fn resources_read_of_a_foreign_uri_is_empty_without_a_call() {
        let bus = FakeBus::new();
        let r = resources_read(&bus, &json!({ "uri": "file:///etc/passwd" }))
            .await
            .unwrap();
        assert_eq!(r, json!({ "contents": [] }));
        assert!(bus.calls().is_empty());
    }

    #[test]
    fn templates_name_the_skill_uri_shape() {
        let t = resource_templates();
        assert_eq!(t["resourceTemplates"][0]["uriTemplate"], "iii://{id}");
    }

    #[tokio::test]
    async fn get_agent_passes_the_id() {
        let bus = FakeBus::new().on("directory::agents::get", |p| {
            Ok(json!({ "id": p["id"], "system_prompt": "Be." }))
        });
        assert_eq!(
            get_agent(&bus, "writer").await.unwrap()["system_prompt"],
            "Be."
        );
    }
}
