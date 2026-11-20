//! A change made inside an agent's turn records that agent's session; a person's change records none; and the
//! links of a ticket follow from its activity.

use iii_kanban::board::{
    BoardContext, CreateCommentInput, CreateTicketInput, UpdatePatch, create_comment,
    create_ticket, find_ticket, update_ticket,
};
use iii_kanban::config::Column;
use iii_kanban::session_links::{LinksFilter, links, ticket_links};
use iii_kanban::store::Board;
use iii_sdk::InvocationCtx;

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

/// The invocation an agent's call arrives as: the harness's turn baggage on it.
fn agent_call(session: &str, turn: &str) -> InvocationCtx {
    InvocationCtx {
        function_id: "kanban::ticket::update".into(),
        invocation_id: "inv-1".into(),
        caller: None,
        activation: None,
        observe_depth: 0,
        metadata: None,
        traceparent: None,
        baggage: Some(format!(
            "iii.session.id={session},iii.message.id={turn},iii.tag.kind=harness.turn"
        )),
    }
}

#[tokio::test]
async fn an_agents_change_records_its_session_and_a_persons_does_not() {
    let mut board = Board::default();
    let ctx = context();
    let created = create_ticket(
        &mut board,
        &CreateTicketInput {
            title: "Charge overages through Stripe".into(),
            ..CreateTicketInput::default()
        },
        &ctx,
        "2026-11-19T10:00:00.000Z",
    )
    .expect("created by a person");

    // Two agent sessions work the ticket; the first one twice.
    let id = created.id.clone();
    iii_sdk::scope(agent_call("console-7f3a-kan-45", "t_1"), async {
        let patch = UpdatePatch {
            status: Some("todo".into()),
            ..UpdatePatch::default()
        };
        update_ticket(
            &mut board,
            &id,
            &patch,
            "deepwork-lead",
            &ctx,
            "2026-11-19T10:01:00.000Z",
        )
        .unwrap();
    })
    .await;
    iii_sdk::scope(agent_call("kan45-retry", "t_9"), async {
        create_comment(
            &mut board,
            &CreateCommentInput {
                ticket_id: "KAN-1".into(),
                body: "Fixed the rounding.".into(),
                parent_id: None,
                author: Some("deepwork-impl".into()),
            },
            "2026-11-19T10:02:00.000Z",
        )
        .unwrap();
    })
    .await;
    iii_sdk::scope(agent_call("console-7f3a-kan-45", "t_2"), async {
        let patch = UpdatePatch {
            priority: Some("high".into()),
            ..UpdatePatch::default()
        };
        update_ticket(
            &mut board,
            &id,
            &patch,
            "deepwork-lead",
            &ctx,
            "2026-11-19T10:03:00.000Z",
        )
        .unwrap();
    })
    .await;

    let ticket = find_ticket(&board, &id).unwrap().clone();
    let created_entry = &ticket.activity[0];
    assert_eq!(
        created_entry.session_id, None,
        "a person's change carries no session"
    );
    assert!(
        ticket.activity[1..]
            .iter()
            .all(|entry| entry.session_id.is_some()),
        "every agent change carries its session"
    );

    let rows = ticket_links(&ticket);
    assert_eq!(rows.len(), 2, "one row per session, not per change");
    assert_eq!(rows[0].session_id, "console-7f3a-kan-45");
    assert_eq!(rows[0].writes, 2);
    assert_eq!(rows[0].first_at, "2026-11-19T10:01:00.000Z");
    assert_eq!(rows[0].last_at, "2026-11-19T10:03:00.000Z");
    assert_eq!(rows[0].turn_id.as_deref(), Some("t_2"));
    assert_eq!(rows[0].actor, "deepwork-lead");
    assert_eq!(rows[1].session_id, "kan45-retry");
    assert_eq!(rows[1].last_action, "comment.created");

    let by_session = links(
        &board,
        &LinksFilter {
            session_id: Some("kan45-retry".into()),
            ..LinksFilter::default()
        },
    );
    assert_eq!(by_session.len(), 1);
    assert_eq!(by_session[0].ticket_key, "KAN-1");
    assert!(
        links(
            &board,
            &LinksFilter {
                project_id: Some("no-such-project".into()),
                ..LinksFilter::default()
            }
        )
        .is_empty(),
        "an unknown project matches nothing"
    );
}

#[test]
fn an_activity_entry_without_a_session_still_reads_from_an_older_board_file() {
    let entry: iii_kanban::store::Activity = serde_json::from_value(serde_json::json!({
        "id": "a1", "ticket_id": "t1", "seq": 1, "type": "ticket.created", "actor": "user", "at": "2026-01-01T00:00:00.000Z"
    }))
    .unwrap();
    assert_eq!(entry.session_id, None);
    let written = serde_json::to_value(&entry).unwrap();
    assert!(
        written.get("session_id").is_none(),
        "no session, no field: the file stays as it was"
    );
}
