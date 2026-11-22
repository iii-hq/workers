//! Question mirrors, decisions and board-wide activity without an engine.
use std::sync::{Arc, RwLock};

use iii_kanban::board::{self, ActivityFilter, BoardContext, CreateTicketInput};
use iii_kanban::config::KanbanConfig;
use iii_kanban::plan::{self, ProjectFields, ReleaseFields};
use iii_kanban::questions::{self, AnswerInput, AskEntry, ListInput, RecordInput};
use iii_kanban::store::{Board, BoardStore};
use serde_json::{Value, json};

const AT: &str = "2026-11-21T12:00:00.000Z";
const LATER: &str = "2026-11-21T12:01:00.000Z";

fn board() -> Board {
    let mut board = Board::empty();
    for title in ["First card", "Second card"] {
        board::create_ticket(
            &mut board,
            &CreateTicketInput {
                title: title.into(),
                ..Default::default()
            },
            &BoardContext::from(&KanbanConfig::default()),
            AT,
        )
        .unwrap();
    }
    board
}

fn entry(ticket: Option<&str>, outcome: Option<&str>) -> AskEntry {
    let mut value = json!({
        "session_id":"session-1","question_id":"call-1","asked_at":1795262400000_i64,
        "state":"open",
        "questions":[{"header":"Builds","question":"How should builds run?","multi_select":false,
            "options":[{"label":"Isolate","description":"Separate worktrees","recommended":true},{"label":"Share","description":"Same checkout"}]}]
    });
    if let Some(ticket) = ticket {
        value["about"] = json!({"ticket":ticket,"why":"Avoid contention","class":"run_health","blocks":["KAN-2"]});
    }
    if let Some(outcome) = outcome {
        value["state"] = json!("answered");
        value["answer_text"] = json!("Isolate");
        value["by"] = json!({"channel":"tower","user":"Pat"});
        value["outcome"] = json!(outcome);
    }
    serde_json::from_value(value).unwrap()
}

fn get<'a>(board: &'a Board, key: &str) -> &'a iii_kanban::store::Ticket {
    board::find_ticket(board, key).unwrap()
}

#[test]
fn mirror_prefers_explicit_ticket_then_session_link_and_skips_unlinked_or_invalid_tickets() {
    let mut board = board();
    questions::mirror(&mut board, &entry(Some("kan-1"), None), Some("KAN-2"), AT).unwrap();
    assert_eq!(get(&board, "KAN-1").comments.len(), 1);
    assert!(get(&board, "KAN-2").comments.is_empty());
    let mut fallback = entry(None, None);
    fallback.question.question_id = "call-2".into();
    questions::mirror(&mut board, &fallback, Some("KAN-2"), AT).unwrap();
    assert_eq!(get(&board, "KAN-2").comments.len(), 1);
    fallback.question.question_id = "call-3".into();
    assert!(
        questions::mirror(&mut board, &fallback, None, AT)
            .unwrap()
            .is_empty()
    );
    assert!(
        questions::mirror(&mut board, &entry(Some("KAN-404"), None), Some("KAN-2"), AT)
            .unwrap()
            .is_empty()
    );
    assert_eq!(get(&board, "KAN-2").comments.len(), 1);
}

#[test]
fn mirror_uses_asking_profile_as_author_and_falls_back_to_session() {
    let mut board = board();
    let mut attributed = entry(Some("KAN-1"), None);
    attributed.asked_by = Some("backend-engineer".into());
    questions::mirror(&mut board, &attributed, None, AT).unwrap();

    let mut unattributed = entry(Some("KAN-1"), None);
    unattributed.question.question_id = "call-2".into();
    questions::mirror(&mut board, &unattributed, None, LATER).unwrap();

    let comments = &get(&board, "KAN-1").comments;
    assert_eq!(comments[0].author, "backend-engineer");
    assert_eq!(comments[1].author, "session-1");
}

#[test]
fn settle_first_appends_both_comments_in_order_and_late_events_do_not_rewrite_them() {
    let mut board = board();
    let mut settled = entry(Some("KAN-1"), Some("answered"));
    settled.asked_by = Some("backend-engineer".into());
    let added = questions::mirror(&mut board, &settled, None, LATER).unwrap();
    assert_eq!(added.len(), 2);
    assert_eq!(added[0].comment.author, "backend-engineer");
    assert_eq!(
        added[1].comment.parent_id.as_ref(),
        Some(&added[0].comment.id)
    );
    assert!(added[0].comment.question.is_some());
    assert!(added[0].comment.answer.is_none());
    assert_eq!(
        added[1].comment.answer.as_ref().unwrap().answer_text,
        "Isolate"
    );
    assert_eq!(
        added[0].ticket.activity.last().unwrap().seq + 1,
        added[1].ticket.activity.last().unwrap().seq
    );
    assert_eq!(questions::open_questions(&added[0].ticket), 1);
    assert_eq!(questions::open_questions(&added[1].ticket), 0);
    let original = serde_json::to_value(&board).unwrap();
    let mut moved = entry(Some("KAN-2"), None);
    moved.question.questions[0].question = "Do not replace the original".into();
    assert!(
        questions::mirror(&mut board, &moved, None, LATER)
            .unwrap()
            .is_empty()
    );
    assert!(
        questions::mirror(&mut board, &settled, None, LATER)
            .unwrap()
            .is_empty()
    );
    assert_eq!(serde_json::to_value(&board).unwrap(), original);
    assert!(get(&board, "KAN-2").comments.is_empty());
}

#[test]
fn question_identity_includes_session_so_forks_with_the_same_call_id_do_not_collide() {
    let mut board = board();
    let mut fork = entry(Some("KAN-1"), None);
    questions::mirror(&mut board, &fork, None, AT).unwrap();
    fork.question.session_id = "fork-1".into();
    questions::mirror(&mut board, &fork, None, LATER).unwrap();
    assert_eq!(questions::open_questions(get(&board, "KAN-1")), 2);
    assert_eq!(get(&board, "KAN-1").comments.len(), 2);
}

#[test]
fn plain_text_keeps_headers_options_recommendation_and_why_without_a_pill() {
    let mut board = board();
    questions::mirror(&mut board, &entry(Some("KAN-1"), None), None, AT).unwrap();
    let ticket = get(&board, "KAN-1");
    assert_eq!(
        ticket.comments[0].body,
        "⛔ Needs you — Builds: How should builds run?\nAvoid contention\n- Isolate: Separate worktrees (recommended)\n- Share: Same checkout"
    );
    assert!(ticket.pills.is_empty());
    assert_eq!(board::summarize(ticket).open_questions, 1);
    assert_eq!(board::brief(ticket).open_questions, 1);
    assert_eq!(board::relation_brief(ticket).open_questions, 1);
    assert_eq!(
        board::ticket_view(&board, ticket, "done", None).open_questions,
        1
    );
}

#[test]
fn all_settlement_outcomes_close_the_question_and_keep_who_answered() {
    for outcome in [
        "answered",
        "dismissed",
        "withdrawn",
        "expired_default",
        "not_delivered",
    ] {
        let mut board = board();
        questions::mirror(&mut board, &entry(Some("KAN-1"), None), None, AT).unwrap();
        questions::mirror(
            &mut board,
            &entry(Some("KAN-1"), Some(outcome)),
            None,
            LATER,
        )
        .unwrap();
        let ticket = get(&board, "KAN-1");
        assert_eq!(questions::open_questions(ticket), 0);
        assert_eq!(board::summarize(ticket).open_questions, 0);
        let listed = questions::list(&board, &ListInput::default()).unwrap();
        assert_eq!(listed.questions[0].state, outcome);
        let answer = listed.questions[0]
            .answer
            .as_ref()
            .unwrap()
            .answer
            .as_ref()
            .unwrap();
        assert_eq!(answer.by.as_ref().unwrap().channel, "tower");
        assert_eq!(answer.by.as_ref().unwrap().user.as_deref(), Some("Pat"));
    }
}

#[test]
fn a_malformed_settlement_does_not_leave_an_orphan_question() {
    let mut board = board();
    let mut settled = entry(Some("KAN-1"), Some("answered"));
    settled.by = None;
    assert!(questions::mirror(&mut board, &settled, None, AT).is_err());
    assert!(get(&board, "KAN-1").comments.is_empty());
}

#[tokio::test]
async fn boot_replay_of_list_entries_repairs_missed_ask_and_settlement_and_survives_restart() {
    let folder = tempfile::tempdir().unwrap();
    let store = BoardStore::new(Arc::new(RwLock::new(KanbanConfig {
        data_path: folder.path().to_string_lossy().into(),
        ..Default::default()
    })));
    store
        .update(|stored, _| {
            *stored = board();
            Ok(())
        })
        .await
        .unwrap();
    // The first live event reached kanban, the settlement did not.
    store
        .update(|board, _| questions::mirror(board, &entry(Some("KAN-1"), None), None, AT))
        .await
        .unwrap();
    let mut missed = entry(None, Some("not_delivered"));
    missed.question.question_id = "missed-ask".into();
    let page: Value = json!({"entries":[serde_json::to_value(entry(Some("KAN-1"),Some("answered"))).unwrap(),serde_json::to_value(&missed).unwrap()]});
    for _ in 0..2 {
        // Reconciliation replays the entire harness list, including closed entries.
        for raw in page["entries"].as_array().unwrap() {
            let entry: AskEntry = serde_json::from_value(raw.clone()).unwrap();
            store
                .update(|board, _| questions::mirror(board, &entry, Some("KAN-2"), LATER))
                .await
                .unwrap();
        }
    }
    let restored = store.read().await.unwrap();
    for key in ["KAN-1", "KAN-2"] {
        assert_eq!(get(&restored, key).comments.len(), 2);
        assert_eq!(questions::open_questions(get(&restored, key)), 0);
    }
    assert_eq!(restored.version, 3);
    assert_eq!(
        get(&restored, "KAN-2").comments[1]
            .answer
            .as_ref()
            .unwrap()
            .outcome,
        questions::AnswerOutcome::NotDelivered
    );
}

fn record_input() -> RecordInput {
    serde_json::from_value(json!({"ticket_id":"KAN-1","questions":[{"header":"Storage","question":"Where to store it?"}],"answer_text":"Reuse board.json","recorded_by":"backend-engineer"})).unwrap()
}

#[test]
fn recording_a_choice_writes_a_complete_pair_with_profile_and_is_not_an_open_ask() {
    let mut board = board();
    let pair = questions::record(&mut board, record_input(), AT).unwrap();
    assert_eq!(pair.len(), 2);
    assert_eq!(
        pair[0].comment.recorded_by.as_deref(),
        Some("backend-engineer")
    );
    assert_eq!(pair[0].comment.author, "backend-engineer");
    assert_eq!(
        pair[0].comment.question.as_ref().unwrap().session_id,
        "recorded:backend-engineer"
    );
    assert_eq!(
        pair[1]
            .comment
            .answer
            .as_ref()
            .unwrap()
            .by
            .as_ref()
            .unwrap()
            .channel,
        "agent"
    );
    assert_eq!(questions::open_questions(get(&board, "KAN-1")), 0);
    let listed = questions::list(
        &board,
        &ListInput {
            state: Some("recorded".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(listed.count, 1);
    assert_eq!(
        listed.questions[0].answer.as_ref().unwrap(),
        &pair[1].comment
    );
    let request =
        json!({"ticket_id":"KAN-1","comment_id":pair[0].comment.id,"dismissed":true,"user":"Pat"});
    assert!(
        questions::answer_request(&board, &serde_json::from_value(request).unwrap())
            .unwrap_err()
            .contains("recorded choice")
    );
}

#[test]
fn invalid_records_leave_comments_untouched() {
    for field in ["recorded_by", "answer_text", "questions"] {
        let mut board = board();
        let mut input = record_input();
        match field {
            "recorded_by" => input.recorded_by.clear(),
            "answer_text" => input.answer_text.clear(),
            _ => input.questions.clear(),
        }
        assert!(questions::record(&mut board, input, AT).is_err());
        assert!(get(&board, "KAN-1").comments.is_empty());
    }
}

#[test]
fn answer_forwards_structured_answers_and_dismissal_as_kanban_without_closing_the_log() {
    let mut board = board();
    let root = questions::mirror(&mut board, &entry(Some("KAN-1"), None), None, AT)
        .unwrap()
        .remove(0)
        .comment;
    for answers in [
        json!([{"header":"Builds","choice":"Isolate"}]),
        json!([{"header":"Builds","choices":["Isolate"]}]),
        json!([{"header":"Builds","other":"Custom"}]),
        json!([{"header":"Builds","skipped":true}]),
    ] {
        let input: AnswerInput = serde_json::from_value(
            json!({"ticket_id":"kan-1","comment_id":root.id,"answers":answers,"user":"Pat"}),
        )
        .unwrap();
        let forwarded = questions::answer_request(&board, &input).unwrap();
        assert_eq!(
            forwarded,
            json!({"session_id":"session-1","question_id":"call-1","answers":answers,"by":{"channel":"kanban","user":"Pat"}})
        );
    }
    let mut input: AnswerInput = serde_json::from_value(
        json!({"ticket_id":"KAN-1","comment_id":root.id,"dismissed":true,"user":"Pat"}),
    )
    .unwrap();
    assert_eq!(
        questions::answer_request(&board, &input).unwrap(),
        json!({"session_id":"session-1","question_id":"call-1","dismissed":true,"by":{"channel":"kanban","user":"Pat"}})
    );
    assert_eq!(questions::open_questions(get(&board, "KAN-1")), 1);
    input.ticket_id = "KAN-2".into();
    assert!(questions::answer_request(&board, &input).is_err());
    input.ticket_id = "KAN-1".into();
    input.dismissed = false;
    assert!(questions::answer_request(&board, &input).is_err());
    input.dismissed = true;
    input.answers = Some(vec![]);
    assert!(questions::answer_request(&board, &input).is_err());
    input.answers = None;
    input.user.clear();
    assert!(questions::answer_request(&board, &input).is_err());
}

#[test]
fn list_filters_projects_releases_state_and_pages_newest_first() {
    let mut board = board();
    let context = BoardContext::from(&KanbanConfig::default());
    let project = plan::create_project(
        &mut board,
        &ProjectFields {
            name: Some("Tower".into()),
            ..Default::default()
        },
        &context,
        AT,
    )
    .unwrap();
    let release = plan::create_release(
        &mut board,
        &ReleaseFields {
            name: Some("November".into()),
            ..Default::default()
        },
        &context,
        AT,
    )
    .unwrap();
    let ticket_id = get(&board, "KAN-1").id.clone();
    let ticket = board.tickets.get_mut(&ticket_id).unwrap();
    ticket.project_id = Some(project.project.id.clone());
    ticket.release_id = Some(release.release.id.clone());
    questions::record(&mut board, record_input(), AT).unwrap();
    let mut open = entry(Some("KAN-2"), None);
    open.question.asked_at += 60_000;
    questions::mirror(&mut board, &open, None, LATER).unwrap();
    let page = questions::list(
        &board,
        &ListInput {
            limit: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(page.questions[0].state, "open");
    assert!(page.cursor.is_some());
    let next = questions::list(
        &board,
        &ListInput {
            limit: Some(1),
            cursor: page.cursor,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(next.questions[0].state, "recorded");
    assert!(next.cursor.is_none());
    for input in [
        ListInput {
            project: Some("Tower".into()),
            ..Default::default()
        },
        ListInput {
            release: Some("November".into()),
            ..Default::default()
        },
        ListInput {
            state: Some("recorded".into()),
            ..Default::default()
        },
    ] {
        let rows = questions::list(&board, &input).unwrap();
        assert_eq!(rows.count, 1);
        assert_eq!(rows.questions[0].comment.ticket_id, ticket_id);
    }
    assert_eq!(
        questions::list(
            &board,
            &ListInput {
                project: Some("".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .count,
        1
    );
    assert_eq!(
        questions::list(
            &board,
            &ListInput {
                project: Some("Missing".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .count,
        0
    );
    assert!(
        questions::list(
            &board,
            &ListInput {
                cursor: Some("missing".into()),
                ..Default::default()
            }
        )
        .is_err()
    );
    // A deleted card still keeps its decision log, and can receive a settlement.
    let id = get(&board, "KAN-2").id.clone();
    board.tickets.get_mut(&id).unwrap().deleted_at = Some(LATER.into());
    open = entry(Some("KAN-2"), Some("not_delivered"));
    questions::mirror(&mut board, &open, None, LATER).unwrap();
    assert_eq!(
        questions::list(
            &board,
            &ListInput {
                state: Some("not_delivered".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .count,
        1
    );
}

#[test]
fn board_wide_activity_is_newest_first_with_filters_and_ticket_history_stays_oldest_first() {
    let mut board = board();
    questions::mirror(&mut board, &entry(Some("KAN-1"), None), None, LATER).unwrap();
    let mut other = entry(Some("KAN-2"), Some("answered"));
    other.question.question_id = "call-2".into();
    questions::mirror(&mut board, &other, None, "2026-11-21T12:02:00.000Z").unwrap();
    let all = board::list_activity(&board, &ActivityFilter::default());
    assert_eq!(all.len(), 5);
    assert_eq!(all[0].ticket_id, get(&board, "KAN-2").id);
    assert!(all.windows(2).all(|pair| pair[0].at >= pair[1].at));
    assert!(all[0].seq > all[1].seq);
    let filtered = board::list_activity(
        &board,
        &ActivityFilter {
            since: Some(AT.into()),
            types: Some(vec!["comment.created".into()]),
            limit: Some(2),
            ..Default::default()
        },
    );
    assert_eq!(filtered.len(), 2);
    assert!(
        filtered
            .iter()
            .all(|entry| entry.kind == "comment.created" && entry.at.as_str() > AT)
    );
    let ticket = board::list_activity(
        &board,
        &ActivityFilter {
            ticket_id: Some("KAN-2".into()),
            ..Default::default()
        },
    );
    assert_eq!(ticket.len(), 3);
    assert_eq!(ticket[0].kind, "ticket.created");
    assert!(
        board::list_activity(
            &board,
            &ActivityFilter {
                ticket_id: Some("KAN-404".into()),
                ..Default::default()
            }
        )
        .is_empty()
    );
    let request: iii_kanban::functions::ActivityListInput =
        serde_json::from_value(json!({})).unwrap();
    assert!(request.ticket_id.is_none());
}

#[test]
fn answering_reservation_and_block_expiry_remain_open_until_delivery_is_confirmed() {
    for state in [questions::AskState::Answering, questions::AskState::Expired] {
        let mut board = board();
        let mut pending = entry(Some("KAN-1"), Some("answered"));
        pending.state = state;
        questions::mirror(&mut board, &pending, None, AT).unwrap();
        assert_eq!(get(&board, "KAN-1").comments.len(), 1);
        assert_eq!(questions::open_questions(get(&board, "KAN-1")), 1);
        questions::mirror(
            &mut board,
            &entry(Some("KAN-1"), Some("answered")),
            None,
            LATER,
        )
        .unwrap();
        assert_eq!(questions::open_questions(get(&board, "KAN-1")), 0);
    }
}

#[test]
fn withdrawn_purge_settlement_closes_an_open_question_without_attribution() {
    let mut board = board();
    questions::mirror(&mut board, &entry(Some("KAN-1"), None), None, AT).unwrap();
    assert_eq!(questions::open_questions(get(&board, "KAN-1")), 1);
    let mut withdrawn = entry(Some("KAN-1"), Some("withdrawn"));
    withdrawn.state = questions::AskState::Withdrawn;
    withdrawn.answer_text = None;
    withdrawn.by = None;
    questions::mirror(&mut board, &withdrawn, None, LATER).unwrap();
    let answer = get(&board, "KAN-1").comments[1].answer.as_ref().unwrap();
    assert_eq!(answer.outcome, questions::AnswerOutcome::Withdrawn);
    assert!(answer.by.is_none());
    assert_eq!(answer.answer_text, "Withdrawn by the asker.");
    assert_eq!(questions::open_questions(get(&board, "KAN-1")), 0);
}

#[test]
fn project_repos_request_preserves_omitted_vs_null_vs_empty() {
    use iii_kanban::functions::{ProjectCreateInput, ProjectUpdateInput};
    let omitted: ProjectUpdateInput = serde_json::from_value(json!({"id":"Tower"})).unwrap();
    assert!(omitted.repos.is_none());
    let clear: ProjectUpdateInput =
        serde_json::from_value(json!({"id":"Tower","repos":null})).unwrap();
    assert_eq!(clear.repos, Some(None));
    let empty: ProjectUpdateInput =
        serde_json::from_value(json!({"id":"Tower","repos":[]})).unwrap();
    assert_eq!(empty.repos, Some(Some(vec![])));
    let create: ProjectCreateInput =
        serde_json::from_value(json!({"name":"Tower","repos":["iii-hq/workers"]})).unwrap();
    assert_eq!(create.repos, Some(vec!["iii-hq/workers".into()]));
}

#[test]
fn state_pagination_continues_if_the_previous_page_question_is_answered() {
    let mut board = board();
    let older = entry(Some("KAN-1"), None);
    let mut newer = entry(Some("KAN-2"), None);
    newer.question.question_id = "call-2".into();
    newer.question.asked_at += 60_000;
    questions::mirror(&mut board, &older, None, AT).unwrap();
    questions::mirror(&mut board, &newer, None, LATER).unwrap();
    let page = questions::list(
        &board,
        &ListInput {
            state: Some("open".into()),
            limit: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    let mut settled = entry(Some("KAN-2"), Some("answered"));
    settled.question.question_id = "call-2".into();
    questions::mirror(&mut board, &settled, None, LATER).unwrap();
    let next = questions::list(
        &board,
        &ListInput {
            state: Some("open".into()),
            cursor: page.cursor,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(next.count, 1);
    assert_eq!(
        next.questions[0]
            .comment
            .question
            .as_ref()
            .unwrap()
            .question_id,
        "call-1"
    );
}
