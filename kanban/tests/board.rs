//! Behaviour of the board operations, the store round-trip, the trigger
//! filters and the function catalog — engine-free.

use iii_kanban::board::TicketView;
use iii_kanban::board::{
    ActivityFilter, BoardContext, CommentFilter, CreateCommentInput, CreateTicketInput,
    TicketFilter, UpdatePatch, create_comment, create_ticket, delete_ticket, find_ticket,
    list_activity, list_comments, list_tickets, move_ticket, next_at, restore_ticket, summarize,
    ticket_links, ticket_view, update_ticket,
};
use iii_kanban::config::{Column, KanbanConfig, normalize};
use iii_kanban::events::{
    ChangeConfig, ChangeEvent, CommentEvent, matches_change, matches_comment, matches_ticket,
    payloads, wants_summary,
};
use iii_kanban::functions::{
    self, BoardGetInput, BoardView, CommentCreateInput, ConfigInfo, TicketCreateInput,
    TicketMoveInput, TicketUpdateInput, board_view,
};
use iii_kanban::store::{Board, Comment, Ticket, read_board, write_board};
use serde_json::{Value, json};

fn context() -> BoardContext {
    BoardContext {
        prefix: "KAN".into(),
        default_status: "backlog".into(),
        done_status: "done".into(),
        canceled_status: None,
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

/// What `functions::ticket_outcome` announces for a one-ticket write: the
/// ticket of the first event (`None` fires nothing) and the caller's result.
struct Fired {
    ticket: Option<Ticket>,
    result: Option<Ticket>,
}

fn ticket_outcome(before: Option<Ticket>, after: Option<Ticket>) -> Fired {
    let outcome = functions::ticket_outcome(&Board::empty(), &context(), before, after, &[]);
    Fired {
        ticket: outcome.tickets.first().map(|view| view.ticket.clone()),
        result: outcome.result.map(|view| view.ticket),
    }
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

    let restored = restore_ticket(&mut board, &created.id, "user", &at(4))
        .unwrap()
        .ticket;
    assert_eq!(restored.deleted_at, None);
    assert_eq!(list_tickets(&board, &TicketFilter::default()).len(), 1);
    assert_eq!(restored.activity.last().unwrap().kind, "ticket.restored");
}

#[test]
fn deleting_a_deleted_ticket_and_restoring_a_live_one_change_nothing() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Twice"), &context(), &at(1)).unwrap();

    let live = ticket(&board, &created.id);
    let restored_live = restore_ticket(&mut board, &created.id, "user", &at(2))
        .unwrap()
        .ticket;
    assert_eq!(restored_live, live, "restoring a live ticket is a no-op");
    assert!(
        ticket_outcome(Some(live), Some(restored_live))
            .ticket
            .is_none(),
        "no ticket.restored event for a no-op"
    );

    let deleted = delete_ticket(&mut board, &created.id, "user", &at(3)).unwrap();
    let again = delete_ticket(&mut board, &created.id, "user", &at(4)).unwrap();
    assert_eq!(
        again, deleted,
        "deleting twice keeps deleted_at, updated_at and the timeline"
    );
    assert_eq!(again.deleted_at.as_deref(), Some(at(3).as_str()));
    assert!(
        ticket_outcome(Some(deleted), Some(again)).ticket.is_none(),
        "no ticket.deleted event for a no-op"
    );
}

#[test]
fn the_change_event_fans_out_only_when_the_ticket_changed() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Events"), &context(), &at(1)).unwrap();
    let deleted = delete_ticket(&mut board, &created.id, "user", &at(2)).unwrap();

    let changed = ticket_outcome(Some(created.clone()), Some(deleted.clone()));
    assert_eq!(changed.ticket.as_ref(), Some(&deleted));
    assert_eq!(changed.result.as_ref(), Some(&deleted));

    let unchanged = ticket_outcome(Some(deleted.clone()), Some(deleted.clone()));
    assert!(unchanged.ticket.is_none(), "same ticket, no event");
    assert_eq!(
        unchanged.result.as_ref(),
        Some(&deleted),
        "the caller still gets the ticket"
    );

    let missing = ticket_outcome(None, None);
    assert!(missing.ticket.is_none() && missing.result.is_none());
}

#[test]
fn moving_a_ticket_to_where_it_already_is_changes_nothing() {
    let mut board = Board::empty();
    for title in ["A", "B", "C"] {
        create_ticket(&mut board, &titled(title), &context(), &at(1)).unwrap();
    }
    let before = board.clone();
    for (key, index) in [
        ("KAN-2", Some(1)),
        ("KAN-1", Some(0)),
        ("KAN-3", None),
        ("KAN-3", Some(99)),
    ] {
        let moved = move_ticket(
            &mut board,
            key,
            "backlog",
            index,
            Some("agent"),
            &context(),
            &at(9),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            moved,
            ticket(&before, key),
            "{key} at {index:?}: ticket unchanged"
        );
        assert!(
            ticket_outcome(Some(ticket(&before, key)), Some(moved))
                .ticket
                .is_none(),
            "{key} at {index:?}: no ticket.moved event"
        );
        assert_eq!(
            board.tickets, before.tickets,
            "{key} at {index:?}: board untouched"
        );
    }

    // A real reorder still counts.
    let moved = move_ticket(
        &mut board,
        "KAN-3",
        "backlog",
        Some(0),
        None,
        &context(),
        &at(10),
    )
    .unwrap()
    .unwrap();
    assert_eq!(moved.order, 1);
    assert_eq!(moved.updated_at, at(10));
    assert!(
        ticket_outcome(Some(ticket(&before, "KAN-3")), Some(moved))
            .ticket
            .is_some()
    );
}

#[test]
fn a_deleted_ticket_cannot_be_moved_until_it_is_restored() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Gone"), &context(), &at(1)).unwrap();
    delete_ticket(&mut board, &created.id, "user", &at(2)).unwrap();
    let before = board.clone();

    let error = move_ticket(
        &mut board,
        &created.key,
        "done",
        None,
        None,
        &context(),
        &at(3),
    )
    .unwrap_err();
    assert_eq!(error, "cannot move a deleted ticket");
    assert_eq!(
        board.tickets, before.tickets,
        "a rejected move leaves the board alone"
    );

    restore_ticket(&mut board, &created.id, "user", &at(4)).unwrap();
    let moved = move_ticket(
        &mut board,
        &created.key,
        "done",
        None,
        None,
        &context(),
        &at(5),
    )
    .unwrap()
    .unwrap();
    assert_eq!(moved.status, "done");
}

#[test]
fn a_deleted_ticket_takes_other_edits_but_not_a_status_change() {
    let mut board = Board::empty();
    let created = create_ticket(&mut board, &titled("Gone"), &context(), &at(1)).unwrap();
    delete_ticket(&mut board, &created.id, "user", &at(2)).unwrap();
    let before = board.clone();

    let status = UpdatePatch {
        status: Some("done".into()),
        ..UpdatePatch::default()
    };
    let error = update_ticket(
        &mut board,
        &created.key,
        &status,
        "user",
        &context(),
        &at(3),
    )
    .unwrap_err();
    assert_eq!(error, "cannot move a deleted ticket");
    assert_eq!(
        board.tickets, before.tickets,
        "a rejected status change leaves the board alone"
    );

    // Annotating still works, and so does naming the column it is already in.
    let annotate = UpdatePatch {
        title: Some("Gone, noted".into()),
        status: Some(created.status.clone()),
        ..UpdatePatch::default()
    };
    let updated = update_ticket(
        &mut board,
        &created.key,
        &annotate,
        "agent",
        &context(),
        &at(4),
    )
    .unwrap()
    .unwrap()
    .ticket;
    assert_eq!(updated.title, "Gone, noted");
    assert_eq!(updated.status, created.status);

    restore_ticket(&mut board, &created.id, "user", &at(5)).unwrap();
    let updated = update_ticket(
        &mut board,
        &created.key,
        &status,
        "user",
        &context(),
        &at(6),
    )
    .unwrap()
    .unwrap()
    .ticket;
    assert_eq!(updated.status, "done");
}

#[test]
fn writes_in_the_same_millisecond_get_strictly_increasing_timestamps() {
    // 2026-01-01T00:00:00.000Z, the clock frozen for every write below.
    const FROZEN: i64 = 1_767_225_600_000;
    let mut board = Board::empty();

    let first = next_at(&board, FROZEN);
    assert_eq!(
        first, "2026-01-01T00:00:00.000Z",
        "an empty board follows the clock"
    );
    let created = create_ticket(&mut board, &titled("A"), &context(), &first).unwrap();

    let mut stamps = vec![first];
    for title in ["B", "C", "D"] {
        let write = next_at(&board, FROZEN);
        let patch = UpdatePatch {
            title: Some(title.into()),
            ..UpdatePatch::default()
        };
        let updated = update_ticket(&mut board, &created.key, &patch, "user", &context(), &write)
            .unwrap()
            .unwrap()
            .ticket;
        assert_eq!(updated.updated_at, write);
        assert_eq!(
            updated.activity.last().map(|entry| entry.at.as_str()),
            Some(write.as_str()),
            "the activity entry carries the write's timestamp"
        );
        stamps.push(write);
    }
    assert!(
        stamps.windows(2).all(|pair| pair[0] < pair[1]),
        "strictly increasing: {stamps:?}"
    );
    assert_eq!(stamps[3], "2026-01-01T00:00:00.003Z");

    // A write to another ticket must not reuse an instant either, and a clock
    // stepped back keeps moving forward.
    let other = next_at(&board, FROZEN - 5_000);
    assert_eq!(other, "2026-01-01T00:00:00.004Z");
    // A later clock is taken as is.
    assert_eq!(next_at(&board, FROZEN + 10_000), "2026-01-01T00:00:10.000Z");
}

fn labelled(board: &mut Board, labels: &[&str]) -> Ticket {
    let input = CreateTicketInput {
        labels: Some(labels.iter().map(|label| label.to_string()).collect()),
        ..titled("Labelled")
    };
    create_ticket(board, &input, &context(), &at(1)).unwrap()
}

fn relabel(board: &mut Board, patch: UpdatePatch, second: u32) -> iii_kanban::board::UpdateResult {
    update_ticket(board, "KAN-1", &patch, "agent", &context(), &at(second))
        .unwrap()
        .unwrap()
}

fn strings(values: &[&str]) -> Option<Vec<String>> {
    Some(values.iter().map(|value| value.to_string()).collect())
}

#[test]
fn add_and_remove_labels_edit_the_current_labels_instead_of_replacing_them() {
    let mut board = Board::empty();
    labelled(&mut board, &["Bug", "ui"]);

    // Two writers that both read ["Bug", "ui"]: neither overwrites the other.
    relabel(
        &mut board,
        UpdatePatch {
            add_labels: strings(&["api"]),
            ..UpdatePatch::default()
        },
        2,
    );
    let both = relabel(
        &mut board,
        UpdatePatch {
            add_labels: strings(&["  perf ", "", "   "]),
            ..UpdatePatch::default()
        },
        3,
    );
    assert_eq!(both.ticket.labels, ["Bug", "ui", "api", "perf"]);
    assert_eq!(
        serde_json::to_value(&both.changes).unwrap(),
        json!([{ "field": "labels", "from": ["Bug", "ui", "api"], "to": ["Bug", "ui", "api", "perf"] }])
    );
    assert_eq!(both.ticket.updated_at, at(3));
    assert_eq!(both.ticket.activity.last().unwrap().kind, "ticket.updated");

    let removed = relabel(
        &mut board,
        UpdatePatch {
            remove_labels: strings(&["UI", " PERF ", "missing"]),
            ..UpdatePatch::default()
        },
        4,
    );
    assert_eq!(removed.ticket.labels, ["Bug", "api"]);
}

#[test]
fn add_labels_skips_a_label_already_present_in_any_ascii_case() {
    let mut board = Board::empty();
    labelled(&mut board, &["Bug"]);
    let added = relabel(
        &mut board,
        UpdatePatch {
            add_labels: strings(&["bug", "BUG", "Feature", "feature"]),
            ..UpdatePatch::default()
        },
        2,
    );
    assert_eq!(
        added.ticket.labels,
        ["Bug", "Feature"],
        "first spelling wins"
    );
}

#[test]
fn labels_then_remove_then_add_apply_in_that_order() {
    let mut board = Board::empty();
    labelled(&mut board, &["old"]);
    let result = relabel(
        &mut board,
        UpdatePatch {
            labels: strings(&["a", "b", "c"]),
            remove_labels: strings(&["B", "x"]),
            add_labels: strings(&["b", "d"]),
            ..UpdatePatch::default()
        },
        2,
    );
    assert_eq!(result.ticket.labels, ["a", "c", "b", "d"]);
    assert_eq!(
        serde_json::to_value(&result.changes).unwrap(),
        json!([{ "field": "labels", "from": ["old"], "to": ["a", "c", "b", "d"] }])
    );
}

#[test]
fn a_label_patch_that_changes_nothing_records_nothing() {
    let mut board = Board::empty();
    let created = labelled(&mut board, &["bug", "ui"]);
    for patch in [
        UpdatePatch {
            add_labels: strings(&["BUG", " ui ", ""]),
            ..UpdatePatch::default()
        },
        UpdatePatch {
            remove_labels: strings(&["nope", " "]),
            ..UpdatePatch::default()
        },
        UpdatePatch {
            add_labels: strings(&[]),
            remove_labels: strings(&[]),
            ..UpdatePatch::default()
        },
        UpdatePatch {
            labels: strings(&["bug", "ui"]),
            ..UpdatePatch::default()
        },
        UpdatePatch {
            labels: strings(&["bug", "ui"]),
            add_labels: strings(&["Ui"]),
            ..UpdatePatch::default()
        },
    ] {
        let result = relabel(&mut board, patch.clone(), 9);
        assert!(result.changes.is_empty(), "{patch:?}");
        assert_eq!(
            result.ticket, created,
            "{patch:?}: updated_at and timeline untouched"
        );
        assert!(
            ticket_outcome(Some(created.clone()), Some(result.ticket))
                .ticket
                .is_none(),
            "{patch:?}: no ticket.updated event"
        );
    }
}

#[test]
fn labels_are_deduplicated_ignoring_case_keeping_the_first_spelling() {
    let mut board = Board::empty();
    let created = labelled(&mut board, &["bug", "Bug", " bug ", "feature", "BUG"]);
    assert_eq!(created.labels, ["bug", "feature"]);
    let result = relabel(
        &mut board,
        UpdatePatch {
            labels: strings(&["Docs", "docs", "bug", "BUG"]),
            ..UpdatePatch::default()
        },
        2,
    );
    assert_eq!(result.ticket.labels, ["Docs", "bug"]);
}

#[test]
fn update_input_accepts_add_labels_and_remove_labels() {
    let input: TicketUpdateInput = serde_json::from_value(
        json!({ "id": "KAN-1", "add_labels": ["a"], "remove_labels": ["b", "c"] }),
    )
    .unwrap();
    assert_eq!(input.add_labels, strings(&["a"]));
    assert_eq!(input.remove_labels, strings(&["b", "c"]));
    let absent: TicketUpdateInput = serde_json::from_value(json!({ "id": "KAN-1" })).unwrap();
    assert!(absent.add_labels.is_none() && absent.remove_labels.is_none());

    let schema = serde_json::to_value(schemars::schema_for!(TicketUpdateInput)).unwrap();
    for field in ["add_labels", "remove_labels"] {
        assert_eq!(
            schema["properties"][field]["items"]["type"], "string",
            "{field}"
        );
        assert_eq!(schema["required"], json!(["id"]), "{field} is optional");
    }
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
    assert_eq!(raw["version"], 2);
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
    assert_eq!(
        fallback.columns.len(),
        6,
        "the defaults end with done, then canceled"
    );
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

fn sample_ticket() -> TicketView {
    let mut board = Board::empty();
    let mut ticket = create_ticket(&mut board, &titled("Sample"), &context(), &at(1)).unwrap();
    ticket.id = TICKET_ID.into();
    ticket.key = "KAN-7".into();
    ticket_view(&Board::empty(), &ticket, "done", None)
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
        releases: vec![],
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
        row.get("id").is_some() && row.get("labels").is_none(),
        "compact rows keep the uuid and drop the heavy fields"
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

fn numbered(board: &mut Board, number: u64, title: &str, description: &str) {
    board.next_number = number;
    let input = CreateTicketInput {
        description: Some(description.into()),
        ..titled(title)
    };
    create_ticket(board, &input, &context(), &at(1)).unwrap();
}

#[test]
fn ticket_links_find_keys_in_both_directions_on_word_boundaries() {
    let mut board = Board::empty();
    // Created out of numeric order: the lists sort by ticket number.
    numbered(&mut board, 20, "Twenty, see KAN-5", "KAN-5 again");
    numbered(
        &mut board,
        5,
        "Subject, blocks KAN-16",
        "Blocked by kan-13 and (KAN-14).",
    );
    numbered(
        &mut board,
        12,
        "Twelve",
        "Cites KAN-50, XKAN-5 and KAN-5x, none of them KAN five.",
    );
    numbered(&mut board, 13, "Thirteen", "");
    numbered(&mut board, 14, "Fourteen", "");
    numbered(&mut board, 15, "Fifteen", "Part of KAN-5.");
    numbered(&mut board, 16, "Sixteen", "");
    numbered(&mut board, 17, "Seventeen", "");
    numbered(&mut board, 18, "Eighteen", "Also KAN-5, but deleted.");
    numbered(&mut board, 120, "Hundred twenty", "");
    for (key, body) in [
        (
            "KAN-5",
            "In KAN-15; also KAN-120, KAN-17, KAN-999 and self KAN-5.",
        ),
        ("KAN-13", "Back to kan-5."),
    ] {
        create_comment(&mut board, &comment(key, body, None, None), &at(2))
            .unwrap()
            .unwrap();
    }
    for key in ["KAN-17", "KAN-18"] {
        delete_ticket(&mut board, key, "user", &at(3)).unwrap();
    }

    let keys = |tickets: Vec<&Ticket>| -> Vec<String> {
        tickets.into_iter().map(|t| t.key.clone()).collect()
    };
    let (mentions, mentioned_by) = ticket_links(&board, "kan-5", "KAN").unwrap();
    assert_eq!(
        keys(mentions),
        ["KAN-13", "KAN-14", "KAN-15", "KAN-16", "KAN-120"],
        "title, description and comments; not KAN-12 inside KAN-120, not itself, not the deleted or the unknown"
    );
    assert_eq!(
        keys(mentioned_by),
        ["KAN-13", "KAN-15", "KAN-20"],
        "once each; not KAN-50, XKAN-5, KAN-5x or the deleted"
    );
    assert!(ticket_links(&board, "KAN-404", "KAN").is_none());
}

#[test]
fn ticket_links_boundary_is_not_a_letter_digit_slash_or_hyphen_on_either_side() {
    for (text, found) in [
        ("fix/KAN-1-login", false),
        ("KAN-1-2", false),
        ("https://x/browse/KAN-1", false),
        ("çKAN-1", false),
        ("KAN-1é", false),
        ("2KAN-1", false),
        ("(KAN-1)", true),
        ("KAN-1.", true),
        ("KAN-1,", true),
        ("“KAN-1”", true),
        ("Revisão: veja KAN-1, ação já feita", true),
        ("_KAN-1_", true),
    ] {
        let mut board = Board::empty();
        numbered(&mut board, 1, "Subject", "");
        numbered(&mut board, 2, "Citing", text);
        let (_, mentioned_by) = ticket_links(&board, "KAN-1", "KAN").unwrap();
        assert_eq!(mentioned_by.len(), usize::from(found), "{text:?}");
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
    assert_eq!(ids.len(), 28);
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
    assert_eq!(info["properties"]["done_status"]["type"], "string");
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

#[test]
fn ticket_writes_answer_with_the_compact_row_only_when_asked() {
    let mut board = Board::empty();
    let ticket = create_ticket(&mut board, &titled("A"), &context(), &at(1)).unwrap();
    let created = create_comment(
        &mut board,
        &CreateCommentInput {
            ticket_id: "KAN-1".into(),
            body: "hi".into(),
            parent_id: None,
            author: Some("tech-lead".into()),
        },
        &at(2),
    )
    .unwrap()
    .unwrap();
    // Writes answer with the ticket view (the ticket plus parent, children and progress).
    let view = ticket_view(&board, &ticket, "done", None);
    let commented = ticket_view(&board, &created.ticket, "done", None);

    assert_eq!(
        serde_json::to_value(functions::written(view.clone(), false)).unwrap(),
        serde_json::to_value(&view).unwrap(),
        "without compact the response is the full ticket view, unchanged"
    );
    let brief = serde_json::to_value(functions::written(commented.clone(), true)).unwrap();
    let mut keys: Vec<&str> = brief
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "assignee",
            "comment_count",
            "id",
            "key",
            "priority",
            "status",
            "title"
        ]
    );
    assert_eq!(brief["key"], "KAN-1");
    assert_eq!(brief["comment_count"], 1);

    let full = serde_json::to_value(functions::comment_created(
        created.comment.clone(),
        commented.clone(),
        false,
    ))
    .unwrap();
    assert_eq!(full["ticket"], serde_json::to_value(&commented).unwrap());
    let compact = serde_json::to_value(functions::comment_created(
        created.comment.clone(),
        commented.clone(),
        true,
    ))
    .unwrap();
    assert_eq!(
        compact["comment"],
        serde_json::to_value(&created.comment).unwrap()
    );
    assert_eq!(
        compact["ticket"], brief,
        "a compact comment write carries the brief"
    );

    for compact in [false, true] {
        let flag = json!(compact);
        let create: TicketCreateInput =
            serde_json::from_value(json!({ "title": "A", "compact": flag })).unwrap();
        let update: TicketUpdateInput =
            serde_json::from_value(json!({ "id": "KAN-1", "compact": flag })).unwrap();
        let moved: TicketMoveInput =
            serde_json::from_value(json!({ "id": "KAN-1", "status": "done", "compact": flag }))
                .unwrap();
        let comment: CommentCreateInput =
            serde_json::from_value(json!({ "ticket_id": "KAN-1", "body": "x", "compact": flag }))
                .unwrap();
        assert_eq!(
            [
                create.compact,
                update.compact,
                moved.compact,
                comment.compact
            ],
            [compact; 4]
        );
    }
    let defaulted: TicketMoveInput =
        serde_json::from_value(json!({ "id": "KAN-1", "status": "done" })).unwrap();
    assert!(!defaulted.compact, "compact is off unless asked");
}
