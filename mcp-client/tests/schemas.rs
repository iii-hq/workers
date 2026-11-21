use mcp_client::functions::{CONFIG_VALIDATE, ON_SERVERS_CHANGED, SERVERS_LIST, catalog, flags};

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
