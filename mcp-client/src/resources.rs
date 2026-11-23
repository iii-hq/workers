//! Resource functions: the three constant definitions and the renaming of
//! list pages.

use serde_json::{Map, Value, json};

use crate::host::FunctionDef;
use crate::reconcile::{Target, Wanted, WantedSet, tool_response_format};
use crate::translate::cap_page;

pub fn list_id(server: &str) -> String {
    format!("mcp::{server}::resources::list")
}
pub fn read_id(server: &str) -> String {
    format!("mcp::{server}::resources::read")
}
pub fn templates_id(server: &str) -> String {
    format!("mcp::{server}::resources::templates")
}

fn page_schema(key: &str, uri_key: &str, with_size: bool) -> Value {
    let mut item = json!({"type": "object", "properties": {
        uri_key: {"type": "string"}, "name": {"type": "string"}, "title": {"type": "string"},
        "description": {"type": "string"}, "mime_type": {"type": "string"}},
        "required": [uri_key, "name"]});
    if with_size {
        item["properties"]["size"] = json!({"type": "integer"});
    }
    json!({"type": "object", "properties": {
        key: {"type": "array", "items": item},
        "next_cursor": {"type": "string"}, "truncated": {"type": "boolean"}},
        "required": [key]})
}

pub fn resource_wanted(server: &str, with_templates: bool) -> WantedSet {
    let cursor = json!({"type": "object", "properties": {"cursor": {"type": "string"}}, "additionalProperties": false});
    let uri = json!({"type": "object", "properties": {"uri": {"type": "string"}}, "required": ["uri"], "additionalProperties": false});
    let mut rows = vec![
        (
            list_id(server),
            format!(
                "List the resources of the `{server}` MCP server: uri, name, description and MIME type. Pass `cursor` from `next_cursor` for the next page. Read one with `mcp::{server}::resources::read`."
            ),
            cursor.clone(),
            page_schema("resources", "uri", true),
            "list",
            Target::ResourceList,
        ),
        (
            read_id(server),
            format!(
                "Read one resource of the `{server}` MCP server by `uri`. Find uris with `mcp::{server}::resources::list`, or fill in a template from `mcp::{server}::resources::templates` when the server has that function."
            ),
            uri,
            tool_response_format(),
            "read",
            Target::ResourceRead,
        ),
    ];
    if with_templates {
        rows.push((
            templates_id(server),
            format!("List the resource URI templates (RFC 6570) of the `{server}` MCP server. Fill in a template's variables yourself, then read the uri with `mcp::{server}::resources::read`."),
            cursor,
            page_schema("templates", "uri_template", false),
            "templates",
            Target::ResourceTemplates,
        ));
    }
    rows.into_iter()
        .map(
            |(id, description, request_format, response_format, kind, target)| {
                let def = FunctionDef {
                    id: id.clone(),
                    description,
                    request_format,
                    response_format,
                    metadata: json!({"mcp": {"server": server, "resource": kind}}),
                };
                (id, Wanted { def, target })
            },
        )
        .collect()
}

fn page(result: &Value, wire_key: &str, key: &str, fields: &[(&str, &str)]) -> Value {
    let items: Vec<Value> = result
        .get(wire_key)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|i| {
            let row: Map<String, Value> = fields
                .iter()
                .filter_map(|(from, to)| Some((to.to_string(), i.get(*from)?.clone())))
                .collect();
            Value::Object(row)
        })
        .collect();
    let mut page = json!({ key: items });
    if let Some(c) = result.get("nextCursor").filter(|c| c.is_string()) {
        page["next_cursor"] = c.clone();
    }
    cap_page(page, key)
}

pub fn list_page(result: &Value) -> Value {
    let f = [
        ("uri", "uri"),
        ("name", "name"),
        ("title", "title"),
        ("description", "description"),
        ("mimeType", "mime_type"),
        ("size", "size"),
    ];
    page(result, "resources", "resources", &f)
}

pub fn templates_page(result: &Value) -> Value {
    let f = [
        ("uriTemplate", "uri_template"),
        ("name", "name"),
        ("title", "title"),
        ("description", "description"),
        ("mimeType", "mime_type"),
    ];
    page(result, "resourceTemplates", "templates", &f)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::host::FunctionDef;
    use crate::reconcile::{Target, WantedSet, plan, tool_response_format};

    fn defs(w: &WantedSet) -> BTreeMap<String, FunctionDef> {
        w.iter().map(|(k, v)| (k.clone(), v.def.clone())).collect()
    }

    #[test]
    fn the_three_functions_have_constant_texts_and_schemas() {
        let w = resource_wanted("fake", true);
        let (l, r, t) = (list_id("fake"), read_id("fake"), templates_id("fake"));
        assert_eq!(
            w.keys().cloned().collect::<Vec<_>>(),
            [
                "mcp::fake::resources::list",
                "mcp::fake::resources::read",
                "mcp::fake::resources::templates"
            ]
        );
        assert_eq!(
            (l.as_str(), r.as_str(), t.as_str()),
            (
                "mcp::fake::resources::list",
                "mcp::fake::resources::read",
                "mcp::fake::resources::templates"
            )
        );
        assert_eq!(
            w[&list_id("fake")].def.description,
            "List the resources of the `fake` MCP server: uri, name, description and MIME type. Pass `cursor` from `next_cursor` for the next page. Read one with `mcp::fake::resources::read`."
        );
        assert_eq!(
            w[&read_id("fake")].def.description,
            "Read one resource of the `fake` MCP server by `uri`. Find uris with `mcp::fake::resources::list`, or fill in a template from `mcp::fake::resources::templates` when the server has that function."
        );
        assert_eq!(
            w[&templates_id("fake")].def.description,
            "List the resource URI templates (RFC 6570) of the `fake` MCP server. Fill in a template's variables yourself, then read the uri with `mcp::fake::resources::read`."
        );
        let cursor = json!({"type": "object", "properties": {"cursor": {"type": "string"}}, "additionalProperties": false});
        assert_eq!(w[&l].def.request_format, cursor);
        assert_eq!(w[&t].def.request_format, cursor);
        assert_eq!(
            w[&r].def.request_format,
            json!({"type": "object", "properties": {"uri": {"type": "string"}}, "required": ["uri"], "additionalProperties": false})
        );
        assert_eq!(w[&r].def.response_format, tool_response_format());
        assert_eq!(w[&l].def.response_format["required"], json!(["resources"]));
        assert_eq!(w[&t].def.response_format["required"], json!(["templates"]));
        assert_eq!(
            w[&r].def.metadata,
            json!({"mcp": {"server": "fake", "resource": "read"}})
        );
        assert_eq!(w[&l].target, Target::ResourceList);
        assert_eq!(w[&r].target, Target::ResourceRead);
        assert_eq!(w[&t].target, Target::ResourceTemplates);
    }

    #[test]
    fn templates_appear_and_go_in_the_plan() {
        let without = defs(&resource_wanted("fake", false));
        let with = defs(&resource_wanted("fake", true));
        assert_eq!(without.len(), 2);
        let p = plan(&without, &with);
        assert_eq!(
            (p.remove.len(), p.change.len(), p.add),
            (0, 0, vec![templates_id("fake")])
        );
        let p = plan(&with, &without);
        assert_eq!(
            (p.remove, p.change.len(), p.add.len()),
            (vec![templates_id("fake")], 0, 0)
        );
    }

    #[test]
    fn pages_are_renamed_and_carry_the_cursor() {
        let wire = json!({"resources": [
            {"uri": "file:///a.txt", "name": "a.txt", "title": "A", "description": "d", "mimeType": "text/plain", "size": 5, "annotations": {"priority": 1}},
            {"uri": "file:///b", "name": "b"}], "nextCursor": "2", "_meta": {}});
        assert_eq!(
            list_page(&wire),
            json!({"resources": [
            {"uri": "file:///a.txt", "name": "a.txt", "title": "A", "description": "d", "mime_type": "text/plain", "size": 5},
            {"uri": "file:///b", "name": "b"}], "next_cursor": "2"})
        );
        assert_eq!(list_page(&json!({})), json!({"resources": []}));
        let wire = json!({"resourceTemplates": [{"uriTemplate": "file:///{name}.txt", "name": "text file", "mimeType": "text/plain"}]});
        assert_eq!(
            templates_page(&wire),
            json!({"templates": [{"uri_template": "file:///{name}.txt", "name": "text file", "mime_type": "text/plain"}]})
        );
    }
}
