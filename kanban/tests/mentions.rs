//! The `@kanban(id=…)` chat mention provider: ticket search ranking, the
//! view a chat surface renders, and the descriptor in the get function's
//! metadata — engine-free.

use iii_kanban::board::{
    BoardContext, CreateTicketInput, create_ticket, delete_ticket, search_tickets,
};
use iii_kanban::config::{Column, KanbanConfig};
use iii_kanban::functions::{self, mention_search_metadata};
use iii_kanban::mentions::{self, GET_FN, SEARCH_FN};
use iii_kanban::store::{Board, Ticket};
use mention_contract::{MentionOpen, MentionProvider, MentionSearchRequest, color};
use serde_json::json;

fn context() -> BoardContext {
    BoardContext {
        prefix: "KAN".into(),
        default_status: "backlog".into(),
        columns: config().columns,
        priorities: config().priorities,
    }
}

fn config() -> KanbanConfig {
    KanbanConfig {
        columns: [("backlog", "Backlog"), ("in_progress", "In progress")]
            .into_iter()
            .map(|(id, label)| Column {
                id: id.into(),
                label: label.into(),
            })
            .collect(),
        ..KanbanConfig::default()
    }
}

fn at(second: u32) -> String {
    format!("2026-01-01T00:00:{second:02}.000Z")
}

fn create(board: &mut Board, input: CreateTicketInput, second: u32) -> Ticket {
    create_ticket(board, &input, &context(), &at(second)).expect("created")
}

fn titled(title: &str) -> CreateTicketInput {
    CreateTicketInput {
        title: title.into(),
        ..CreateTicketInput::default()
    }
}

/// KAN-1 "Fix login redirect", KAN-2 "Login page polish", KAN-3 "Database
/// migration" (mentions login in its description), oldest first.
fn board() -> Board {
    let mut board = Board::empty();
    create(&mut board, titled("Fix login redirect"), 1);
    create(&mut board, titled("Login page polish"), 2);
    create(
        &mut board,
        CreateTicketInput {
            description: Some("Touches the login flow".into()),
            ..titled("Database migration")
        },
        3,
    );
    board
}

fn keys(board: &Board, query: &str) -> Vec<String> {
    search_tickets(board, query, 10)
        .into_iter()
        .map(|ticket| ticket.key.clone())
        .collect()
}

#[test]
fn search_puts_keys_first_in_every_spelling() {
    let board = board();
    for query in ["KAN-2", "kan-2", "2", "#2"] {
        assert_eq!(keys(&board, query)[0], "KAN-2", "query {query:?}");
    }
}

#[test]
fn search_ranks_title_prefix_then_word_prefix_then_description() {
    let board = board();
    assert_eq!(keys(&board, "login"), ["KAN-2", "KAN-1", "KAN-3"]);
    assert_eq!(keys(&board, "fix log"), ["KAN-1"]);
    assert_eq!(keys(&board, "redirect fix"), ["KAN-1"]);
    assert!(keys(&board, "nothing like this").is_empty());
}

#[test]
fn an_empty_query_lists_the_most_recently_updated_first() {
    assert_eq!(keys(&board(), "  "), ["KAN-3", "KAN-2", "KAN-1"]);
}

#[test]
fn search_skips_deleted_tickets_and_honours_the_limit() {
    let mut board = board();
    delete_ticket(&mut board, "KAN-2", "user", &at(9));
    assert_eq!(keys(&board, "login"), ["KAN-1", "KAN-3"]);
    assert_eq!(search_tickets(&board, "", 1).len(), 1);
}

#[test]
fn search_rows_carry_the_uuid_key_and_state() {
    let board = board();
    let items = mentions::search(
        &board,
        &config(),
        &MentionSearchRequest {
            query: "kan-1".into(),
            limit: Some(1),
            context: None,
        },
    );
    assert_eq!(items.len(), 1);
    let ticket = &board.tickets.values().find(|t| t.key == "KAN-1").unwrap();
    assert_eq!(items[0].id, ticket.id);
    assert_eq!(items[0].hint.as_deref(), Some("KAN-1"));
    assert_eq!(items[0].label, "Fix login redirect");
    assert_eq!(items[0].description.as_deref(), Some("Backlog · low"));
}

#[test]
fn view_renders_status_label_priority_and_open_target() {
    let mut board = Board::empty();
    let ticket = create(
        &mut board,
        CreateTicketInput {
            status: Some("in_progress".into()),
            priority: Some("urgent".into()),
            assignee: Some("coder".into()),
            labels: Some(vec!["auth".into()]),
            ..titled("Fix login redirect")
        },
        1,
    );
    let view = mentions::view(&ticket, &config());
    assert_eq!(view.id, ticket.id);
    assert_eq!(view.hint.as_deref(), Some("KAN-1"));
    assert_eq!(view.description.as_deref(), Some("In progress"));
    assert_eq!(view.color.as_deref(), Some(color::ROSE));
    let fields: Vec<(&str, &str)> = view
        .fields
        .iter()
        .map(|f| (f.label.as_str(), f.value.as_str()))
        .collect();
    assert_eq!(
        fields,
        [
            ("Status", "In progress"),
            ("Priority", "urgent"),
            ("Assignee", "coder"),
            ("Labels", "auth"),
        ]
    );
    assert_eq!(
        view.open,
        Some(MentionOpen::Page {
            page: "kanban-ticket".into(),
            context: Some(json!({ "id": "KAN-1" })),
        })
    );
    assert_eq!(
        view.summary.as_deref(),
        Some(
            r#"Kanban ticket KAN-1 "Fix login redirect" · status: In progress · priority: urgent · assignee: coder · labels: auth"#
        )
    );
    let data = view.data.as_ref().unwrap();
    assert_eq!(data["key"], json!("KAN-1"));
    assert_eq!(data["comment_count"], json!(0));
    assert_eq!(data["description"], json!(""));
    assert!(data.get("comments").is_none());
}

#[test]
fn the_preview_carries_only_the_start_of_a_long_description() {
    let mut board = Board::empty();
    let ticket = create(
        &mut board,
        CreateTicketInput {
            description: Some("é".repeat(400)),
            ..titled("Long")
        },
        1,
    );
    let data = mentions::view(&ticket, &config()).data.unwrap();
    let description = data["description"].as_str().unwrap();
    assert_eq!(description.chars().count(), 281);
    assert!(description.ends_with('…'));
}

#[test]
fn a_deleted_ticket_still_resolves_but_says_so() {
    let mut board = board();
    let deleted = delete_ticket(&mut board, "KAN-1", "user", &at(9)).unwrap();
    let view = mentions::view(&deleted, &config());
    assert_eq!(view.color.as_deref(), Some(color::NEUTRAL));
    assert!(view.fields.iter().any(|f| f.label == "Deleted"));
    assert!(view.summary.unwrap().ends_with("· deleted"));
}

#[test]
fn the_get_function_declares_the_provider() {
    let provider = MentionProvider::from_metadata(&mentions::provider().metadata())
        .expect("descriptor validates");
    assert_eq!(provider.name, "kanban");
    assert_eq!(provider.search, SEARCH_FN);
    let details = provider.details.expect("details");
    assert_eq!(details.function_id, "kanban::ticket::get");
    assert!(functions::FUNCTION_IDS.contains(&details.function_id.as_str()));
    assert!(functions::FUNCTION_IDS.contains(&SEARCH_FN));
    assert!(functions::FUNCTION_IDS.contains(&GET_FN));
    // The search function is plumbing too: no descriptor, hidden from agents.
    let search = mention_search_metadata();
    assert_eq!(search["internal"], json!(true));
    assert_eq!(MentionProvider::from_metadata(&search), None);
}
