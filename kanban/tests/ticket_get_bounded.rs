//! `kanban::ticket::get` bounds its activity; storage and `activity::list` do not.

use iii_kanban::board::{
    ActivityFilter, BoardContext, CreateCommentInput, CreateTicketInput, UpdatePatch,
    create_comment, create_ticket, find_ticket, list_activity, ticket_view, update_ticket,
};
use iii_kanban::config::Column;
use iii_kanban::functions::bound_for_get;
use iii_kanban::store::Board;
use serde_json::Value;

fn context() -> BoardContext {
    BoardContext {
        prefix: "KAN".into(),
        default_status: "backlog".into(),
        done_status: "done".into(),
        canceled_status: None,
        columns: [("backlog", "Backlog"), ("done", "Done")]
            .into_iter()
            .map(|(id, label)| Column {
                id: id.into(),
                label: label.into(),
            })
            .collect(),
        priorities: ["low", "medium", "high", "urgent"]
            .into_iter()
            .map(String::from)
            .collect(),
    }
}

fn at(n: u32) -> String {
    format!("2026-01-01T00:{:02}:{:02}.000Z", n / 60, n % 60)
}

fn edit(board: &mut Board, n: u32, text: String) {
    let patch = UpdatePatch {
        description: Some(text),
        ..UpdatePatch::default()
    };
    update_ticket(board, "KAN-1", &patch, "user", &context(), &at(n)).unwrap();
}

fn board_with_ticket() -> Board {
    let mut board = Board::empty();
    let input = CreateTicketInput {
        title: "Long".into(),
        ..CreateTicketInput::default()
    };
    create_ticket(&mut board, &input, &context(), &at(0)).unwrap();
    board
}

fn get(board: &Board) -> Value {
    let ticket = find_ticket(board, "KAN-1").unwrap();
    serde_json::to_value(bound_for_get(ticket_view(board, ticket, "done", None))).unwrap()
}

#[test]
fn fifty_description_edits_stay_far_under_the_cap() {
    let mut board = board_with_ticket();
    for n in 1..=50 {
        edit(&mut board, n, format!("{n}{}", "x".repeat(5000)));
    }
    let size = serde_json::to_string(&get(&board)).unwrap().len();
    assert!(size < 64 * 1024, "{size} bytes");
}

#[test]
fn truncated_values_carry_the_original_length_and_the_list_keeps_full_text() {
    let mut board = board_with_ticket();
    edit(&mut board, 1, "a".repeat(4506));
    let body = get(&board);
    let to = body["activity"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["changes"].as_array().into_iter().flatten())
        .find(|c| c["field"] == "description")
        .unwrap()["to"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(to.contains("truncated, 4506 chars"), "{to}");
    assert!(to.len() < 700);
    let full = list_activity(
        &board,
        &ActivityFilter {
            ticket_id: "KAN-1".into(),
            ..ActivityFilter::default()
        },
    );
    let kept = full
        .iter()
        .flat_map(|e| e.changes.iter().flatten())
        .find(|c| c.field == "description")
        .unwrap();
    assert_eq!(kept.to.as_str().unwrap().len(), 4506);
    // The stored ticket is untouched too.
    assert_eq!(
        find_ticket(&board, "KAN-1").unwrap().description.len(),
        4506
    );
}

#[test]
fn activity_is_capped_to_the_newest_and_comments_stay_complete() {
    let mut board = board_with_ticket();
    let comment = CreateCommentInput {
        ticket_id: "KAN-1".into(),
        body: "first".into(),
        ..CreateCommentInput::default()
    };
    create_comment(&mut board, &comment, &at(1)).unwrap();
    for n in 2..=80 {
        edit(&mut board, n, format!("v{n}"));
    }
    let total = find_ticket(&board, "KAN-1").unwrap().activity.len() as u64;
    let body = get(&board);
    let activity = body["activity"].as_array().unwrap();
    assert_eq!(body["activity_total"], total);
    assert_eq!(
        activity
            .iter()
            .filter(|e| e["type"] != "comment.created")
            .count(),
        50
    );
    assert!(activity.iter().any(|e| e["type"] == "comment.created"));
    assert_eq!(body["comments"].as_array().unwrap().len(), 1);
}

#[test]
fn a_short_ticket_is_returned_unchanged() {
    let mut board = board_with_ticket();
    edit(&mut board, 1, "short".into());
    let ticket = find_ticket(&board, "KAN-1").unwrap();
    let view = ticket_view(&board, ticket, "done", None);
    let mut expected = serde_json::to_value(&view).unwrap();
    expected["activity_total"] = (ticket.activity.len() as u64).into();
    assert_eq!(get(&board), expected);
}
