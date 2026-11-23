use mcp_client::functions::{CONFIG_VALIDATE, ON_SERVERS_CHANGED, SERVERS_LIST, catalog, flags};
use serde_json::Value;

/// The keys `collect_worker_interface.py` takes as a typed schema.
const DEFINING: [&str; 9] = [
    "type",
    "properties",
    "$ref",
    "allOf",
    "anyOf",
    "oneOf",
    "enum",
    "items",
    "const",
];

#[test]
fn every_operator_function_has_typed_schemas() {
    let catalog = catalog();
    let ids: Vec<&str> = catalog.iter().map(|(id, ..)| *id).collect();
    for id in [SERVERS_LIST, CONFIG_VALIDATE, ON_SERVERS_CHANGED] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
    for (id, request, response) in &catalog {
        assert!(id.starts_with("mcp-client::"), "{id}");
        for (side, schema) in [("request", request), ("response", response)] {
            let typed = schema
                .as_object()
                .is_some_and(|o| DEFINING.iter().any(|k| o.contains_key(*k)));
            assert!(typed, "{id} {side}: {schema}");
        }
    }
}

#[test]
fn the_servers_list_schema_names_every_field_the_page_reads() {
    let (_, _, response) = catalog()
        .into_iter()
        .find(|(id, ..)| *id == SERVERS_LIST)
        .unwrap();
    let text = response.to_string();
    for field in [
        // ServersList
        "stdio_enabled",
        "callback_url",
        "servers",
        // ServerView
        "name",
        "transport",
        "target",
        "auth",
        "state",
        "stage",
        "since",
        "error",
        "approved",
        "fingerprint",
        "protocol_version",
        "server_info",
        "tools_listed",
        "tools",
        "skipped",
        "has_instructions",
        "capabilities",
        "resource_functions",
        "prompts",
        "list_errors",
        "login",
        "process",
    ] {
        assert!(text.contains(&format!("\"{field}\"")), "{field}");
    }
}

#[test]
fn registration_flags_carry_the_security_rules() {
    const NOT_OBSERVABLE: [&str; 4] = [
        "mcp-client::process::logs",
        "mcp-client::login::start",
        "mcp-client::login::complete",
        "mcp-client::oauth-callback",
    ];
    for (id, ..) in catalog() {
        let f = flags(id);
        assert!(f.internal, "{id}");
        assert_eq!(f.observable, !NOT_OBSERVABLE.contains(&id), "{id}");
        assert_eq!(f.private, id == ON_SERVERS_CHANGED, "{id}");
    }
}

/// The keys of a schema's `properties`, in serde_json order (sorted).
fn property_keys(schema: &Value) -> Vec<&str> {
    let properties = schema["properties"].as_object();
    properties.map_or(vec![], |p| p.keys().map(String::as_str).collect())
}

#[test]
fn login_and_callback_functions_have_typed_schemas() {
    type Keys = Option<&'static [&'static str]>;
    let rows: [(&str, Keys, Keys); 5] = [
        (
            "mcp-client::login::start",
            Some(&["server"]),
            Some(&["authorize_url", "expires_at", "redirect_uri"]),
        ),
        (
            "mcp-client::login::complete",
            Some(&["callback_url"]),
            Some(&["server", "state"]),
        ),
        ("mcp-client::login::cancel", Some(&["server"]), None),
        ("mcp-client::login::sign-out", None, Some(&["state"])),
        (
            "mcp-client::oauth-callback",
            Some(&["query_params"]),
            Some(&["body", "headers", "status_code"]),
        ),
    ];
    let catalog = catalog();
    for (id, request, response) in rows {
        let found = catalog.iter().find(|(listed, ..)| *listed == id);
        let (_, request_schema, response_schema) =
            found.unwrap_or_else(|| panic!("{id} missing from the catalog"));
        for (side, keys, schema) in [
            ("request", request, request_schema),
            ("response", response, response_schema),
        ] {
            if let Some(keys) = keys {
                assert_eq!(property_keys(schema), keys, "{id} {side}");
            }
        }
    }
}
