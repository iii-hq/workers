//! The mention hook's decisions and the text agents read — engine-free.

use judge::mentions::registry::{parse_providers, Provider};
use judge::mentions::render::{
    advertised_providers, already_resolved, mentions_block, pending_mentions, providers_block,
    DetailsCall, MentionStatus, ResolvedMention,
};
use judge::mentions::{plan, BLOCK_BUDGET_CHARS};
use mention_contract::MentionProvider;
use serde_json::{json, Value};

fn user(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

fn assistant(text: &str) -> Value {
    json!({ "role": "assistant", "content": [{ "type": "text", "text": text }] })
}

fn providers() -> Vec<Provider> {
    parse_providers(&json!({
        "functions": [
            {
                "function_id": "session::mention::get",
                "metadata": MentionProvider::new("session", "Sessions", "session::mention::search")
                    .description("A chat session")
                    .details("session::get", "session_id")
                    .metadata()
            },
            {
                "function_id": "kanban::mention::get",
                "metadata": MentionProvider::new("kanban", "Tickets", "kanban::mention::search")
                    .description("A kanban ticket")
                    .details("kanban::ticket::get", "id")
                    .metadata()
            },
            { "function_id": "kanban::ticket::get", "metadata": {} },
            {
                "function_id": "zz::mention::get",
                "metadata": MentionProvider::new("kanban", "Impostor", "zz::search").metadata()
            }
        ]
    }))
}

fn resolved(id: &str) -> ResolvedMention {
    ResolvedMention {
        token: format!("@kanban(id=\"{id}\")"),
        name: "kanban".into(),
        id: id.into(),
        status: MentionStatus::Resolved,
        label: Some("Tickets".into()),
        summary: Some(format!("Kanban ticket {id} \"Fix login\"")),
        details: Some(DetailsCall {
            function_id: "kanban::ticket::get".into(),
            payload: json!({ "id": id }),
        }),
        error: None,
    }
}

#[test]
fn providers_are_sorted_and_a_contested_name_goes_to_the_first_function_id() {
    let providers = providers();
    let names: Vec<(&str, &str)> = providers
        .iter()
        .map(|p| (p.descriptor.name.as_str(), p.get_function_id.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("kanban", "kanban::mention::get"),
            ("session", "session::mention::get")
        ]
    );
}

#[test]
fn pending_mentions_come_from_what_users_wrote_only() {
    let messages = vec![
        user(r#"look at @kanban(id="1") and `@kanban(id="code")`"#),
        assistant(r#"Sure, @kanban(id="2") too"#),
        json!({ "role": "user", "content": [
            { "type": "text", "text": "and @session(id=\"s_1\")" },
            { "type": "text", "text": "<attached-file path=\"a.md\">@kanban(id=\"file\")</attached-file>" }
        ]}),
        user(r#"again @kanban(id="1")"#),
    ];
    let pending = pending_mentions(&messages, &Default::default(), 10);
    let ids: Vec<&str> = pending.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["1", "s_1"]);
}

#[test]
fn the_newest_mentions_win_when_there_are_too_many() {
    let text: String = (0..5).map(|i| format!("@kanban(id=\"{i}\") ")).collect();
    let pending = pending_mentions(&[user(&text)], &Default::default(), 2);
    let ids: Vec<&str> = pending.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["3", "4"]);
}

#[test]
fn an_earlier_block_marks_its_mentions_resolved() {
    let block = mentions_block(&[resolved("1")], BLOCK_BUDGET_CHARS);
    let messages = vec![
        user(r#"look at @kanban(id="1")"#),
        user(&block),
        user(r#"and now @kanban(id="2")"#),
    ];
    let skip = already_resolved(&messages);
    assert!(skip.contains(&("kanban".to_string(), "1".to_string())));
    let pending = pending_mentions(&messages, &skip, 10);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "2");
}

#[test]
fn the_plan_advertises_once_and_again_when_the_set_changes() {
    let providers = providers();
    let first = plan(&[user("hello")], &providers);
    assert!(first.advertise);
    assert!(first.pending.is_empty());

    let index = providers_block(&providers);
    let advertised = vec![user("hello"), user(&index)];
    assert_eq!(
        advertised_providers(&advertised).map(|set| set.into_iter().collect::<Vec<_>>()),
        Some(vec!["kanban".to_string(), "session".to_string()])
    );
    assert!(!plan(&advertised, &providers).advertise);
    assert!(plan(&advertised, &providers[..1]).advertise);
    assert!(!plan(&[user("hello")], &[]).advertise);
}

#[test]
fn the_index_and_the_block_read_as_the_prompt_describes() {
    let index = providers_block(&providers());
    assert!(index.starts_with("<mention_providers>\n"));
    assert!(index.contains("- @kanban — Tickets: A kanban ticket\n"));
    assert!(index.ends_with("</mention_providers>"));

    let block = mentions_block(
        &[
            resolved("6ac4"),
            ResolvedMention {
                status: MentionStatus::NotFound,
                summary: None,
                details: None,
                ..resolved("gone")
            },
            ResolvedMention {
                token: "@calendar(id=\"e1\")".into(),
                name: "calendar".into(),
                id: "e1".into(),
                status: MentionStatus::UnknownProvider,
                label: None,
                summary: None,
                details: None,
                error: None,
            },
        ],
        BLOCK_BUDGET_CHARS,
    );
    assert!(block.starts_with("<mentions>\n"));
    assert!(block.contains("pre-verified"));
    assert!(block.contains(
        "- @kanban(id=\"6ac4\") — Kanban ticket 6ac4 \"Fix login\"\n  details: kanban::ticket::get {\"id\":\"6ac4\"}\n"
    ));
    assert!(block.contains("- @kanban(id=\"gone\") — not found"));
    assert!(
        block.contains("- @calendar(id=\"e1\") — no installed worker provides @calendar mentions")
    );
    assert!(block.ends_with("</mentions>"));
}

#[test]
fn the_block_keeps_to_its_budget_and_counts_what_it_left_out() {
    let many: Vec<ResolvedMention> = (0..40).map(|i| resolved(&format!("id-{i}"))).collect();
    let block = mentions_block(&many, 1_000);
    assert!(block.len() < 1_200);
    assert!(block.contains("more mention(s) not shown"));
}
