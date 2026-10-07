//! Descriptor validation, metadata round trip and the wire shapes.

use mention_contract::{
    MentionField, MentionOpen, MentionProvider, MentionSearchRequest, MentionView, ProviderError,
    DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT, METADATA_KEY,
};
use serde_json::json;

fn kanban() -> MentionProvider {
    MentionProvider::new("kanban", "Tickets", "kanban::mention::search")
        .description("Kanban ticket")
        .icon("ticket")
        .color("blue")
        .details("kanban::ticket::get", "id")
}

#[test]
fn metadata_round_trips_and_marks_the_function_internal() {
    let provider = kanban();
    let metadata = provider.metadata();
    assert_eq!(metadata["internal"], json!(true));
    assert_eq!(metadata["trace_hidden"], json!(true));
    assert_eq!(metadata[METADATA_KEY]["name"], json!("kanban"));
    assert_eq!(MentionProvider::from_metadata(&metadata), Some(provider));
}

#[test]
fn from_metadata_ignores_functions_without_a_descriptor() {
    assert_eq!(
        MentionProvider::from_metadata(&json!({ "internal": true })),
        None
    );
    assert_eq!(MentionProvider::from_metadata(&json!(null)), None);
}

#[test]
fn from_metadata_tolerates_unknown_fields_and_defaults() {
    let metadata = json!({
        "mention": {
            "name": "cal",
            "label": "Events",
            "search": "cal::mention::search",
            "details": { "function_id": "cal::event::get" },
            "future_field": 1
        }
    });
    let provider = MentionProvider::from_metadata(&metadata).expect("parses");
    assert_eq!(provider.v, 1);
    assert_eq!(provider.details.expect("details").id_field, "id");
}

#[test]
fn validate_rejects_bad_names_and_empty_fields() {
    let mut provider = kanban();
    provider.name = "fn".into();
    assert_eq!(
        provider.validate(),
        Err(ProviderError::ReservedName("fn".into()))
    );
    provider.name = "Kanban".into();
    assert!(matches!(
        provider.validate(),
        Err(ProviderError::InvalidName(_))
    ));
    provider.name = "kanban".into();
    provider.search = " ".into();
    assert_eq!(
        provider.validate(),
        Err(ProviderError::EmptyField("search"))
    );
    // A rejected descriptor is invisible to consumers.
    assert_eq!(MentionProvider::from_metadata(&provider.metadata()), None);
}

#[test]
fn details_payload_uses_the_declared_field() {
    let provider = MentionProvider::new("session", "Sessions", "session::mention::search")
        .details("session::get", "session_id");
    assert_eq!(
        provider.details.expect("details").payload("s_1"),
        json!({ "session_id": "s_1" })
    );
}

#[test]
fn search_limit_defaults_and_clamps() {
    let mut request = MentionSearchRequest::default();
    assert_eq!(request.effective_limit(), DEFAULT_SEARCH_LIMIT);
    request.limit = Some(0);
    assert_eq!(request.effective_limit(), 1);
    request.limit = Some(10_000);
    assert_eq!(request.effective_limit(), MAX_SEARCH_LIMIT);
    let parsed: MentionSearchRequest = serde_json::from_value(json!({})).expect("empty ok");
    assert_eq!(parsed.query, "");
}

fn view() -> MentionView {
    MentionView {
        id: "6ac4".into(),
        label: "Fix login redirect".into(),
        hint: Some("KAN-12".into()),
        description: Some("In progress".into()),
        icon: None,
        color: None,
        fields: vec![MentionField::new("priority", "high").tone("warning")],
        open: Some(MentionOpen::Page {
            page: "kanban-ticket".into(),
            context: Some(json!({ "id": "KAN-12" })),
        }),
        summary: None,
        data: None,
        updated_at: None,
    }
}

#[test]
fn agent_summary_falls_back_to_the_display_fields() {
    assert_eq!(
        view().agent_summary(),
        r#"KAN-12 "Fix login redirect" — In progress · priority: high"#
    );
    let mut written = view();
    written.summary = Some("  custom line ".into());
    assert_eq!(written.agent_summary(), "custom line");
}

#[test]
fn open_serializes_as_one_keyed_object() {
    assert_eq!(
        serde_json::to_value(view().open).expect("serializes"),
        json!({ "page": "kanban-ticket", "context": { "id": "KAN-12" } })
    );
    let session: MentionOpen = serde_json::from_value(json!({ "session": "s_1" })).expect("ok");
    assert_eq!(
        session,
        MentionOpen::Session {
            session: "s_1".into()
        }
    );
    let url: MentionOpen = serde_json::from_value(json!({ "url": "https://x" })).expect("ok");
    assert_eq!(
        url,
        MentionOpen::Url {
            url: "https://x".into()
        }
    );
}

#[test]
fn view_omits_empty_optionals() {
    let minimal = MentionView {
        id: "1".into(),
        label: "x".into(),
        hint: None,
        description: None,
        icon: None,
        color: None,
        fields: Vec::new(),
        open: None,
        summary: None,
        data: None,
        updated_at: None,
    };
    assert_eq!(
        serde_json::to_value(minimal).expect("serializes"),
        json!({ "id": "1", "label": "x" })
    );
}
