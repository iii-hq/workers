//! Behaviour of the board operations, the store round-trip, the trigger
//! filters and the function catalog — engine-free.

use iii_kanban::board::{
    ActivityFilter, BoardContext, CommentFilter, CreateCommentInput, CreateTicketInput,
    TicketFilter, UpdatePatch, create_comment, create_ticket, delete_ticket, find_ticket,
    list_activity, list_comments, list_tickets, move_ticket, restore_ticket, summarize,
    update_ticket,
};
use iii_kanban::config::{Column, KanbanConfig, normalize};
use iii_kanban::events::{
    ChangeConfig, ChangeEvent, CommentEvent, matches_change, matches_comment, matches_ticket,
    payloads, wants_summary,
};
use iii_kanban::functions::{
    self, BoardGetInput, BoardView, CommentCreateInput, ConfigInfo, TicketCreateInput,
    TicketUpdateInput, board_view,
};
use iii_kanban::store::{Board, Comment, Ticket, read_board, write_board};
use serde_json::{Value, json};

fn context() -> BoardContext {
    BoardContext {
        prefix: "KAN".into(),
        default_status: "backlog".into(),
        columns: [("backlog", "Backlog"), ("todo", "To do"), ("done", "Done")]
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

fn at(second: u32) -> String {
    format!("2026-01-01T00:00:{second:02}.000Z")
}

fn titled(title: &str) -> CreateTicketInput {
    CreateTicketInput {
        title: title.into(),
        ..CreateTicketInput::default()
    }
}

fn comment(
    ticket: &str,
    body: &str,
    author: Option<&str>,
    parent: Option<&str>,
) -> CreateCommentInput {
    CreateCommentInput {
        ticket_id: ticket.into(),
        body: body.into(),
        parent_id: parent.map(String::from),
        author: author.map(String::from),
    }
}

fn ticket(board: &Board, id: &str) -> Ticket {
    find_ticket(board, id).cloned().expect("ticket exists")
}

#[test]
fn create_ticket_numbers_tickets_and_honours_the_default_column() {
    let mut board = Board::empty();
    let first = create_ticket(&mut board, &titled("First"), &context(), &at(1)).unwrap();
    let second = create_ticket(
        &mut board,
        &CreateTicketInput {
            status: Some("todo".into()),
            ..titled("Second")
        },
        &context(),
        &at(2),
    )
    .unwrap();
    assert_eq!(first.key, "KAN-1");
    assert_eq!(second.key, "KAN-2");
    assert_eq!(first.status, "backlog");
    assert_eq!(second.status, "todo");
    assert_eq!(
        first.priority, "low",
        "the first configured priority is the default"
    );
    assert_eq!(first.activity[0].kind, "ticket.created");
    assert_eq!(board.next_number, 3);
}

#[test]
fn create_ticket_rejects_an_unknown_column_and_an_empty_title() {
    let mut board = Board::empty();
    let err = create_ticket(
        &mut board,
        &CreateTicketInput {
            status: Some("nope".into()),
            ..titled("x")
        },
        &context(),
        &at(1),
    )
    .unwrap_err();
    assert!(err.contains("unknown status"));
    assert_eq!(
        create_ticket(&mut board, &titled("   "), &context(), &at(1)).unwrap_err(),
        "title is required"
    );
    assert!(board.tickets.is_empty());
}

#[test]
fn find_ticket_resolves_the_uuid_and_the_key_case_insensitively() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Lookup"), &context(), &at(1)).unwrap();
    assert_eq!(find_ticket(&board, &created.id).unwrap().id, created.id);
    assert_eq!(find_ticket(&board, &created.key).unwrap().id, created.id);
    assert_eq!(find_ticket(&board, "kan-1").unwrap().id, created.id);
    assert!(find_ticket(&board, "KAN-404").is_none());
    assert!(find_ticket(&board, "  ").is_none());
}

#[test]
fn move_ticket_reindexes_the_target_column_and_logs_only_a_lane_change() {
    let mut board = Board::empty();
    let a = create_ticket(&mut board, &titled("A"), &context(), &at(1)).unwrap();
    let b = create_ticket(&mut board, &titled("B"), &context(), &at(2)).unwrap();

    let before = a.activity.len();
    move_ticket(
        &mut board,
        &a.id,
        "backlog",
        Some(1),
        None,
        &context(),
        &at(3),
    )
    .unwrap();
    assert_eq!(ticket(&board, &a.id).order, 2);
    assert_eq!(ticket(&board, &b.id).order, 1);
    assert_eq!(
        ticket(&board, &a.id).activity.len(),
        before,
        "reordering inside a lane adds no timeline entry"
    );

    let moved = move_ticket(
        &mut board,
        &a.id,
        "done",
        None,
        Some("agent"),
        &context(),
        &at(4),
    )
    .unwrap()
    .unwrap();
    assert_eq!(moved.status, "done");
    let last = moved.activity.last().unwrap();
    assert_eq!(last.kind, "ticket.moved");
    assert_eq!(last.actor, "agent");
    assert_eq!(
        serde_json::to_value(last.changes.as_ref().unwrap()).unwrap(),
        json!([{ "field": "status", "from": "backlog", "to": "done" }])
    );
    assert_eq!(
        ticket(&board, &b.id).order,
        1,
        "the source column is compacted"
    );
    assert!(move_ticket(&mut board, &a.id, "ghost", None, None, &context(), &at(5)).is_err());
    assert!(
        move_ticket(&mut board, "KAN-9", "done", None, None, &context(), &at(5))
            .unwrap()
            .is_none()
    );
}

#[test]
fn update_ticket_records_changes_and_rejects_bad_input() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Edit me"), &context(), &at(1)).unwrap();
    let result = update_ticket(
        &mut board,
        &created.key,
        &UpdatePatch {
            title: Some("Edited".into()),
            priority: Some("high".into()),
            assignee: Some(Some("console-ui".into())),
            ..UpdatePatch::default()
        },
        "agent",
        &context(),
        &at(2),
    )
    .unwrap()
    .unwrap();
    assert_eq!(result.ticket.title, "Edited");
    assert_eq!(result.ticket.assignee.as_deref(), Some("console-ui"));
    assert_eq!(result.changes.len(), 3);
    assert_eq!(result.ticket.updated_at, at(2));
    assert_eq!(
        result.ticket.activity.last().unwrap().kind,
        "ticket.updated"
    );

    let unassigned = update_ticket(
        &mut board,
        &created.id,
        &UpdatePatch {
            assignee: Some(None),
            ..UpdatePatch::default()
        },
        "agent",
        &context(),
        &at(3),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        unassigned.ticket.assignee, None,
        "an explicit null unassigns"
    );

    let unchanged = update_ticket(
        &mut board,
        &created.id,
        &UpdatePatch {
            title: Some("Edited".into()),
            ..UpdatePatch::default()
        },
        "agent",
        &context(),
        &at(4),
    )
    .unwrap()
    .unwrap();
    assert!(unchanged.changes.is_empty());
    assert_eq!(
        unchanged.ticket.updated_at,
        at(3),
        "a no-op update is not recorded"
    );

    let bad_status = update_ticket(
        &mut board,
        &created.id,
        &UpdatePatch {
            status: Some("ghost".into()),
            ..UpdatePatch::default()
        },
        "agent",
        &context(),
        &at(5),
    );
    assert!(bad_status.unwrap_err().contains("unknown status"));
    let empty_title = update_ticket(
        &mut board,
        &created.id,
        &UpdatePatch {
            title: Some("   ".into()),
            ..UpdatePatch::default()
        },
        "agent",
        &context(),
        &at(5),
    );
    assert_eq!(empty_title.unwrap_err(), "title cannot be empty");
    assert!(
        update_ticket(
            &mut board,
            "KAN-9",
            &UpdatePatch::default(),
            "agent",
            &context(),
            &at(5)
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn a_soft_delete_leaves_the_row_on_the_board_and_can_be_restored() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Doomed"), &context(), &at(1)).unwrap();
    let deleted = delete_ticket(&mut board, &created.key, "user", &at(2)).unwrap();
    assert_eq!(deleted.deleted_at.as_deref(), Some(at(2).as_str()));
    assert!(
        find_ticket(&board, &created.key).is_some(),
        "the row is still addressable"
    );
    assert!(
        list_tickets(&board, &TicketFilter::default()).is_empty(),
        "it leaves the board"
    );
    assert_eq!(
        list_tickets(
            &board,
            &TicketFilter {
                include_deleted: true,
                ..TicketFilter::default()
            }
        )
        .len(),
        1
    );
    assert_eq!(deleted.activity.last().unwrap().kind, "ticket.deleted");
    let again = delete_ticket(&mut board, &created.id, "user", &at(3)).unwrap();
    assert_eq!(
        again.activity.len(),
        2,
        "deleting twice records nothing new"
    );

    let restored = restore_ticket(&mut board, &created.id, "user", &at(4)).unwrap();
    assert_eq!(restored.deleted_at, None);
    assert_eq!(list_tickets(&board, &TicketFilter::default()).len(), 1);
    assert_eq!(restored.activity.last().unwrap().kind, "ticket.restored");
}

#[test]
fn replies_attach_to_the_root_comment_and_stay_one_level_deep() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Threads"), &context(), &at(1)).unwrap();
    let root = create_comment(
        &mut board,
        &comment(&created.key, "root", Some("user"), None),
        &at(2),
    )
    .unwrap()
    .unwrap();
    let reply = create_comment(
        &mut board,
        &comment(
            &created.id,
            "reply",
            Some("console-ui"),
            Some(&root.comment.id),
        ),
        &at(3),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        reply.comment.parent_id.as_deref(),
        Some(root.comment.id.as_str())
    );

    let nested = create_comment(
        &mut board,
        &comment(
            &created.id,
            "nested",
            Some("console-ui"),
            Some(&reply.comment.id),
        ),
        &at(4),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        nested.comment.parent_id.as_deref(),
        Some(root.comment.id.as_str()),
        "a reply to a reply joins the root thread"
    );
    assert_eq!(nested.ticket.comments.len(), 3);
    let last = nested.ticket.activity.last().unwrap();
    assert_eq!(last.kind, "comment.created");
    assert_eq!(last.comment_id.as_deref(), Some(nested.comment.id.as_str()));
}

#[test]
fn create_comment_refuses_an_unknown_parent_and_a_deleted_ticket() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Guards"), &context(), &at(1)).unwrap();
    let err = create_comment(
        &mut board,
        &comment(&created.id, "x", None, Some("missing")),
        &at(2),
    )
    .unwrap_err();
    assert!(err.contains("unknown comment"));
    assert_eq!(
        create_comment(&mut board, &comment(&created.id, "   ", None, None), &at(2)).unwrap_err(),
        "comment body is required"
    );
    assert!(
        create_comment(&mut board, &comment("KAN-9", "x", None, None), &at(2))
            .unwrap()
            .is_none()
    );
    delete_ticket(&mut board, &created.id, "user", &at(3));
    let err = create_comment(
        &mut board,
        &comment(&created.id, "late", None, None),
        &at(4),
    )
    .unwrap_err();
    assert!(err.contains("deleted ticket"));
}

#[test]
fn list_comments_filters_by_author_thread_and_time() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Filtered"), &context(), &at(1)).unwrap();
    for (second, body, author) in [
        (2, "mine", "console-ui"),
        (3, "theirs", "user"),
        (4, "later mine", "console-ui"),
    ] {
        create_comment(
            &mut board,
            &comment(&created.id, body, Some(author), None),
            &at(second),
        )
        .unwrap();
    }
    let bodies = |filter: CommentFilter| -> Vec<String> {
        list_comments(&board, &filter)
            .into_iter()
            .map(|c: Comment| c.body)
            .collect()
    };
    let base = || CommentFilter {
        ticket_id: created.id.clone(),
        ..CommentFilter::default()
    };
    assert_eq!(bodies(base()).len(), 3);
    assert_eq!(
        bodies(CommentFilter {
            author: Some("console-ui".into()),
            ..base()
        })
        .len(),
        2
    );
    assert_eq!(
        bodies(CommentFilter {
            exclude_author: Some("console-ui".into()),
            ..base()
        }),
        vec!["theirs"]
    );
    assert_eq!(
        bodies(CommentFilter {
            author: Some("console-ui".into()),
            since: Some(at(3)),
            ..base()
        }),
        vec!["later mine"]
    );
    assert_eq!(
        bodies(CommentFilter {
            limit: Some(1),
            ..base()
        }),
        vec!["mine"]
    );
    assert!(
        bodies(CommentFilter {
            ticket_id: "KAN-999".into(),
            ..CommentFilter::default()
        })
        .is_empty()
    );
    let activity = list_activity(
        &board,
        &ActivityFilter {
            ticket_id: created.key.clone(),
            types: Some(vec!["comment.created".into()]),
            since: Some(at(2)),
            limit: None,
        },
    );
    assert_eq!(activity.len(), 2);
    assert_eq!(activity[0].seq, 3);
}

#[test]
fn summarize_carries_the_board_card_fields() {
    let mut board = Board::empty();
    let created = create_ticket(
        &mut board,
        &CreateTicketInput {
            assignee: Some("iii".into()),
            labels: Some(vec![" feature ".into(), "".into()]),
            ..titled("Card")
        },
        &context(),
        &at(1),
    )
    .unwrap();
    create_comment(&mut board, &comment(&created.id, "hi", None, None), &at(2)).unwrap();
    let summary = summarize(&ticket(&board, &created.id));
    assert_eq!(summary.comment_count, 1);
    assert_eq!(summary.assignee.as_deref(), Some("iii"));
    assert_eq!(summary.key, "KAN-1");
    assert_eq!(summary.labels, vec!["feature"]);
}

#[test]
fn list_tickets_filters_by_column_and_assignee() {
    let mut board = Board::empty();
    let a = create_ticket(
        &mut board,
        &CreateTicketInput {
            assignee: Some("tech-lead".into()),
            ..titled("A")
        },
        &context(),
        &at(1),
    )
    .unwrap();
    create_ticket(
        &mut board,
        &CreateTicketInput {
            status: Some("todo".into()),
            ..titled("B")
        },
        &context(),
        &at(2),
    )
    .unwrap();
    let keys = |filter: TicketFilter| -> Vec<String> {
        list_tickets(&board, &filter)
            .into_iter()
            .map(|t| t.key.clone())
            .collect()
    };
    assert_eq!(
        keys(TicketFilter {
            status: Some("todo".into()),
            ..TicketFilter::default()
        }),
        vec!["KAN-2"]
    );
    assert_eq!(
        keys(TicketFilter {
            assignee: Some("".into()),
            ..TicketFilter::default()
        }),
        vec!["KAN-2"],
        "an empty assignee matches unassigned tickets"
    );
    assert_eq!(
        keys(TicketFilter {
            assignee: Some("tech-lead".into()),
            ..TicketFilter::default()
        }),
        vec![a.key.clone()]
    );
}

#[tokio::test]
async fn the_board_file_round_trips_and_a_missing_file_is_an_empty_board() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nested").join("board.json");
    assert!(read_board(&file).await.unwrap().tickets.is_empty());

    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Persisted"), &context(), &at(1)).unwrap();
    create_comment(
        &mut board,
        &comment(&created.id, "hi", Some("user"), None),
        &at(2),
    )
    .unwrap();
    write_board(&file, &board).await.unwrap();

    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(raw["version"], 1);
    assert_eq!(raw["next_number"], 2);
    assert!(
        raw["tickets"][&created.id]["activity"][0]
            .get("comment_id")
            .is_none(),
        "absent optional activity fields stay absent on disk"
    );

    let reread = read_board(&file).await.unwrap();
    assert_eq!(reread.tickets, board.tickets);
    assert!(
        std::fs::read_dir(file.parent().unwrap()).unwrap().count() == 1,
        "no temp file is left behind"
    );

    std::fs::write(&file, "{not json").unwrap();
    assert!(
        read_board(&file)
            .await
            .unwrap_err()
            .contains("not valid JSON")
    );
}

#[test]
fn normalize_config_repairs_hostile_input() {
    let fallback = normalize(&json!({}));
    assert_eq!(fallback.columns.len(), 5);
    assert_eq!(fallback, KanbanConfig::default());
    let repaired = normalize(&json!({
        "id_prefix": "bad prefix!",
        "default_status": "nope",
        "columns": [{ "id": "open", "label": "Open" }, { "id": "open", "label": "Dup" }, "junk", { "id": "closed" }],
        "priorities": ["p1", " ", "p2"]
    }));
    assert_eq!(repaired.id_prefix, "KAN");
    assert_eq!(repaired.default_status, "open");
    assert_eq!(repaired.columns.len(), 2);
    assert_eq!(repaired.priorities, vec!["p1", "p2"]);
}

const TICKET_ID: &str = "8ec5c3fc-21bd-4142-b503-31bdb9e1ce70";

fn sample_ticket() -> Ticket {
    let mut board = Board::empty();
    let mut ticket = create_ticket(&mut board, &titled("Sample"), &context(), &at(1)).unwrap();
    ticket.id = TICKET_ID.into();
    ticket.key = "KAN-7".into();
    ticket
}

fn change_event(event: &str) -> ChangeEvent {
    ChangeEvent {
        event: event.into(),
        ticket_id: TICKET_ID.into(),
        ticket_key: "KAN-7".into(),
        ticket: sample_ticket().into(),
        comment: None,
        activity: None,
        at: at(9),
    }
}

fn comment_event(author: &str, parent: Option<&str>) -> CommentEvent {
    CommentEvent {
        event: "comment.created".into(),
        ticket_id: TICKET_ID.into(),
        ticket_key: "KAN-7".into(),
        ticket: sample_ticket().into(),
        comment: Comment {
            id: "comment-1".into(),
            ticket_id: TICKET_ID.into(),
            parent_id: parent.map(String::from),
            body: "hello".into(),
            author: author.into(),
            created_at: at(9),
        },
        author: author.into(),
        at: at(9),
    }
}

#[test]
fn the_change_filter_matches_by_key_uuid_and_event_name() {
    let event = change_event("ticket.updated");
    assert!(matches_change(&json!({}), &event));
    assert!(matches_change(&json!({ "ticket_id": "KAN-7" }), &event));
    assert!(
        matches_change(&json!({ "ticket_id": "kan-7" }), &event),
        "the key is case-insensitive"
    );
    assert!(
        matches_change(&json!({ "ticket_id": TICKET_ID }), &event),
        "the uuid works too"
    );
    assert!(!matches_change(&json!({ "ticket_id": "KAN-8" }), &event));
    assert!(matches_change(
        &json!({ "events": ["ticket.updated"] }),
        &event
    ));
    assert!(!matches_change(
        &json!({ "events": ["comment.created"] }),
        &event
    ));
    assert!(!matches_change(
        &json!({ "ticket_id": "KAN-7", "events": ["ticket.moved"] }),
        &event
    ));
    assert!(
        matches_change(&Value::Null, &event),
        "a malformed config is no filter"
    );
}

#[test]
fn the_comment_filter_narrows_by_ticket_author_and_thread() {
    let from_agent = comment_event("console-ui", None);
    let from_user = comment_event("user", None);
    let reply = comment_event("console-ui", Some("root-1"));

    assert!(matches_comment(
        &json!({ "ticket_id": "KAN-7" }),
        &from_user
    ));
    assert!(
        matches_comment(&json!({ "ticket_id": "KAN-7" }), &reply),
        "replies match a ticket-wide binding"
    );
    assert!(matches_comment(
        &json!({ "ticket_id": "KAN-7", "author": "console-ui" }),
        &from_agent
    ));
    assert!(
        !matches_comment(
            &json!({ "ticket_id": "KAN-7", "author": "console-ui" }),
            &from_user
        ),
        "an agent watching its own comments is not woken by a human comment"
    );
    assert!(matches_comment(
        &json!({ "ticket_id": "KAN-7", "exclude_author": "console-ui" }),
        &from_user
    ));
    assert!(!matches_comment(
        &json!({ "ticket_id": "KAN-7", "exclude_author": "console-ui" }),
        &from_agent
    ));
    assert!(!matches_comment(
        &json!({ "ticket_id": "KAN-7", "root_only": true }),
        &reply
    ));
    assert!(matches_comment(
        &json!({ "ticket_id": "KAN-7", "root_only": true }),
        &from_agent
    ));
    assert!(
        !matches_comment(&json!({}), &from_user),
        "a comment binding always names its ticket"
    );
    assert!(!matches_comment(
        &json!({ "ticket_id": "KAN-99" }),
        &from_user
    ));
}

#[test]
fn a_binding_can_ask_for_the_summary_payload() {
    assert!(!wants_summary(&json!({ "ticket_id": "KAN-7" })));
    assert!(!wants_summary(
        &json!({ "ticket_id": "KAN-7", "ticket": "full" })
    ));
    assert!(wants_summary(
        &json!({ "ticket_id": "KAN-7", "ticket": "summary" })
    ));

    let event = comment_event("user", None);
    let (full, summary) = payloads(&event, &event.ticket).unwrap();
    assert!(
        full["ticket"]["comments"].is_array(),
        "the full payload carries the thread"
    );
    assert_eq!(full["ticket"]["key"], "KAN-7");
    assert!(
        summary["ticket"].get("comments").is_none(),
        "the summary drops the thread"
    );
    assert!(summary["ticket"].get("description").is_none());
    assert_eq!(summary["ticket"]["key"], "KAN-7");
    assert_eq!(summary["ticket"]["comment_count"], 0);
    assert_eq!(
        summary["comment"]["body"], "hello",
        "the new comment itself stays"
    );

    let config: ChangeConfig = serde_json::from_value(json!({ "ticket": "summary" })).unwrap();
    assert!(config.ticket.is_some());
    let schema = serde_json::to_value(schemars::schema_for!(ChangeConfig)).unwrap();
    assert!(schema["properties"]["ticket"].is_object());
    assert!(
        serde_json::from_value::<ChangeConfig>(json!({ "ticket": "whole" })).is_err(),
        "only full and summary are accepted"
    );
}

#[test]
fn list_tickets_filters_by_updated_since() {
    let mut board = Board::empty();
    create_ticket(&mut board, &titled("Old"), &context(), &at(1)).unwrap();
    let fresh = create_ticket(&mut board, &titled("Fresh"), &context(), &at(5)).unwrap();
    let keys: Vec<String> = list_tickets(
        &board,
        &TicketFilter {
            updated_since: Some(at(3)),
            ..TicketFilter::default()
        },
    )
    .into_iter()
    .map(|t| t.key.clone())
    .collect();
    assert_eq!(keys, vec![fresh.key]);
}

#[test]
fn board_view_narrows_columns_limits_rows_and_can_go_compact() {
    let mut board = Board::empty();
    let config = KanbanConfig {
        columns: context().columns,
        default_status: "backlog".into(),
        ..KanbanConfig::default()
    };
    for (second, title, status) in [
        (1, "A", "todo"),
        (2, "B", "todo"),
        (3, "C", "done"),
        (4, "D", "backlog"),
    ] {
        create_ticket(
            &mut board,
            &CreateTicketInput {
                status: Some(status.into()),
                ..titled(title)
            },
            &context(),
            &at(second),
        )
        .unwrap();
    }

    let everything = board_view(&board, &config, &BoardGetInput::default());
    assert_eq!(everything.columns.len(), 3);
    assert_eq!(everything.ticket_count, 4);
    let first = serde_json::to_value(&everything.columns[0].tickets[0]).unwrap();
    assert!(
        first.get("id").is_some(),
        "the default row is the full summary"
    );

    let narrowed = board_view(
        &board,
        &config,
        &BoardGetInput {
            statuses: Some(vec!["todo".into(), "ghost".into()]),
            limit: Some(1),
            compact: true,
            ..BoardGetInput::default()
        },
    );
    assert_eq!(narrowed.columns.len(), 1, "unknown column ids are ignored");
    assert_eq!(narrowed.columns[0].id, "todo");
    assert_eq!(
        narrowed.columns[0].tickets.len(),
        1,
        "limit applies per column"
    );
    let row = serde_json::to_value(&narrowed.columns[0].tickets[0]).unwrap();
    assert_eq!(row["key"], "KAN-1");
    assert!(
        row.get("id").is_none() && row.get("labels").is_none(),
        "compact rows drop the heavy fields"
    );
    assert_eq!(row["comment_count"], 0);
    assert_eq!(narrowed.ticket_count, 4, "ticket_count stays board-wide");

    let recent = board_view(
        &board,
        &config,
        &BoardGetInput {
            updated_since: Some(at(2)),
            ..BoardGetInput::default()
        },
    );
    let kept: Vec<usize> = recent.columns.iter().map(|c| c.tickets.len()).collect();
    assert_eq!(
        kept,
        vec![1, 0, 1],
        "backlog D and done C are newer than t=2"
    );

    let schema = serde_json::to_value(schemars::schema_for!(BoardGetInput)).unwrap();
    for field in ["statuses", "updated_since", "limit", "compact"] {
        assert!(
            schema["properties"][field].is_object(),
            "{field} is in the schema"
        );
    }
}

#[test]
fn matches_ticket_treats_an_empty_reference_as_no_filter() {
    assert!(matches_ticket("", TICKET_ID, "KAN-7"));
    assert!(matches_ticket("KAN-7", TICKET_ID, "KAN-7"));
    assert!(!matches_ticket("KAN-8", TICKET_ID, "KAN-7"));
}

#[test]
fn the_catalog_names_every_function_once_with_typed_object_schemas() {
    let ids = functions::FUNCTION_IDS;
    assert_eq!(ids.len(), 13);
    let mut unique = ids.to_vec();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
    assert!(ids.iter().all(|id| id.starts_with("kanban::")));

    let request = serde_json::to_value(schemars::schema_for!(TicketCreateInput)).unwrap();
    assert_eq!(request["type"], "object");
    assert_eq!(request["required"], json!(["title"]));
    let update = serde_json::to_value(schemars::schema_for!(TicketUpdateInput)).unwrap();
    assert_eq!(
        update["properties"]["assignee"]["type"],
        json!(["string", "null"])
    );
    let comment = serde_json::to_value(schemars::schema_for!(CommentCreateInput)).unwrap();
    assert_eq!(comment["required"], json!(["body", "ticket_id"]));
    let board = serde_json::to_value(schemars::schema_for!(BoardView)).unwrap();
    assert_eq!(board["properties"]["columns"]["type"], "array");
    let info = serde_json::to_value(schemars::schema_for!(ConfigInfo)).unwrap();
    assert!(info["properties"]["subscribers"].is_object());
}

#[test]
fn update_input_distinguishes_a_missing_assignee_from_null() {
    let absent: TicketUpdateInput = serde_json::from_value(json!({ "id": "KAN-1" })).unwrap();
    assert_eq!(absent.assignee, None);
    let cleared: TicketUpdateInput =
        serde_json::from_value(json!({ "id": "KAN-1", "assignee": null })).unwrap();
    assert_eq!(cleared.assignee, Some(None));
    let set: TicketUpdateInput =
        serde_json::from_value(json!({ "id": "KAN-1", "assignee": "tech-lead" })).unwrap();
    assert_eq!(set.assignee, Some(Some("tech-lead".into())));
}
