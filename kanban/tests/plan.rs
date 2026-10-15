//! Subtasks, projects and releases: the ticket relations and their
//! invariants, the derived views, shipping, the label conversion, the board
//! file versions and the trigger filters — engine-free, like `tests/board.rs`.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use iii_kanban::board::{
    BoardContext, CreateCommentInput, CreateTicketInput, TicketFilter, TicketView, UpdatePatch,
    UpdateResult, create_comment, create_ticket, delete_ticket, event_releases, find_ticket,
    list_tickets, move_ticket, restore_ticket, ticket_view, update_ticket,
};
use iii_kanban::config::{Column, KanbanConfig};
use iii_kanban::events::{
    ChangeConfig, ChangeEvent, CommentConfig, CommentEvent, matches_change, matches_comment,
    payloads,
};
use iii_kanban::functions::{
    self, BoardGetInput, BoardView, LabelConvertInput, ReleaseShipInput, TicketCreateInput,
    TicketUpdateInput, board_view, change_events, ticket_outcome, views,
};
use iii_kanban::plan::{
    self, ConvertInput, ConvertInto, EntityView, ProjectFields, ProjectStatus, ReleaseFields,
    ReleaseStatus, natural_cmp,
};
use iii_kanban::store::{Board, BoardStore, Change, Ticket, read_board, write_board};
use serde_json::{Value, json};

/* ── helpers ──────────────────────────────────────────────────────────── */

fn context() -> BoardContext {
    BoardContext {
        prefix: "KAN".into(),
        default_status: "backlog".into(),
        done_status: "done".into(),
        canceled_status: None,
        columns: [
            ("backlog", "Backlog"),
            ("todo", "To do"),
            ("review", "In review"),
            ("done", "Done"),
        ]
        .into_iter()
        .map(|(id, label)| Column {
            id: id.into(),
            label: label.into(),
        })
        .collect(),
        priorities: ["low", "medium", "high"]
            .into_iter()
            .map(String::from)
            .collect(),
    }
}

fn clock(tick: u32) -> String {
    format!("2026-01-01T{:02}:{:02}:00.000Z", tick / 60, tick % 60)
}

/// A board plus a clock that moves one minute per write.
struct Env {
    board: Board,
    tick: u32,
}

impl Env {
    fn new() -> Self {
        Self {
            board: Board::empty(),
            tick: 0,
        }
    }

    fn at(&mut self) -> String {
        self.tick += 1;
        clock(self.tick)
    }

    fn create(&mut self, input: CreateTicketInput) -> Ticket {
        let at = self.at();
        create_ticket(&mut self.board, &input, &context(), &at).expect("created")
    }

    fn ticket(&mut self, title: &str) -> Ticket {
        self.create(CreateTicketInput {
            title: title.into(),
            ..CreateTicketInput::default()
        })
    }

    fn ticket_in(&mut self, title: &str, status: &str, labels: &[&str]) -> Ticket {
        self.create(CreateTicketInput {
            title: title.into(),
            status: Some(status.into()),
            labels: Some(labels.iter().map(|label| label.to_string()).collect()),
            ..CreateTicketInput::default()
        })
    }

    fn update(&mut self, key: &str, patch: UpdatePatch) -> Result<UpdateResult, String> {
        let at = self.at();
        update_ticket(&mut self.board, key, &patch, "user", &context(), &at)
            .map(|result| result.expect("ticket exists"))
    }

    fn get(&self, key: &str) -> Ticket {
        find_ticket(&self.board, key)
            .cloned()
            .expect("ticket exists")
    }

    fn view(&self, key: &str) -> TicketView {
        ticket_view(&self.board, &self.get(key), "done", None)
    }

    fn project(&mut self, name: &str) -> plan::ProjectView {
        let at = self.at();
        let fields = ProjectFields {
            name: Some(name.into()),
            ..ProjectFields::default()
        };
        plan::create_project(&mut self.board, &fields, &context(), &at).expect("project")
    }

    fn release(&mut self, name: &str, target: Option<&str>) -> plan::ReleaseView {
        let at = self.at();
        let fields = ReleaseFields {
            name: Some(name.into()),
            target_at: target.map(|day| Some(day.to_string())),
            ..ReleaseFields::default()
        };
        plan::create_release(&mut self.board, &fields, &context(), &at).expect("release")
    }

    fn release_view(&self, name: &str) -> plan::ReleaseView {
        plan::release_views(&self.board, &context())
            .into_iter()
            .find(|view| view.release.name == name)
            .expect("release exists")
    }

    fn project_view(&self, name: &str) -> plan::ProjectView {
        plan::project_views(&self.board, &context(), true)
            .into_iter()
            .find(|view| view.project.name == name)
            .expect("project exists")
    }

    fn set(&mut self, key: &str, project: Option<&str>, release: Option<&str>) {
        self.update(
            key,
            UpdatePatch {
                project: project.map(|name| Some(name.to_string())),
                release: release.map(|name| Some(name.to_string())),
                ..UpdatePatch::default()
            },
        )
        .expect("filed");
    }

    fn status(&mut self, key: &str, status: &str) {
        self.update(
            key,
            UpdatePatch {
                status: Some(status.into()),
                ..UpdatePatch::default()
            },
        )
        .expect("moved");
    }

    fn delete(&mut self, key: &str) {
        let at = self.at();
        delete_ticket(&mut self.board, key, "user", &at).expect("deleted");
    }

    fn ship(
        &mut self,
        release: &str,
        move_open_to: Option<Option<&str>>,
    ) -> Result<(plan::ReleaseView, Vec<String>), String> {
        let at = self.at();
        plan::ship_release(
            &mut self.board,
            release,
            move_open_to.map(|target| target.map(String::from)),
            &context(),
            &at,
        )
    }
}

fn parent_patch(parent: Option<&str>) -> UpdatePatch {
    UpdatePatch {
        parent: Some(parent.map(String::from)),
        ..UpdatePatch::default()
    }
}

fn keys(tickets: Vec<&Ticket>) -> Vec<String> {
    tickets
        .into_iter()
        .map(|ticket| ticket.key.clone())
        .collect()
}

fn strs(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn change(field: &str, from: Value, to: Value) -> Change {
    Change {
        field: field.into(),
        from,
        to,
    }
}

fn assert_untouched(before: &Board, after: &Board) {
    assert_eq!(before.tickets, after.tickets);
    assert_eq!(before.projects, after.projects);
    assert_eq!(before.releases, after.releases);
    assert_eq!(before.next_number, after.next_number);
}

fn convert_input(label: &str, into: ConvertInto, name: Option<&str>) -> ConvertInput {
    ConvertInput {
        label: label.into(),
        into,
        id: None,
        name: name.map(String::from),
        description: None,
        target_at: None,
        replace: false,
        remove_label: false,
        mark_released_if_done: false,
    }
}

/// What `kanban::label::convert` does under the store lock.
fn convert(
    env: &mut Env,
    input: &ConvertInput,
) -> Result<(plan::Buckets, plan::Converted), String> {
    let at = env.at();
    let planned = plan::plan_convert(&env.board, input)?;
    let buckets = planned.buckets(&env.board);
    let converted = plan::apply_convert(&mut env.board, &planned, input, &context(), &at);
    Ok((buckets, converted))
}

fn release_of(entity: &EntityView) -> &plan::ReleaseView {
    match entity {
        EntityView::Release(view) => view,
        EntityView::Project(_) => panic!("a release was expected"),
    }
}

fn project_of(entity: &EntityView) -> &plan::ProjectView {
    match entity {
        EntityView::Project(view) => view,
        EntityView::Release(_) => panic!("a project was expected"),
    }
}

/* ── board file versions ──────────────────────────────────────────────── */

const V1_BOARD: &str = r#"{
  "version": 1,
  "next_number": 2,
  "tickets": {
    "t1": {
      "id": "t1", "key": "KAN-1", "title": "Old", "status": "backlog",
      "priority": "low", "created_at": "2025-01-01T00:00:00.000Z",
      "updated_at": "2025-01-01T00:00:00.000Z"
    }
  }
}"#;

#[tokio::test]
async fn a_v1_board_loads_with_empty_relations_and_is_written_back_as_v2() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    std::fs::write(&file, V1_BOARD).unwrap();

    let board = read_board(&file).await.unwrap();
    assert_eq!(board.version, 2);
    assert!(board.projects.is_empty() && board.releases.is_empty());
    let old = &board.tickets["t1"];
    assert_eq!(
        (&old.parent_id, &old.project_id, &old.release_id),
        (&None, &None, &None)
    );

    write_board(&file, &board).await.unwrap();
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(raw["version"], 2);
    assert_eq!(raw["projects"], json!({}));
    assert_eq!(raw["releases"], json!({}));
    for field in ["parent_id", "project_id", "release_id"] {
        assert_eq!(
            raw["tickets"]["t1"][field],
            Value::Null,
            "{field} is always serialized"
        );
    }
}

#[tokio::test]
async fn a_board_from_a_newer_worker_is_refused_and_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    // Not even shaped like a v2 board: the version is what the error names.
    let newer = r#"{"version": 3, "next_number": 9, "tickets": [1, 2, 3]}"#;
    std::fs::write(&file, newer).unwrap();

    let error = read_board(&file).await.unwrap_err();
    assert!(error.contains("version 3"), "{error}");
    assert!(error.contains("version 2"), "{error}");

    let config = KanbanConfig {
        data_path: dir.path().display().to_string(),
        ..KanbanConfig::default()
    };
    let store = BoardStore::new(Arc::new(RwLock::new(config)));
    let written = store
        .update(|board, _| {
            board.next_number += 1;
            Ok(())
        })
        .await
        .unwrap_err();
    assert!(written.contains("version 3"), "{written}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), newer);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn the_first_write_over_a_v1_board_keeps_a_copy_and_never_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("board.json");
    let copy = dir.path().join("board.v1.json");
    std::fs::write(&file, V1_BOARD).unwrap();
    let config = KanbanConfig {
        data_path: dir.path().display().to_string(),
        ..KanbanConfig::default()
    };
    let store = BoardStore::new(Arc::new(RwLock::new(config)));

    // A mutation that fails writes nothing, so it keeps nothing either.
    store
        .update(|_, _| Err::<(), _>("refused".to_string()))
        .await
        .unwrap_err();
    assert!(!copy.exists());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), V1_BOARD);

    store
        .update(|board, _| {
            board.next_number += 1;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&copy).unwrap(),
        V1_BOARD,
        "the copy is the file as the old worker wrote it"
    );
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(raw["version"], 2);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no temp file is left behind"
    );

    // Later writes leave the copy alone; so does a second trip through v1.
    std::fs::write(&copy, "sentinel").unwrap();
    for _ in 0..2 {
        store.update(|_, _| Ok(())).await.unwrap();
        std::fs::write(&file, V1_BOARD).unwrap();
        store.update(|_, _| Ok(())).await.unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "sentinel");
    }
}

#[tokio::test]
async fn a_new_or_current_board_keeps_no_copy() {
    let dir = tempfile::tempdir().unwrap();
    let config = KanbanConfig {
        data_path: dir.path().display().to_string(),
        ..KanbanConfig::default()
    };
    let store = BoardStore::new(Arc::new(RwLock::new(config)));
    for _ in 0..2 {
        // The first write creates the file, the second rewrites a v2 one.
        store.update(|_, _| Ok(())).await.unwrap();
    }
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "only board.json"
    );
}

/* ── subtasks ─────────────────────────────────────────────────────────── */

#[test]
fn attaching_a_subtask_writes_activity_on_both_tickets_and_bumps_both() {
    let mut env = Env::new();
    env.ticket("Parent");
    env.ticket("Child");
    let at = env.at();
    let result = update_ticket(
        &mut env.board,
        "KAN-2",
        &parent_patch(Some("KAN-1")),
        "tech-lead",
        &context(),
        &at,
    )
    .unwrap()
    .unwrap();
    let (parent, child) = (env.get("KAN-1"), env.get("KAN-2"));

    assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(
        result.changes,
        [change("parent", Value::Null, json!("KAN-1"))]
    );
    assert_eq!(result.related, std::slice::from_ref(&parent.id));

    let on_child = child.activity.last().unwrap();
    assert_eq!(on_child.kind, "ticket.updated");
    assert_eq!(on_child.actor, "tech-lead");
    assert_eq!(on_child.changes.as_deref(), Some(&result.changes[..]));

    let on_parent = parent.activity.last().unwrap();
    assert_eq!(on_parent.kind, "subtask.added");
    assert_eq!(on_parent.actor, "tech-lead");
    assert_eq!(
        on_parent.changes.as_deref(),
        Some(&[change("subtask", Value::Null, json!("KAN-2"))][..])
    );
    assert_eq!(on_parent.at, at);
    assert_eq!((&parent.updated_at, &child.updated_at), (&at, &at));
    assert_eq!(on_parent.seq, 2, "the parent's timeline continues");
}

#[test]
fn detaching_writes_the_removal_on_both_and_null_or_blank_detaches() {
    for detach in [None, Some("  ")] {
        let mut env = Env::new();
        env.ticket("Parent");
        env.ticket("Child");
        env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
        let result = env.update("KAN-2", parent_patch(detach)).unwrap();
        let (parent, child) = (env.get("KAN-1"), env.get("KAN-2"));

        assert_eq!(child.parent_id, None);
        assert_eq!(
            result.changes,
            [change("parent", json!("KAN-1"), Value::Null)]
        );
        assert_eq!(result.related, std::slice::from_ref(&parent.id));
        let on_parent = parent.activity.last().unwrap();
        assert_eq!(on_parent.kind, "subtask.removed");
        assert_eq!(
            on_parent.changes.as_deref(),
            Some(&[change("subtask", json!("KAN-2"), Value::Null)][..])
        );
        assert_eq!(parent.updated_at, child.updated_at);
        assert!(env.view("KAN-1").children.is_empty());
    }
}

#[test]
fn moving_a_subtask_to_another_parent_writes_three_tickets() {
    let mut env = Env::new();
    for title in ["First parent", "Second parent", "Child"] {
        env.ticket(title);
    }
    env.update("KAN-3", parent_patch(Some("KAN-1"))).unwrap();
    let result = env.update("KAN-3", parent_patch(Some("KAN-2"))).unwrap();

    assert_eq!(
        result.changes,
        [change("parent", json!("KAN-1"), json!("KAN-2"))]
    );
    assert_eq!(result.related.len(), 2);
    assert_eq!(
        env.get("KAN-1").activity.last().unwrap().kind,
        "subtask.removed"
    );
    assert_eq!(
        env.get("KAN-2").activity.last().unwrap().kind,
        "subtask.added"
    );
    let times = [env.get("KAN-1"), env.get("KAN-2"), env.get("KAN-3")].map(|t| t.updated_at);
    assert!(times.iter().all(|time| *time == times[0]), "{times:?}");
}

#[test]
fn the_parent_is_found_by_key_in_any_case_or_by_uuid() {
    let mut env = Env::new();
    let parent = env.ticket("Parent");
    for (title, reference) in [("a", "kan-1"), ("b", "KAN-1"), ("c", parent.id.as_str())] {
        env.ticket(title);
        let key = format!("KAN-{}", env.board.next_number - 1);
        env.update(&key, parent_patch(Some(reference))).unwrap();
        assert_eq!(env.get(&key).parent_id.as_deref(), Some(parent.id.as_str()));
    }
    assert_eq!(env.view("KAN-1").children.len(), 3);
}

#[test]
fn a_subtask_keeps_to_one_level_a_live_parent_and_never_itself() {
    let mut env = Env::new();
    for title in ["Parent", "Child", "Loner", "Gone"] {
        env.ticket(title);
    }
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    env.delete("KAN-4");

    for (key, parent, expected) in [
        ("KAN-3", "KAN-404", "unknown parent: KAN-404"),
        ("KAN-3", "KAN-3", "cannot be its own parent"),
        ("KAN-3", "KAN-4", "parent KAN-4 is deleted"),
        ("KAN-3", "KAN-2", "KAN-2 is itself a subtask"),
        ("KAN-1", "KAN-3", "KAN-1 has subtasks and cannot become one"),
    ] {
        let before = env.board.clone();
        let error = env.update(key, parent_patch(Some(parent))).unwrap_err();
        assert!(error.contains(expected), "{key} -> {parent}: {error}");
        assert_untouched(&before, &env.board);
    }

    // Subtasks that are deleted do not make a parent of their parent.
    env.delete("KAN-2");
    env.update("KAN-1", parent_patch(Some("KAN-3")))
        .expect("only live subtasks block it");
    assert_eq!(env.get("KAN-1").parent_id, Some(env.get("KAN-3").id));
}

#[test]
fn a_rejected_patch_changes_nothing_even_when_other_fields_were_valid() {
    let mut env = Env::new();
    env.ticket("Parent");
    env.ticket("Other");
    let before = env.board.clone();
    let at = env.at();
    let error = update_ticket(
        &mut env.board,
        "KAN-2",
        &UpdatePatch {
            title: Some("Renamed".into()),
            parent: Some(Some("KAN-404".into())),
            ..UpdatePatch::default()
        },
        "user",
        &context(),
        &at,
    )
    .unwrap_err();
    assert!(error.contains("unknown parent"));
    assert_untouched(&before, &env.board);
}

#[test]
fn setting_the_same_parent_again_records_nothing() {
    let mut env = Env::new();
    env.ticket("Parent");
    env.ticket("Child");
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    let before = env.board.clone();
    let result = env.update("KAN-2", parent_patch(Some("kan-1"))).unwrap();
    assert!(result.changes.is_empty() && result.related.is_empty());
    assert_untouched(&before, &env.board);

    // Neither does re-sending a parent that has since been deleted.
    env.delete("KAN-1");
    let before = env.board.clone();
    let result = env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    assert!(result.changes.is_empty());
    assert_untouched(&before, &env.board);
}

#[test]
fn creating_a_subtask_files_it_under_the_parent_and_copies_nothing() {
    let mut env = Env::new();
    env.project("iii cloud");
    env.release("Phase 2", None);
    env.ticket("Parent");
    env.set("KAN-1", Some("iii cloud"), Some("Phase 2"));
    let at = env.at();
    let child = create_ticket(
        &mut env.board,
        &CreateTicketInput {
            title: "Child".into(),
            parent: Some("KAN-1".into()),
            ..CreateTicketInput::default()
        },
        &context(),
        &at,
    )
    .unwrap();

    assert_eq!(child.parent_id, Some(env.get("KAN-1").id));
    assert_eq!(
        (&child.project_id, &child.release_id),
        (&None, &None),
        "no inheritance: the console prefills, the worker does not"
    );
    assert_eq!(
        child.activity[0].changes.as_deref(),
        Some(
            &[
                change("status", Value::Null, json!("backlog")),
                change("parent", Value::Null, json!("KAN-1")),
            ][..]
        ),
        "the status change stays first"
    );
    let parent = env.get("KAN-1");
    assert_eq!(parent.activity.last().unwrap().kind, "subtask.added");
    assert_eq!(parent.updated_at, at);
}

#[test]
fn creating_with_a_bad_parent_creates_nothing() {
    let mut env = Env::new();
    env.ticket("Parent");
    env.ticket("Child");
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    env.delete("KAN-1");
    env.ticket("Live");
    for (parent, expected) in [
        ("KAN-404", "unknown parent"),
        ("KAN-1", "is deleted"),
        ("KAN-2", "itself a subtask"),
    ] {
        let before = env.board.clone();
        let at = env.at();
        let error = create_ticket(
            &mut env.board,
            &CreateTicketInput {
                title: "Nope".into(),
                parent: Some(parent.into()),
                ..CreateTicketInput::default()
            },
            &context(),
            &at,
        )
        .unwrap_err();
        assert!(error.contains(expected), "{parent}: {error}");
        assert_untouched(&before, &env.board);
    }
}

#[test]
fn the_view_carries_the_parent_the_live_children_by_number_and_the_progress() {
    let mut env = Env::new();
    for title in ["Parent", "A", "B", "C", "D"] {
        env.ticket(title);
    }
    // Attached out of order: the view sorts by ticket number.
    for key in ["KAN-4", "KAN-2", "KAN-5", "KAN-3"] {
        env.update(key, parent_patch(Some("KAN-1"))).unwrap();
    }
    env.status("KAN-2", "done");
    env.status("KAN-3", "review");
    env.status("KAN-4", "done");
    env.delete("KAN-4");

    let parent = env.view("KAN-1");
    let order: Vec<&str> = parent.children.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(order, ["KAN-2", "KAN-3", "KAN-5"], "live only, by number");
    assert_eq!(
        (parent.progress.done, parent.progress.total),
        (1, 3),
        "review is not done; the deleted child is excluded"
    );
    assert_eq!(parent.parent, None);

    let child = env.view("KAN-3");
    let brief = child.parent.as_ref().expect("a parent brief");
    assert_eq!(
        (brief.key.as_str(), brief.title.as_str()),
        ("KAN-1", "Parent")
    );
    assert_eq!((child.progress.done, child.progress.total), (0, 0));
    assert!(child.children.is_empty());

    let strict = ticket_view(&env.board, &env.get("KAN-1"), "review", None);
    assert_eq!(
        (strict.progress.done, strict.progress.total),
        (1, 3),
        "only the configured done column counts"
    );
}

#[test]
fn deleting_a_parent_leaves_the_children_and_the_brief_says_deleted() {
    let mut env = Env::new();
    env.ticket("Parent");
    env.ticket("Child");
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    let child_before = env.get("KAN-2");
    env.delete("KAN-1");

    assert_eq!(env.get("KAN-2"), child_before, "the child is untouched");
    let view = env.view("KAN-2");
    let parent = view.parent.expect("still points at the parent");
    assert!(parent.deleted_at.is_some());
    assert_eq!(parent.key, "KAN-1");
}

#[test]
fn a_subtask_can_be_filed_under_its_own_project_and_release() {
    let mut env = Env::new();
    env.project("iii cloud");
    env.project("package");
    env.release("Phase 2", None);
    env.ticket("Parent");
    env.ticket("Child");
    env.set("KAN-1", Some("iii cloud"), Some("Phase 2"));
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    env.set("KAN-2", Some("package"), None);
    assert_eq!(
        env.view("KAN-1").children[0].project_id,
        env.get("KAN-2").project_id
    );
    assert_ne!(env.get("KAN-1").project_id, env.get("KAN-2").project_id);
    assert_eq!(env.get("KAN-2").release_id, None);
}

fn assert_one_level(board: &Board) {
    for ticket in board.tickets.values() {
        if let Some(parent) = ticket
            .parent_id
            .as_deref()
            .and_then(|id| board.tickets.get(id))
        {
            assert_eq!(
                parent.parent_id, None,
                "{} is a subtask of {}, which is a subtask itself",
                ticket.key, parent.key
            );
        }
    }
}

#[test]
fn restoring_a_subtask_whose_parent_became_a_subtask_detaches_it_on_both_sides() {
    let mut env = Env::new();
    for title in ["Q", "P", "C"] {
        env.ticket(title);
    }
    env.update("KAN-3", parent_patch(Some("KAN-2"))).unwrap();
    env.delete("KAN-3");
    // C is deleted, so P has no live subtask and may become one itself.
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    let before = env.get("KAN-3");
    let parent_id = env.get("KAN-2").id;

    let at = env.at();
    let restored = restore_ticket(&mut env.board, "KAN-3", "user", &at).unwrap();
    assert_one_level(&env.board);
    assert_eq!(restored.ticket.deleted_at, None);
    assert_eq!(restored.ticket.parent_id, None, "it comes back on its own");
    assert_eq!(restored.ticket.updated_at, at);
    assert_eq!(restored.related, [parent_id]);
    assert_eq!(
        restored.changes,
        [change("parent", json!("KAN-2"), Value::Null)]
    );

    // The child: the detach as a normal detach writes it, then the restore (last, it names the event).
    let tail: Vec<(&str, Option<&Vec<Change>>)> = restored.ticket.activity[before.activity.len()..]
        .iter()
        .map(|entry| (entry.kind.as_str(), entry.changes.as_ref()))
        .collect();
    assert_eq!(
        tail,
        [
            ("ticket.updated", Some(&restored.changes)),
            ("ticket.restored", None)
        ]
    );
    // The parent: the removal, and its version moved.
    let parent = env.get("KAN-2");
    let removal = parent.activity.last().unwrap();
    assert_eq!(removal.kind, "subtask.removed");
    assert_eq!(
        removal.changes,
        Some(vec![change("subtask", json!("KAN-3"), Value::Null)])
    );
    assert_eq!(parent.updated_at, at);
    assert_eq!(
        parent.parent_id,
        Some(env.get("KAN-1").id),
        "P stays under Q"
    );

    // Both are announced, the restored ticket first.
    let outcome = ticket_outcome(
        &env.board,
        &context(),
        Some(before),
        Some(restored.ticket),
        &restored.related,
    );
    let announced: Vec<&str> = outcome
        .tickets
        .iter()
        .map(|view| view.ticket.key.as_str())
        .collect();
    assert_eq!(announced, ["KAN-3", "KAN-2"]);
    assert!(outcome.tickets[0].parent.is_none());
    assert!(outcome.tickets[1].children.is_empty());
}

#[test]
fn restoring_a_subtask_under_a_deleted_or_missing_parent_detaches_it() {
    let mut env = Env::new();
    for title in ["P", "Child", "Orphan"] {
        env.ticket(title);
    }
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    env.delete("KAN-2");
    env.delete("KAN-1");

    let at = env.at();
    let restored = restore_ticket(&mut env.board, "KAN-2", "user", &at).unwrap();
    assert_eq!(restored.ticket.parent_id, None, "the parent is deleted");
    assert_eq!(restored.related, [env.get("KAN-1").id]);
    assert_eq!(
        env.get("KAN-1").activity.last().unwrap().kind,
        "subtask.removed",
        "a deleted parent gets the removal too"
    );

    // A parent id the board does not know (a hand-edited file).
    env.delete("KAN-3");
    env.board
        .tickets
        .get_mut(&env.get("KAN-3").id)
        .unwrap()
        .parent_id = Some("ghost".into());
    let at = env.at();
    let restored = restore_ticket(&mut env.board, "KAN-3", "user", &at).unwrap();
    assert_eq!(restored.ticket.parent_id, None);
    assert_eq!(restored.related, ["ghost"]);
    assert_eq!(restored.changes[0].from, json!("ghost"));
    assert!(
        views(&env.board, &context(), &restored.related).is_empty(),
        "an unknown parent announces nothing"
    );
}

#[test]
fn restoring_keeps_the_parent_the_project_and_the_release_when_the_parent_still_fits() {
    let mut env = Env::new();
    env.project("cloud");
    env.release("Phase 2", None);
    env.ticket("Parent");
    env.ticket("Child");
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    env.set("KAN-2", Some("cloud"), Some("Phase 2"));
    env.delete("KAN-2");
    let parent_before = env.get("KAN-1");

    let at = env.at();
    let restored = restore_ticket(&mut env.board, "KAN-2", "user", &at).unwrap();
    assert_eq!(restored.ticket.parent_id, Some(parent_before.id.clone()));
    assert_eq!(restored.ticket.project_id, env.get("KAN-2").project_id);
    assert!(restored.ticket.project_id.is_some() && restored.ticket.release_id.is_some());
    assert!(restored.changes.is_empty() && restored.related.is_empty());
    assert_eq!(
        restored.ticket.activity.last().unwrap().kind,
        "ticket.restored"
    );
    assert_eq!(env.get("KAN-1"), parent_before, "the parent is not written");
    assert_eq!(env.view("KAN-1").children[0].key, "KAN-2");

    // Restoring a live ticket changes nothing and names no one else.
    let at = env.at();
    let again = restore_ticket(&mut env.board, "KAN-2", "user", &at).unwrap();
    assert_eq!(again.ticket, restored.ticket);
    assert!(again.changes.is_empty() && again.related.is_empty());
}

#[test]
fn a_deleted_ticket_cannot_become_a_subtask_but_can_be_detached() {
    let mut env = Env::new();
    for title in ["Parent", "Gone", "Child"] {
        env.ticket(title);
    }
    env.update("KAN-3", parent_patch(Some("KAN-1"))).unwrap();
    env.delete("KAN-2");
    env.delete("KAN-3");

    let before = env.board.clone();
    let error = env
        .update("KAN-2", parent_patch(Some("KAN-1")))
        .unwrap_err();
    assert!(error.contains("KAN-2 is deleted"), "{error}");
    assert_untouched(&before, &env.board);

    // Detaching a deleted subtask is the way out of a parent that cannot hold it.
    let detached = env.update("KAN-3", parent_patch(None)).unwrap();
    assert_eq!(detached.ticket.parent_id, None);
    assert_eq!(detached.related, [env.get("KAN-1").id]);
    assert_eq!(
        env.get("KAN-1").activity.last().unwrap().kind,
        "subtask.removed"
    );
}

/* ── ticket project and release ───────────────────────────────────────── */

#[test]
fn project_and_release_are_set_by_id_or_name_and_recorded_with_names() {
    let mut env = Env::new();
    let cloud = env.project("iii cloud");
    env.release("Phase 2", None);
    env.ticket("One");
    env.ticket("Two");

    let first = env
        .update(
            "KAN-1",
            UpdatePatch {
                project: Some(Some("III CLOUD".into())),
                release: Some(Some(" phase 2 ".into())),
                ..UpdatePatch::default()
            },
        )
        .unwrap();
    assert_eq!(
        first.changes,
        [
            change("project", Value::Null, json!("iii cloud")),
            change("release", Value::Null, json!("Phase 2")),
        ]
    );
    assert!(first.related.is_empty());
    env.update(
        "KAN-2",
        UpdatePatch {
            project: Some(Some(cloud.project.id.clone())),
            ..UpdatePatch::default()
        },
    )
    .unwrap();
    assert_eq!(
        env.get("KAN-2").project_id.as_deref(),
        Some(cloud.project.id.as_str())
    );

    // Moving and clearing record the previous name.
    env.project("package");
    let moved = env
        .update(
            "KAN-1",
            UpdatePatch {
                project: Some(Some("package".into())),
                release: Some(None),
                ..UpdatePatch::default()
            },
        )
        .unwrap();
    assert_eq!(
        moved.changes,
        [
            change("project", json!("iii cloud"), json!("package")),
            change("release", json!("Phase 2"), Value::Null),
        ]
    );
    let again = env
        .update(
            "KAN-1",
            UpdatePatch {
                project: Some(Some("package".into())),
                release: Some(None),
                ..UpdatePatch::default()
            },
        )
        .unwrap();
    assert!(again.changes.is_empty(), "the same relation is no change");
}

#[test]
fn an_unknown_project_or_release_is_an_error_and_a_released_one_takes_tickets() {
    let mut env = Env::new();
    env.release("Phase 1", None);
    env.ticket("One");
    env.ship("Phase 1", None).unwrap();
    for (patch, expected) in [
        (
            UpdatePatch {
                project: Some(Some("ghost".into())),
                ..UpdatePatch::default()
            },
            "unknown project: ghost",
        ),
        (
            UpdatePatch {
                release: Some(Some("ghost".into())),
                ..UpdatePatch::default()
            },
            "unknown release: ghost",
        ),
    ] {
        let before = env.board.clone();
        let error = env.update("KAN-1", patch).unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert_untouched(&before, &env.board);
    }
    env.set("KAN-1", None, Some("Phase 1"));
    assert_eq!(
        env.release_view("Phase 1").reopened,
        1,
        "not done inside a shipped release"
    );
}

#[test]
fn an_archived_project_refuses_new_references_but_keeps_the_old_ones() {
    let mut env = Env::new();
    env.project("old");
    env.ticket("Inside");
    env.ticket("Outside");
    env.set("KAN-1", Some("old"), None);
    let at = env.at();
    plan::set_project_archived(&mut env.board, "old", true, &context(), &at).unwrap();

    let before = env.board.clone();
    let error = env
        .update(
            "KAN-2",
            UpdatePatch {
                project: Some(Some("old".into())),
                ..UpdatePatch::default()
            },
        )
        .unwrap_err();
    assert!(error.contains("project old is archived"), "{error}");
    let at = env.at();
    let error = create_ticket(
        &mut env.board,
        &CreateTicketInput {
            title: "New".into(),
            project: Some("old".into()),
            ..CreateTicketInput::default()
        },
        &context(),
        &at,
    )
    .unwrap_err();
    assert!(error.contains("archived"), "{error}");
    assert_untouched(&before, &env.board);

    // The ticket that was already in it can be edited and can leave.
    let kept = env
        .update(
            "KAN-1",
            UpdatePatch {
                project: Some(Some("old".into())),
                title: Some("Still inside".into()),
                ..UpdatePatch::default()
            },
        )
        .unwrap();
    assert_eq!(kept.changes.len(), 1, "only the title changed");
    env.update(
        "KAN-1",
        UpdatePatch {
            project: Some(None),
            ..UpdatePatch::default()
        },
    )
    .unwrap();
    assert_eq!(env.get("KAN-1").project_id, None);
}

#[test]
fn a_name_cannot_spell_another_entitys_id_and_ids_are_found_in_any_case() {
    let mut env = Env::new();
    let cloud = env.project("cloud");
    env.project("package");
    let phase = env.release("Phase 2", None);
    env.release("Phase 3", None);
    let (project_id, release_id) = (cloud.project.id.clone(), phase.release.id.clone());

    let at = env.at();
    for name in [project_id.clone(), project_id.to_uppercase()] {
        let fields = ProjectFields {
            name: Some(name.clone()),
            ..ProjectFields::default()
        };
        let error = plan::create_project(&mut env.board, &fields, &context(), &at).unwrap_err();
        assert!(error.contains("id of another project"), "{name}: {error}");
        let error =
            plan::update_project(&mut env.board, "package", &fields, &context(), &at).unwrap_err();
        assert!(error.contains("id of another project"), "{name}: {error}");
    }
    for name in [release_id.clone(), release_id.to_uppercase()] {
        let fields = ReleaseFields {
            name: Some(name.clone()),
            ..ReleaseFields::default()
        };
        let error = plan::create_release(&mut env.board, &fields, &context(), &at).unwrap_err();
        assert!(error.contains("id of another release"), "{name}: {error}");
        let error =
            plan::update_release(&mut env.board, "Phase 3", &fields, &context(), &at).unwrap_err();
        assert!(error.contains("id of another release"), "{name}: {error}");
    }
    assert!(env.board.projects.values().all(|p| p.name != project_id));

    // The id finds its entity whatever the case, as the name does.
    env.ticket("One");
    env.set(
        "KAN-1",
        Some(&project_id.to_uppercase()),
        Some(&release_id.to_uppercase()),
    );
    let ticket = env.get("KAN-1");
    assert_eq!(ticket.project_id.as_deref(), Some(project_id.as_str()));
    assert_eq!(ticket.release_id.as_deref(), Some(release_id.as_str()));
    let filter = TicketFilter {
        project_id: Some(project_id.to_uppercase()),
        release_id: Some(release_id.to_uppercase()),
        ..TicketFilter::default()
    };
    assert_eq!(keys(list_tickets(&env.board, &filter)), ["KAN-1"]);
    let renamed = plan::update_project(
        &mut env.board,
        &project_id.to_uppercase(),
        &ProjectFields {
            description: Some("by upper-case id".into()),
            ..ProjectFields::default()
        },
        &context(),
        &at,
    )
    .unwrap();
    assert_eq!(renamed.project.id, project_id);
}

/* ── projects ─────────────────────────────────────────────────────────── */

#[test]
fn project_names_are_required_unique_and_colors_are_checked() {
    let mut env = Env::new();
    let created = env.project("  iii cloud  ");
    assert_eq!(created.project.name, "iii cloud");
    assert_eq!(created.status, ProjectStatus::Active);
    assert_eq!((created.total, created.done), (0, 0));

    let at = env.at();
    let create = |env: &mut Env, fields: ProjectFields| {
        plan::create_project(&mut env.board, &fields, &context(), &at)
    };
    for (fields, expected) in [
        (ProjectFields::default(), "project name is required"),
        (
            ProjectFields {
                name: Some("   ".into()),
                ..ProjectFields::default()
            },
            "project name is required",
        ),
        (
            ProjectFields {
                name: Some("III Cloud".into()),
                ..ProjectFields::default()
            },
            "already exists",
        ),
        (
            ProjectFields {
                name: Some("fresh".into()),
                color: Some(Some("magenta".into())),
                ..ProjectFields::default()
            },
            "unknown color: magenta",
        ),
    ] {
        let error = create(&mut env, fields).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    assert_eq!(env.board.projects.len(), 1);

    let full = create(
        &mut env,
        ProjectFields {
            name: Some("package".into()),
            description: Some(" Binaries ".into()),
            lead: Some(Some("tech-lead".into())),
            color: Some(Some("teal".into())),
        },
    )
    .unwrap();
    assert_eq!(full.project.description, "Binaries");
    assert_eq!(full.project.lead.as_deref(), Some("tech-lead"));
    assert_eq!(full.project.color.as_deref(), Some("teal"));
    assert_eq!(full.project.source_label, None);
}

#[test]
fn updating_a_project_renames_clears_and_leaves_unchanged_alone() {
    let mut env = Env::new();
    let created = env.project("one");
    env.project("two");
    let at = env.at();
    let update = |env: &mut Env, reference: &str, fields: ProjectFields, at: &str| {
        plan::update_project(&mut env.board, reference, &fields, &context(), at)
    };

    let set = update(
        &mut env,
        "ONE",
        ProjectFields {
            lead: Some(Some("user".into())),
            color: Some(Some("rose".into())),
            ..ProjectFields::default()
        },
        &at,
    )
    .unwrap();
    assert_eq!(set.project.updated_at, at);
    assert_eq!(
        (set.project.lead.as_deref(), set.project.color.as_deref()),
        (Some("user"), Some("rose"))
    );

    let later = env.at();
    let same = update(
        &mut env,
        &created.project.id,
        ProjectFields {
            name: Some("ONE".into()),
            lead: Some(Some("user".into())),
            ..ProjectFields::default()
        },
        &later,
    )
    .unwrap();
    assert_eq!(
        (same.project.name.as_str(), same.project.updated_at.as_str()),
        ("ONE", later.as_str()),
        "a rename in case is a change"
    );

    let cleared = env.at();
    let cleared_view = update(
        &mut env,
        "one",
        ProjectFields {
            lead: Some(None),
            color: Some(None),
            ..ProjectFields::default()
        },
        &cleared,
    )
    .unwrap();
    assert_eq!(
        (cleared_view.project.lead, cleared_view.project.color),
        (None, None)
    );

    let untouched = env.at();
    let nothing = update(&mut env, "one", ProjectFields::default(), &untouched).unwrap();
    assert_eq!(nothing.project.updated_at, cleared, "no change, no bump");

    for (fields, expected) in [
        (
            ProjectFields {
                name: Some("Two".into()),
                ..ProjectFields::default()
            },
            "already exists",
        ),
        (
            ProjectFields {
                name: Some(" ".into()),
                ..ProjectFields::default()
            },
            "name is required",
        ),
        (
            ProjectFields {
                color: Some(Some("nope".into())),
                ..ProjectFields::default()
            },
            "unknown color",
        ),
    ] {
        let error = update(&mut env, "one", fields, &untouched).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    let error = update(&mut env, "ghost", ProjectFields::default(), &untouched).unwrap_err();
    assert!(error.contains("project not found: ghost"), "{error}");
}

#[test]
fn archiving_and_restoring_a_project_is_idempotent_and_the_view_counts_live_tickets() {
    let mut env = Env::new();
    env.project("cloud");
    env.ticket_in("A", "done", &[]);
    env.ticket_in("B", "todo", &[]);
    env.ticket_in("C", "done", &[]);
    for key in ["KAN-1", "KAN-2", "KAN-3"] {
        env.set(key, Some("cloud"), None);
    }
    env.delete("KAN-3");

    let view = env.project_view("cloud");
    assert_eq!(
        (view.total, view.done),
        (2, 1),
        "deleted tickets are not counted"
    );
    assert_eq!(
        view.counts,
        [("backlog", 0), ("todo", 1), ("review", 0), ("done", 1)]
            .into_iter()
            .map(|(column, count)| (column.to_string(), count))
            .collect()
    );

    let at = env.at();
    let archived =
        plan::set_project_archived(&mut env.board, "cloud", true, &context(), &at).unwrap();
    assert_eq!(archived.status, ProjectStatus::Archived);
    assert_eq!(archived.project.archived_at.as_deref(), Some(at.as_str()));
    let later = env.at();
    let twice =
        plan::set_project_archived(&mut env.board, "cloud", true, &context(), &later).unwrap();
    assert_eq!(
        twice.project, archived.project,
        "archiving twice changes nothing"
    );
    assert!(plan::project_views(&env.board, &context(), false).is_empty());
    assert_eq!(plan::project_views(&env.board, &context(), true).len(), 1);

    let restored =
        plan::set_project_archived(&mut env.board, "cloud", false, &context(), &later).unwrap();
    assert_eq!(restored.status, ProjectStatus::Active);
    assert_eq!(restored.project.archived_at, None);
    let error =
        plan::set_project_archived(&mut env.board, "ghost", true, &context(), &later).unwrap_err();
    assert!(error.contains("project not found"), "{error}");
}

/* ── releases ─────────────────────────────────────────────────────────── */

#[test]
fn release_names_are_unique_and_the_target_is_a_real_iso_date() {
    let mut env = Env::new();
    env.release("Phase 2", Some("2026-10-30"));
    let at = env.at();
    let create = |env: &mut Env, name: &str, target: Option<&str>| {
        plan::create_release(
            &mut env.board,
            &ReleaseFields {
                name: Some(name.into()),
                target_at: target.map(|day| Some(day.to_string())),
                ..ReleaseFields::default()
            },
            &context(),
            &at,
        )
    };
    assert!(
        create(&mut env, " phase 2 ", None)
            .unwrap_err()
            .contains("already exists")
    );
    assert!(
        create(&mut env, "  ", None)
            .unwrap_err()
            .contains("release name is required")
    );
    for bad in [
        "2026-1-5",
        "2026-02-30",
        "tomorrow",
        "30/10/2026",
        "2026-10-30T10:00",
    ] {
        let error = create(&mut env, "dated", Some(bad)).unwrap_err();
        assert!(error.contains("YYYY-MM-DD"), "{bad}: {error}");
    }
    assert_eq!(env.board.releases.len(), 1);
    // A blank target is no target.
    assert_eq!(
        create(&mut env, "open", Some(" "))
            .unwrap()
            .release
            .target_at,
        None
    );

    let later = env.at();
    let update = |env: &mut Env, fields: ReleaseFields| {
        plan::update_release(&mut env.board, "phase 2", &fields, &context(), &later)
    };
    let renamed = update(
        &mut env,
        ReleaseFields {
            name: Some("Phase Two".into()),
            description: Some(" ship it ".into()),
            target_at: Some(Some("2026-11-01".into())),
        },
    )
    .unwrap();
    assert_eq!(
        (
            renamed.release.name.as_str(),
            renamed.release.description.as_str(),
            renamed.release.target_at.as_deref(),
            renamed.release.updated_at.as_str()
        ),
        ("Phase Two", "ship it", Some("2026-11-01"), later.as_str())
    );
    let error = plan::update_release(
        &mut env.board,
        "Phase Two",
        &ReleaseFields {
            name: Some("OPEN".into()),
            ..ReleaseFields::default()
        },
        &context(),
        &later,
    )
    .unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    let cleared = plan::update_release(
        &mut env.board,
        "Phase Two",
        &ReleaseFields {
            target_at: Some(None),
            ..ReleaseFields::default()
        },
        &context(),
        &later,
    )
    .unwrap();
    assert_eq!(cleared.release.target_at, None);
    assert!(
        plan::update_release(
            &mut env.board,
            "ghost",
            &ReleaseFields::default(),
            &context(),
            &later
        )
        .unwrap_err()
        .contains("release not found: ghost")
    );
}

#[test]
fn release_status_is_derived_planned_active_or_released() {
    let mut env = Env::new();
    env.release("R", None);
    assert_eq!(
        env.release_view("R").status,
        ReleaseStatus::Planned,
        "empty"
    );

    env.ticket_in("A", "backlog", &[]);
    env.ticket_in("B", "todo", &[]);
    env.set("KAN-1", None, Some("R"));
    assert_eq!(
        env.release_view("R").status,
        ReleaseStatus::Planned,
        "everything still in the first column"
    );

    env.set("KAN-2", None, Some("R"));
    let active = env.release_view("R");
    assert_eq!(
        active.status,
        ReleaseStatus::Active,
        "a ticket left the first column"
    );
    assert_eq!((active.total, active.done), (2, 0));

    env.status("KAN-2", "backlog");
    assert_eq!(env.release_view("R").status, ReleaseStatus::Planned);
    env.status("KAN-2", "done");
    env.delete("KAN-2");
    assert_eq!(
        env.release_view("R").status,
        ReleaseStatus::Planned,
        "deleted tickets do not count"
    );

    env.status("KAN-1", "done");
    env.ship("R", None).unwrap();
    let released = env.release_view("R");
    assert_eq!(released.status, ReleaseStatus::Released);
    assert_eq!(released.reopened, 0);
    assert!(released.release.released_at.is_some());
}

#[test]
fn a_release_view_counts_per_column_reopened_tickets_and_projects() {
    let mut env = Env::new();
    env.project("iii cloud");
    env.project("package");
    env.release("R", None);
    for (title, status) in [
        ("a", "done"),
        ("b", "done"),
        ("c", "backlog"),
        ("d", "review"),
        ("e", "done"),
        ("f", "todo"),
    ] {
        env.ticket_in(title, status, &[]);
    }
    for (key, project) in [
        ("KAN-1", Some("iii cloud")),
        ("KAN-2", Some("iii cloud")),
        ("KAN-3", Some("iii cloud")),
        ("KAN-4", Some("package")),
        ("KAN-5", None),
        ("KAN-6", None),
    ] {
        env.set(key, project, Some("R"));
    }
    env.delete("KAN-6");

    let view = env.release_view("R");
    assert_eq!((view.total, view.done), (5, 3));
    assert_eq!(view.counts["done"], 3);
    assert_eq!(view.counts["todo"], 0, "deleted tickets are not counted");
    assert_eq!(
        view.reopened, 0,
        "only a released release has reopened tickets"
    );

    let cloud = env
        .board
        .projects
        .values()
        .find(|p| p.name == "iii cloud")
        .unwrap()
        .id
        .clone();
    let package = env
        .board
        .projects
        .values()
        .find(|p| p.name == "package")
        .unwrap()
        .id
        .clone();
    let shape: Vec<(Option<&str>, u64, u64)> = view
        .projects
        .iter()
        .map(|p| (p.project_id.as_deref(), p.total, p.done))
        .collect();
    assert_eq!(
        shape,
        [
            (Some(cloud.as_str()), 3, 2),
            (Some(package.as_str()), 1, 0),
            (None, 1, 1)
        ],
        "biggest first, ties named projects before none"
    );

    env.ship("R", Some(None)).unwrap();
    env.set("KAN-3", None, Some("R"));
    let shipped = env.release_view("R");
    assert_eq!(shipped.status, ReleaseStatus::Released);
    assert_eq!(
        shipped.reopened, 1,
        "KAN-3 came back after shipping and is not done"
    );
}

#[test]
fn releases_sort_by_target_date_nulls_last_then_by_natural_name() {
    let mut env = Env::new();
    env.release("Phase 10", Some("2026-10-30"));
    env.release("Phase 2", Some("2026-10-30"));
    env.release("Beta", None);
    env.release("Alpha 11", None);
    env.release("Alpha 9", None);
    env.release("Phase 1", Some("2026-09-01"));
    let names: Vec<String> = plan::release_views(&env.board, &context())
        .into_iter()
        .map(|view| view.release.name)
        .collect();
    assert_eq!(
        names,
        [
            "Phase 1", "Phase 2", "Phase 10", "Alpha 9", "Alpha 11", "Beta"
        ]
    );
}

#[test]
fn natural_order_compares_digit_runs_by_value_and_ignores_case() {
    for (a, b, expected) in [
        ("Phase 2", "Phase 10", Ordering::Less),
        ("phase 10", "Phase 2", Ordering::Greater),
        ("Phase 02", "Phase 2", Ordering::Less), // equal by value: the plain order decides
        ("v1.9", "v1.10", Ordering::Less),
        ("alpha", "BETA", Ordering::Less),
        ("a", "a1", Ordering::Less),
        ("same", "same", Ordering::Equal),
        ("12345678901234567890123", "2", Ordering::Greater),
    ] {
        assert_eq!(natural_cmp(a, b), expected, "{a} vs {b}");
        assert_eq!(natural_cmp(b, a), expected.reverse(), "{b} vs {a}");
    }
}

/* ── ship, reopen, delete ─────────────────────────────────────────────── */

fn shipping_env() -> Env {
    let mut env = Env::new();
    env.release("Phase 2", None);
    env.release("Phase 3", None);
    env.ticket_in("done one", "done", &[]);
    env.ticket_in("open one", "backlog", &[]);
    env.ticket_in("review one", "review", &[]);
    env.ticket_in("gone", "todo", &[]);
    for key in ["KAN-1", "KAN-2", "KAN-3", "KAN-4"] {
        env.set(key, None, Some("Phase 2"));
    }
    env.delete("KAN-4");
    env
}

#[test]
fn shipping_with_open_tickets_needs_to_be_told_where_they_go() {
    let mut env = shipping_env();
    let before = env.board.clone();
    let error = env.ship("Phase 2", None).unwrap_err();
    assert_eq!(
        error,
        "2 tickets are not done; choose where they go (move_open_to)"
    );
    assert_untouched(&before, &env.board);
}

#[test]
fn shipping_moves_the_open_tickets_to_another_release_with_activity() {
    let mut env = shipping_env();
    let (view, moved) = env.ship("phase 2", Some(Some("Phase 3"))).unwrap();
    let shipped_at = view.release.released_at.clone().unwrap();

    assert_eq!(view.status, ReleaseStatus::Released);
    assert_eq!((view.total, view.done, view.reopened), (1, 1, 0));
    assert_eq!(moved.len(), 2);
    let target = env
        .board
        .releases
        .values()
        .find(|r| r.name == "Phase 3")
        .unwrap()
        .id
        .clone();
    for key in ["KAN-2", "KAN-3"] {
        let ticket = env.get(key);
        assert_eq!(ticket.release_id.as_deref(), Some(target.as_str()), "{key}");
        assert_eq!(ticket.updated_at, shipped_at);
        let last = ticket.activity.last().unwrap();
        assert_eq!(last.kind, "ticket.updated");
        assert_eq!(
            last.changes.as_deref(),
            Some(&[change("release", json!("Phase 2"), json!("Phase 3"))][..])
        );
    }
    assert_eq!(
        env.get("KAN-1").release_id,
        Some(view.release.id.clone()),
        "done stays"
    );
    assert!(
        env.get("KAN-4").release_id.is_some(),
        "a deleted ticket keeps pointing at the release"
    );
    assert_eq!(env.release_view("Phase 3").total, 2);
    let events = views(&env.board, &context(), &moved);
    assert_eq!(events.len(), 2, "one event per moved ticket");
}

#[test]
fn shipping_with_null_takes_the_open_tickets_out_of_any_release() {
    let mut env = shipping_env();
    let (_, moved) = env.ship("Phase 2", Some(None)).unwrap();
    assert_eq!(moved.len(), 2);
    for key in ["KAN-2", "KAN-3"] {
        assert_eq!(env.get(key).release_id, None);
        assert_eq!(
            env.get(key).activity.last().unwrap().changes.as_deref(),
            Some(&[change("release", json!("Phase 2"), Value::Null)][..])
        );
    }
}

#[test]
fn shipping_refuses_a_bad_target_and_an_already_released_release() {
    let mut env = shipping_env();
    env.release("Phase 1", None);
    env.ship("Phase 1", None).unwrap();

    for (target, expected) in [
        (
            "Phase 2",
            "cannot move the open tickets into Phase 2 itself",
        ),
        ("Phase 1", "release Phase 1 is already released"),
        ("ghost", "release not found: ghost"),
    ] {
        let before = env.board.clone();
        let error = env.ship("Phase 2", Some(Some(target))).unwrap_err();
        assert!(error.contains(expected), "{target}: {error}");
        assert_untouched(&before, &env.board);
    }
    let error = env.ship("Phase 1", None).unwrap_err();
    assert!(error.contains("already released"), "{error}");
    assert!(
        env.ship("ghost", None)
            .unwrap_err()
            .contains("release not found")
    );
}

#[test]
fn an_empty_release_can_be_shipped_without_saying_where_anything_goes() {
    let mut env = Env::new();
    env.release("Empty", None);
    let (view, moved) = env.ship("Empty", None).unwrap();
    assert_eq!(view.status, ReleaseStatus::Released);
    assert!(moved.is_empty());

    env.release("Done", None);
    env.ticket_in("finished", "done", &[]);
    env.set("KAN-1", None, Some("Done"));
    assert!(
        env.ship("Done", None).is_ok(),
        "nothing open, nothing to place"
    );
}

#[test]
fn shipping_with_nothing_open_ignores_move_open_to() {
    let mut env = Env::new();
    env.release("Released", None);
    env.ship("Released", None).unwrap();
    env.release("Done", None);
    env.ticket_in("finished", "done", &[]);
    env.set("KAN-1", None, Some("Done"));
    for (release, target) in [("Done", "ghost"), ("Empty", "Released"), ("Alone", "Alone")] {
        if release != "Done" {
            env.release(release, None);
        }
        let (view, moved) = env.ship(release, Some(Some(target))).unwrap();
        assert_eq!(
            view.status,
            ReleaseStatus::Released,
            "{release} -> {target}"
        );
        assert!(moved.is_empty(), "{release} -> {target}");
    }

    // With something open the same targets are refused.
    env.release("Busy", None);
    env.ticket("open");
    env.set("KAN-2", None, Some("Busy"));
    assert!(
        env.ship("Busy", Some(Some("ghost")))
            .unwrap_err()
            .contains("release not found: ghost")
    );
}

#[test]
fn reopening_clears_the_date_and_the_moved_tickets_do_not_come_back() {
    let mut env = shipping_env();
    env.ship("Phase 2", Some(Some("Phase 3"))).unwrap();
    let at = env.at();
    let reopened = plan::reopen_release(&mut env.board, "Phase 2", &context(), &at).unwrap();
    assert_eq!(reopened.release.released_at, None);
    assert_eq!(reopened.release.updated_at, at);
    assert_eq!(
        reopened.status,
        ReleaseStatus::Active,
        "the done ticket is outside the first column"
    );
    assert_eq!(
        env.release_view("Phase 3").total,
        2,
        "moved tickets stay moved"
    );

    let later = env.at();
    let again = plan::reopen_release(&mut env.board, "Phase 2", &context(), &later).unwrap();
    assert_eq!(
        again.release.updated_at, at,
        "reopening a open release changes nothing"
    );
    env.ship("Phase 2", None).expect("it can be shipped again");
}

#[test]
fn a_release_is_deleted_only_when_no_ticket_references_it() {
    let mut env = Env::new();
    env.release("Used", None);
    env.release("Free", None);
    env.release("Trashed", None);
    env.ticket("live");
    env.ticket("dead");
    env.set("KAN-1", None, Some("Used"));
    env.set("KAN-2", None, Some("Trashed"));
    env.delete("KAN-2");

    for name in ["Used", "Trashed"] {
        let before = env.board.clone();
        let error = plan::delete_release(&mut env.board, name).unwrap_err();
        assert!(error.contains("still has 1 tickets"), "{name}: {error}");
        assert_untouched(&before, &env.board);
    }
    assert!(
        plan::delete_release(&mut env.board, "ghost")
            .unwrap_err()
            .contains("release not found")
    );

    plan::delete_release(&mut env.board, "free").unwrap();
    let names: Vec<&str> = env
        .board
        .releases
        .values()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, ["Used", "Trashed"], "the others keep their order");

    // Once the ticket leaves, so can the release.
    env.update(
        "KAN-1",
        UpdatePatch {
            release: Some(None),
            ..UpdatePatch::default()
        },
    )
    .unwrap();
    plan::delete_release(&mut env.board, "Used").unwrap();
}

/* ── label conversion ─────────────────────────────────────────────────── */

fn phase_board() -> Env {
    let mut env = Env::new();
    env.release("Phase 2", None);
    env.release("Phase 3", None);
    env.ticket_in("fresh", "backlog", &["phase-2"]); // KAN-1: added
    env.ticket_in("here already", "done", &["Phase-2", "bug"]); // KAN-2: already here
    env.ticket_in("elsewhere", "todo", &["phase-2"]); // KAN-3: in another release
    env.ticket_in("deleted", "backlog", &["phase-2"]); // KAN-4: deleted
    env.ticket_in("unlabelled", "backlog", &["phase-3"]); // KAN-5: not carrying it
    env.set("KAN-2", None, Some("Phase 2"));
    env.set("KAN-3", None, Some("Phase 3"));
    env.delete("KAN-4");
    env
}

#[test]
fn converting_a_label_sorts_the_tickets_into_buckets_and_keeps_the_others_where_they_are() {
    let mut env = phase_board();
    let (buckets, converted) = convert(
        &mut env,
        &convert_input("PHASE-2", ConvertInto::Release, Some("phase 2")),
    )
    .unwrap();

    assert_eq!(buckets.added, strs(&["KAN-1"]));
    assert_eq!(buckets.already_here, strs(&["KAN-2"]));
    assert_eq!(buckets.in_other, strs(&["KAN-3"]));
    assert_eq!(buckets.deleted, strs(&["KAN-4"]));
    assert!(!converted.created, "the name is an existing release");
    assert_eq!(converted.changed, [env.get("KAN-1").id]);
    assert_eq!(release_of(&converted.entity).release.name, "Phase 2");

    let phase_2 = env.release_view("Phase 2");
    assert_eq!(
        env.get("KAN-1").release_id.as_deref(),
        Some(phase_2.release.id.as_str())
    );
    assert_eq!(
        env.get("KAN-3").release_id,
        env.release_view("Phase 3").release.id.into(),
        "kept"
    );
    assert_eq!(
        env.get("KAN-4").release_id,
        None,
        "deleted tickets are skipped"
    );
    assert_eq!(env.get("KAN-5").release_id, None);
    assert_eq!(
        env.get("KAN-1").labels,
        ["phase-2"],
        "the label stays by default"
    );
    assert_eq!(
        phase_2.release.source_label, None,
        "an existing release keeps its fields"
    );

    let last = env.get("KAN-1").activity.last().unwrap().clone();
    assert_eq!(last.kind, "ticket.updated");
    assert_eq!(
        last.changes.as_deref(),
        Some(&[change("release", Value::Null, json!("Phase 2"))][..])
    );
    assert_eq!(env.get("KAN-1").updated_at, last.at);
}

#[test]
fn replace_moves_the_tickets_that_sit_in_another_release() {
    let mut env = phase_board();
    let mut input = convert_input("phase-2", ConvertInto::Release, Some("Phase 2"));
    input.replace = true;
    let (buckets, converted) = convert(&mut env, &input).unwrap();

    assert_eq!(
        buckets.in_other,
        strs(&["KAN-3"]),
        "the bucket is where it was"
    );
    assert_eq!(converted.changed.len(), 2);
    let phase_2 = env.release_view("Phase 2").release.id;
    assert_eq!(
        env.get("KAN-3").release_id.as_deref(),
        Some(phase_2.as_str())
    );
    assert_eq!(
        env.get("KAN-3").activity.last().unwrap().changes.as_deref(),
        Some(&[change("release", json!("Phase 3"), json!("Phase 2"))][..])
    );
}

#[test]
fn remove_label_takes_the_label_off_the_tickets_that_end_up_in_the_entity() {
    let mut env = phase_board();
    let mut input = convert_input("phase-2", ConvertInto::Release, Some("Phase 2"));
    input.remove_label = true;
    let (_, converted) = convert(&mut env, &input).unwrap();

    assert_eq!(env.get("KAN-1").labels, Vec::<String>::new(), "added");
    assert_eq!(
        env.get("KAN-2").labels,
        ["bug"],
        "already here, other labels stay"
    );
    assert_eq!(
        env.get("KAN-3").labels,
        ["phase-2"],
        "kept in the other release, kept labelled"
    );
    assert_eq!(
        env.get("KAN-4").labels,
        ["phase-2"],
        "deleted tickets are skipped"
    );
    assert_eq!(converted.changed.len(), 2, "KAN-1 and KAN-2");
    let last = env.get("KAN-2").activity.last().unwrap().clone();
    assert_eq!(
        last.changes.as_deref(),
        Some(&[change("labels", json!(["Phase-2", "bug"]), json!(["bug"]))][..]),
        "a ticket that only loses the label is still a write"
    );
    assert_eq!(env.get("KAN-2").updated_at, last.at);

    // With replace the moved ticket loses it too.
    let mut env = phase_board();
    let mut input = convert_input("phase-2", ConvertInto::Release, Some("Phase 2"));
    input.remove_label = true;
    input.replace = true;
    convert(&mut env, &input).unwrap();
    assert!(env.get("KAN-3").labels.is_empty());
    assert_eq!(
        env.get("KAN-3")
            .activity
            .last()
            .unwrap()
            .changes
            .as_deref()
            .map(<[Change]>::len),
        Some(2),
        "release and labels in one entry"
    );
}

#[test]
fn running_a_conversion_twice_adds_nothing() {
    let mut env = phase_board();
    let mut input = convert_input("phase-2", ConvertInto::Release, Some("Phase 2"));
    input.replace = true;
    input.remove_label = true;
    convert(&mut env, &input).unwrap();
    let snapshot = env.board.clone();

    let (buckets, converted) = convert(&mut env, &input).unwrap();
    assert!(buckets.added.is_empty() && buckets.in_other.is_empty());
    assert_eq!(buckets.deleted, strs(&["KAN-4"]));
    assert!(converted.changed.is_empty() && !converted.created);
    assert_untouched(&snapshot, &env.board);
}

#[test]
fn a_new_name_creates_the_entity_and_records_the_label_it_came_from() {
    let mut env = phase_board();
    let mut input = convert_input("phase-3", ConvertInto::Release, Some("  Phase 3 beta "));
    input.description = Some(" the beta ".into());
    input.target_at = Some("2026-12-01".into());
    let (buckets, converted) = convert(&mut env, &input).unwrap();

    assert!(converted.created);
    let release = &release_of(&converted.entity).release;
    assert_eq!(release.name, "Phase 3 beta");
    assert_eq!(release.description, "the beta");
    assert_eq!(release.target_at.as_deref(), Some("2026-12-01"));
    assert_eq!(release.source_label.as_deref(), Some("phase-3"));
    assert_eq!(buckets.added, strs(&["KAN-5"]));
    assert_eq!(
        env.get("KAN-5").release_id.as_deref(),
        Some(release.id.as_str())
    );
    assert_eq!(env.board.releases.len(), 3);

    // The same name again reuses it (case ignored) instead of creating a twin.
    let (_, again) = convert(
        &mut env,
        &convert_input("phase-3", ConvertInto::Release, Some("PHASE 3 BETA")),
    )
    .unwrap();
    assert!(!again.created);
    assert_eq!(env.board.releases.len(), 3);
}

#[test]
fn an_id_adds_to_that_entity_and_a_missing_target_is_an_error() {
    let mut env = phase_board();
    let phase_3 = env.release_view("Phase 3").release.id;
    let mut input = convert_input("phase-2", ConvertInto::Release, None);
    input.id = Some(phase_3.clone());
    let (buckets, converted) = convert(&mut env, &input).unwrap();
    assert_eq!(
        buckets.already_here,
        strs(&["KAN-3"]),
        "KAN-3 is in Phase 3"
    );
    assert_eq!(buckets.in_other, strs(&["KAN-2"]));
    assert_eq!(release_of(&converted.entity).release.id, phase_3);

    for (input, expected) in [
        (
            convert_input("phase-2", ConvertInto::Release, None),
            "pass id",
        ),
        (
            convert_input("  ", ConvertInto::Release, Some("x")),
            "label is required",
        ),
        (
            ConvertInput {
                id: Some("ghost".into()),
                ..convert_input("phase-2", ConvertInto::Release, None)
            },
            "release not found: ghost",
        ),
        (
            ConvertInput {
                id: Some("ghost".into()),
                ..convert_input("phase-2", ConvertInto::Project, None)
            },
            "project not found: ghost",
        ),
        (
            ConvertInput {
                target_at: Some("soon".into()),
                ..convert_input("phase-2", ConvertInto::Release, Some("New"))
            },
            "YYYY-MM-DD",
        ),
    ] {
        let before = env.board.clone();
        let error = convert(&mut env, &input).unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert_untouched(&before, &env.board);
    }
}

#[test]
fn a_label_becomes_a_project_with_the_same_buckets_and_refuses_an_archived_one() {
    let mut env = Env::new();
    env.project("iii cloud");
    env.project("package");
    env.ticket_in("free", "backlog", &["iii-cloud"]);
    env.ticket_in("mine", "done", &["iii-cloud"]);
    env.ticket_in("theirs", "todo", &["III-CLOUD"]);
    env.set("KAN-2", Some("iii cloud"), None);
    env.set("KAN-3", Some("package"), None);

    let (buckets, converted) = convert(
        &mut env,
        &convert_input("iii-cloud", ConvertInto::Project, Some("iii cloud")),
    )
    .unwrap();
    assert_eq!(buckets.added, strs(&["KAN-1"]));
    assert_eq!(buckets.already_here, strs(&["KAN-2"]));
    assert_eq!(buckets.in_other, strs(&["KAN-3"]));
    let view = project_of(&converted.entity);
    assert_eq!((view.total, view.done), (2, 1));
    assert_eq!(
        env.get("KAN-1").activity.last().unwrap().changes.as_deref(),
        Some(&[change("project", Value::Null, json!("iii cloud"))][..])
    );

    let at = env.at();
    plan::set_project_archived(&mut env.board, "iii cloud", true, &context(), &at).unwrap();
    let error = convert(
        &mut env,
        &convert_input("iii-cloud", ConvertInto::Project, Some("iii cloud")),
    )
    .unwrap_err();
    assert!(error.contains("is archived"), "{error}");

    let (_, created) = convert(
        &mut env,
        &convert_input("iii-cloud", ConvertInto::Project, Some("Brand new")),
    )
    .unwrap();
    assert!(created.created);
    assert_eq!(
        project_of(&created.entity).project.source_label.as_deref(),
        Some("iii-cloud")
    );
}

#[test]
fn mark_released_if_done_dates_the_release_at_the_last_ticket_not_at_the_conversion() {
    let mut env = Env::new();
    env.ticket_in("early", "done", &["phase-0"]);
    env.ticket_in("late", "done", &["phase-0"]);
    env.ticket_in("other", "backlog", &["phase-1"]);
    let late = env.get("KAN-2").updated_at;

    let mut input = convert_input("phase-0", ConvertInto::Release, Some("Phase 0"));
    input.mark_released_if_done = true;
    let (_, converted) = convert(&mut env, &input).unwrap();
    let release = release_of(&converted.entity);
    assert_eq!(release.release.released_at.as_deref(), Some(late.as_str()));
    assert_eq!(release.status, ReleaseStatus::Released);
    assert_eq!(converted.changed.len(), 2);
    assert_ne!(
        env.get("KAN-2").updated_at,
        late,
        "the tickets were bumped, the date was not"
    );

    // Not every ticket is done: it stays open, and without the flag too.
    for (label, name, flag) in [
        ("phase-1", "Phase 1", true),
        ("phase-0", "Phase 0 again", false),
    ] {
        let mut input = convert_input(label, ConvertInto::Release, Some(name));
        input.mark_released_if_done = flag;
        input.replace = true;
        let (_, converted) = convert(&mut env, &input).unwrap();
        assert_eq!(
            release_of(&converted.entity).release.released_at,
            None,
            "{name}"
        );
    }

    // A conversion into a project ignores the flag, and an empty release is not "all done".
    let mut empty = convert_input("nothing", ConvertInto::Release, Some("Empty"));
    empty.mark_released_if_done = true;
    let (_, converted) = convert(&mut env, &empty).unwrap();
    assert_eq!(release_of(&converted.entity).release.released_at, None);
}

#[test]
fn a_dry_run_plans_on_a_shared_borrow_and_the_board_is_not_touched() {
    let env = phase_board();
    let before = env.board.clone();
    let mut input = convert_input("phase-2", ConvertInto::Release, Some("Phase 2"));
    input.replace = true;
    input.remove_label = true;
    let planned = plan::plan_convert(&env.board, &input).unwrap();
    assert_eq!(planned.buckets(&env.board).added, strs(&["KAN-1"]));
    assert_eq!(
        planned.existing(),
        Some(env.release_view("Phase 2").release.id.as_str()),
        "an existing target is reported"
    );
    let new = plan::plan_convert(
        &env.board,
        &convert_input("phase-2", ConvertInto::Release, Some("Brand new")),
    )
    .unwrap();
    assert_eq!(
        new.existing(),
        None,
        "a target to create is not an entity yet"
    );
    assert_eq!(new.buckets(&env.board).added, strs(&["KAN-1"]));
    assert_eq!(new.buckets(&env.board).in_other, strs(&["KAN-2", "KAN-3"]));
    assert_untouched(&before, &env.board);
}

/* ── events, the views and the trigger filters ────────────────────────── */

fn event_of(board: &Board, key: &str) -> ChangeEvent {
    let ticket = find_ticket(board, key).unwrap();
    ChangeEvent {
        event: "ticket.updated".into(),
        ticket_id: ticket.id.clone(),
        ticket_key: ticket.key.clone(),
        ticket: ticket_view(board, ticket, "done", None).into(),
        comment: None,
        activity: ticket.activity.last().cloned(),
        at: ticket.updated_at.clone(),
        releases: event_releases(board, ticket, &ticket.updated_at),
    }
}

fn comment_event_of(env: &mut Env, key: &str, author: &str) -> CommentEvent {
    let at = env.at();
    let result = create_comment(
        &mut env.board,
        &CreateCommentInput {
            ticket_id: key.into(),
            body: "hello".into(),
            parent_id: None,
            author: Some(author.into()),
        },
        &at,
    )
    .unwrap()
    .unwrap();
    CommentEvent {
        event: "comment.created".into(),
        ticket_id: result.ticket.id.clone(),
        ticket_key: result.ticket.key.clone(),
        ticket: ticket_view(&env.board, &result.ticket, "done", None).into(),
        comment: result.comment,
        author: author.into(),
        at,
    }
}

#[test]
fn a_write_that_touches_two_tickets_announces_both_and_a_no_op_announces_none() {
    let mut env = Env::new();
    env.ticket("Parent");
    env.ticket("Child");
    let before = env.get("KAN-2");
    let result = env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    let outcome = ticket_outcome(
        &env.board,
        &context(),
        Some(before),
        Some(result.ticket.clone()),
        &result.related,
    );
    let announced: Vec<&str> = outcome
        .tickets
        .iter()
        .map(|v| v.ticket.key.as_str())
        .collect();
    assert_eq!(
        announced,
        ["KAN-2", "KAN-1"],
        "the ticket first, then its parent"
    );
    assert_eq!(
        outcome
            .result
            .as_ref()
            .unwrap()
            .parent
            .as_ref()
            .unwrap()
            .key,
        "KAN-1"
    );
    let parent = &outcome.tickets[1];
    assert_eq!(
        parent.children[0].key, "KAN-2",
        "the parent event carries the new subtask"
    );
    assert_eq!(parent.ticket.activity.last().unwrap().kind, "subtask.added");

    let same = env.get("KAN-2");
    let none = ticket_outcome(&env.board, &context(), Some(same.clone()), Some(same), &[]);
    assert!(none.tickets.is_empty(), "nothing changed, nothing fires");
    assert!(none.result.is_some());
}

#[test]
fn a_reorder_inside_a_column_announces_no_release_the_ticket_left_earlier() {
    let mut env = Env::new();
    env.release("Phase 2", None);
    env.release("Phase 3", None);
    env.ticket("First");
    env.ticket("Second");
    env.set("KAN-1", None, Some("Phase 2"));
    env.set("KAN-1", None, Some("Phase 3"));
    let leaving = env.get("KAN-1").activity.last().unwrap().at.clone();
    let names = |board: &Board, at: &str| -> Vec<String> {
        event_releases(board, find_ticket(board, "KAN-1").unwrap(), at)
            .into_iter()
            .map(|release| release.name)
            .collect()
    };

    assert_eq!(
        names(&env.board, &leaving),
        ["Phase 3", "Phase 2"],
        "the write that moved it names both"
    );

    // A reorder records no activity: the newest entry is the move out of Phase 2.
    let at = env.at();
    move_ticket(
        &mut env.board,
        "KAN-1",
        "backlog",
        Some(1),
        None,
        &context(),
        &at,
    )
    .unwrap()
    .unwrap();
    let ticket = env.get("KAN-1");
    assert_eq!(ticket.updated_at, at, "the reorder is a write");
    assert_eq!(
        ticket.activity.last().unwrap().at,
        leaving,
        "and records nothing"
    );
    assert_eq!(names(&env.board, &at), ["Phase 3"]);
    let event = event_of(&env.board, "KAN-1");
    assert!(matches_change(&json!({ "release_id": "Phase 3" }), &event));
    assert!(
        !matches_change(&json!({ "release_id": "Phase 2" }), &event),
        "Phase 2 was left before this write"
    );
}

#[test]
fn a_write_names_its_events_after_the_call_for_the_first_ticket_and_ticket_updated_after() {
    let mut env = Env::new();
    for title in ["Q", "P", "C"] {
        env.ticket(title);
    }
    env.update("KAN-3", parent_patch(Some("KAN-2"))).unwrap();
    env.delete("KAN-3");
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    let before = env.get("KAN-3");
    let at = env.at();
    let restored = restore_ticket(&mut env.board, "KAN-3", "user", &at).unwrap();
    let outcome = ticket_outcome(
        &env.board,
        &context(),
        Some(before),
        Some(restored.ticket),
        &restored.related,
    );
    let comment = comment_event_of(&mut env, "KAN-3", "user").comment;

    let events = change_events(
        &env.board,
        "ticket.restored",
        &outcome.tickets,
        Some(&comment),
        &at,
    );
    let summary: Vec<(&str, &str, &str)> = events
        .iter()
        .map(|event| {
            (
                event.event.as_str(),
                event.ticket_key.as_str(),
                event.activity.as_ref().unwrap().kind.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("ticket.restored", "KAN-3", "ticket.restored"),
            ("ticket.updated", "KAN-2", "subtask.removed"),
        ]
    );
    assert!(events.iter().all(|event| event.at == at));
    assert_eq!(
        events[0].comment.as_ref(),
        Some(&comment),
        "the call's own event carries it"
    );
    assert_eq!(events[1].comment, None);

    // Each event names the releases its own ticket left in this write.
    let mut env = shipping_env();
    let (_, moved) = env.ship("Phase 2", Some(Some("Phase 3"))).unwrap();
    assert_eq!(moved.len(), 2);
    let at = clock(env.tick);
    let shipped = views(&env.board, &context(), &moved);
    let events = change_events(&env.board, "ticket.updated", &shipped, None, &at);
    assert_eq!(events.len(), 2);
    for event in &events {
        let releases: Vec<&str> = event.releases.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(releases, ["Phase 3", "Phase 2"], "{}", event.ticket_key);
    }
}

#[test]
fn every_ticket_serializes_as_a_view_with_the_contract_field_names() {
    let mut env = Env::new();
    env.project("cloud");
    env.release("R", None);
    env.ticket("Parent");
    env.ticket("Child");
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    env.set("KAN-2", Some("cloud"), Some("R"));

    let keys_of = |value: &Value| -> BTreeSet<String> {
        value.as_object().unwrap().keys().cloned().collect()
    };
    let set =
        |names: &[&str]| -> BTreeSet<String> { names.iter().map(|n| n.to_string()).collect() };

    let parent = serde_json::to_value(env.view("KAN-1")).unwrap();
    assert_eq!(
        keys_of(&parent),
        set(&[
            "id",
            "key",
            "title",
            "description",
            "status",
            "priority",
            "assignee",
            "labels",
            "parent_id",
            "project_id",
            "release_id",
            "order",
            "created_at",
            "updated_at",
            "deleted_at",
            "created_by",
            "comments",
            "activity",
            "parent",
            "children",
            "progress",
        ])
    );
    assert_eq!(parent["progress"], json!({ "done": 0, "total": 1 }));
    assert_eq!(parent["parent"], Value::Null);
    let brief = &parent["children"][0];
    assert_eq!(
        keys_of(brief),
        set(&[
            "id",
            "key",
            "title",
            "status",
            "priority",
            "assignee",
            "deleted_at",
            "project_id",
            "release_id",
        ])
    );
    assert_eq!(brief["key"], "KAN-2");

    let project = serde_json::to_value(env.project_view("cloud")).unwrap();
    assert_eq!(
        keys_of(&project),
        set(&[
            "id",
            "name",
            "description",
            "lead",
            "color",
            "source_label",
            "created_at",
            "updated_at",
            "archived_at",
            "status",
            "total",
            "done",
            "counts",
        ])
    );
    assert_eq!(project["status"], "active");
    let release = serde_json::to_value(env.release_view("R")).unwrap();
    assert_eq!(
        keys_of(&release),
        set(&[
            "id",
            "name",
            "description",
            "target_at",
            "released_at",
            "source_label",
            "created_at",
            "updated_at",
            "status",
            "total",
            "done",
            "counts",
            "reopened",
            "projects",
        ])
    );
    assert_eq!(
        release["status"], "planned",
        "its only ticket is still in the first column"
    );
    assert_eq!(release["projects"][0]["total"], 1);
    assert_eq!(
        keys_of(&release["projects"][0]),
        set(&["project_id", "total", "done"])
    );
}

#[test]
fn the_board_view_carries_the_done_column_the_projects_and_the_releases() {
    let mut env = Env::new();
    env.project("cloud");
    env.project("old");
    env.release("Phase 10", None);
    env.release("Phase 2", None);
    let at = env.at();
    plan::set_project_archived(&mut env.board, "old", true, &context(), &at).unwrap();
    env.ticket("One");

    let config = KanbanConfig {
        columns: context().columns,
        default_status: "backlog".into(),
        done_status: "review".into(),
        canceled_status: None,
        ..KanbanConfig::default()
    };
    let view = board_view(&env.board, &config, &BoardGetInput::default());
    assert_eq!(view.done_status, "review");
    let projects: Vec<(&str, ProjectStatus)> = view
        .projects
        .iter()
        .map(|p| (p.project.name.as_str(), p.status))
        .collect();
    assert_eq!(
        projects,
        [
            ("cloud", ProjectStatus::Active),
            ("old", ProjectStatus::Archived)
        ],
        "archived projects are included"
    );
    let releases: Vec<&str> = view
        .releases
        .iter()
        .map(|r| r.release.name.as_str())
        .collect();
    assert_eq!(releases, ["Phase 2", "Phase 10"]);

    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["done_status"], "review");
    assert_eq!(json["projects"][1]["status"], "archived");
    assert_eq!(json["columns"][0]["tickets"][0]["parent_id"], Value::Null);

    let schema = serde_json::to_value(schemars::schema_for!(BoardView)).unwrap();
    for field in ["done_status", "projects", "releases"] {
        assert!(schema["properties"][field].is_object(), "{field}");
    }
}

#[test]
fn tickets_are_filtered_by_parent_project_and_release_by_key_uuid_or_name() {
    let mut env = Env::new();
    let cloud = env.project("iii cloud");
    env.project("package");
    let phase = env.release("Phase 2", None);
    for title in ["Parent", "Child A", "Child B", "Loose"] {
        env.ticket(title);
    }
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    env.update("KAN-3", parent_patch(Some("KAN-1"))).unwrap();
    env.set("KAN-1", Some("iii cloud"), Some("Phase 2"));
    env.set("KAN-2", Some("package"), None);
    env.delete("KAN-3");

    let list =
        |parent: Option<&str>, project: Option<&str>, release: Option<&str>, deleted: bool| {
            keys(list_tickets(
                &env.board,
                &TicketFilter {
                    parent_id: parent.map(String::from),
                    project_id: project.map(String::from),
                    release_id: release.map(String::from),
                    include_deleted: deleted,
                    ..TicketFilter::default()
                },
            ))
        };
    let parent_id = env.get("KAN-1").id;

    for reference in ["KAN-1", "kan-1", parent_id.as_str()] {
        assert_eq!(
            list(Some(reference), None, None, false),
            ["KAN-2"],
            "{reference}"
        );
    }
    assert_eq!(list(Some("KAN-1"), None, None, true), ["KAN-2", "KAN-3"]);
    assert_eq!(
        list(Some(""), None, None, false),
        ["KAN-1", "KAN-4"],
        "empty = not a subtask"
    );
    for reference in ["iii cloud", "III CLOUD", cloud.project.id.as_str()] {
        assert_eq!(
            list(None, Some(reference), None, false),
            ["KAN-1"],
            "{reference}"
        );
    }
    assert_eq!(
        list(None, Some(""), None, false),
        ["KAN-4"],
        "empty = no project"
    );
    for reference in ["Phase 2", "phase 2", phase.release.id.as_str()] {
        assert_eq!(
            list(None, None, Some(reference), false),
            ["KAN-1"],
            "{reference}"
        );
    }
    assert_eq!(
        list(None, None, Some(""), false),
        ["KAN-2", "KAN-4"],
        "empty = no release"
    );
    assert_eq!(
        list(Some("KAN-1"), Some("package"), Some(""), false),
        ["KAN-2"],
        "filters combine"
    );

    // An unknown reference matches nothing instead of falling back to everything.
    assert!(list(Some("KAN-404"), None, None, true).is_empty());
    assert!(list(None, Some("ghost"), None, true).is_empty());
    assert!(list(None, None, Some("ghost"), true).is_empty());

    // The board read applies the same filters per column and keeps the counts.
    let config = KanbanConfig {
        columns: context().columns,
        default_status: "backlog".into(),
        ..KanbanConfig::default()
    };
    let view = board_view(
        &env.board,
        &config,
        &BoardGetInput {
            parent_id: Some("KAN-1".into()),
            ..BoardGetInput::default()
        },
    );
    assert_eq!(view.columns[0].tickets.len(), 1);
    assert_eq!(view.ticket_count, 3, "ticket_count stays board-wide");
    let row = serde_json::to_value(&view.columns[0].tickets[0]).unwrap();
    assert_eq!(row["parent_id"], json!(parent_id));
    assert_eq!(row["project_id"], json!(env.get("KAN-2").project_id));
    assert_eq!(row["release_id"], Value::Null);
}

#[test]
fn the_change_trigger_parent_id_fires_for_the_parent_and_each_subtask() {
    let mut env = Env::new();
    for title in ["Parent", "Child", "Stranger"] {
        env.ticket(title);
    }
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    let parent_id = env.get("KAN-1").id;

    for reference in ["KAN-1", "kan-1", parent_id.as_str()] {
        let config = json!({ "parent_id": reference });
        assert!(
            matches_change(&config, &event_of(&env.board, "KAN-1")),
            "{reference}: the parent"
        );
        assert!(
            matches_change(&config, &event_of(&env.board, "KAN-2")),
            "{reference}: its subtask"
        );
        assert!(
            !matches_change(&config, &event_of(&env.board, "KAN-3")),
            "{reference}: a stranger"
        );
    }
    assert!(!matches_change(
        &json!({ "parent_id": "KAN-2" }),
        &event_of(&env.board, "KAN-1")
    ));
    assert!(
        !matches_change(
            &json!({ "parent_id": "KAN-404" }),
            &event_of(&env.board, "KAN-2")
        ),
        "unknown matches nothing"
    );
    assert!(
        matches_change(&json!({ "parent_id": "" }), &event_of(&env.board, "KAN-3")),
        "blank is no filter"
    );

    // It combines with the other filters, and a summary payload still knows its parent's uuid.
    let child = event_of(&env.board, "KAN-2");
    assert!(!matches_change(
        &json!({ "parent_id": "KAN-1", "events": ["ticket.moved"] }),
        &child
    ));
    assert!(!matches_change(
        &json!({ "parent_id": "KAN-1", "ticket_id": "KAN-1" }),
        &child
    ));
    let mut summary = child.clone();
    summary.ticket = child.ticket.summarized();
    assert!(matches_change(&json!({ "parent_id": parent_id }), &summary));
    assert!(
        !matches_change(&json!({ "parent_id": "KAN-1" }), &summary),
        "a summary carries no parent key"
    );
}

#[test]
fn the_change_trigger_release_id_fires_for_tickets_in_the_release_and_those_moved_out() {
    let mut env = Env::new();
    env.release("Phase 2", None);
    env.release("Phase 3", None);
    for title in ["In", "Moving", "Outside"] {
        env.ticket(title);
    }
    env.set("KAN-1", None, Some("Phase 2"));
    env.set("KAN-2", None, Some("Phase 2"));
    let phase_2 = env.release_view("Phase 2").release.id;
    let phase_3 = env.release_view("Phase 3").release.id;

    // A status change of a ticket that is in the release.
    env.status("KAN-1", "todo");
    for reference in [phase_2.as_str(), "Phase 2", "phase 2"] {
        let config = json!({ "release_id": reference });
        assert!(
            matches_change(&config, &event_of(&env.board, "KAN-1")),
            "{reference}: in the release"
        );
        assert!(
            !matches_change(&config, &event_of(&env.board, "KAN-3")),
            "{reference}: outside"
        );
    }

    // KAN-2 moves from Phase 2 to Phase 3: both releases hear about it.
    env.set("KAN-2", None, Some("Phase 3"));
    let moved = event_of(&env.board, "KAN-2");
    assert_eq!(moved.releases.len(), 2);
    for (reference, expected) in [
        (phase_2.as_str(), true),
        ("Phase 2", true),
        (phase_3.as_str(), true),
        ("Phase 3", true),
        ("ghost", false),
    ] {
        assert_eq!(
            matches_change(&json!({ "release_id": reference }), &moved),
            expected,
            "{reference}"
        );
    }

    // Taken out of every release: only the one it left is told.
    env.update(
        "KAN-2",
        UpdatePatch {
            release: Some(None),
            ..UpdatePatch::default()
        },
    )
    .unwrap();
    let cleared = event_of(&env.board, "KAN-2");
    assert!(matches_change(
        &json!({ "release_id": "Phase 3" }),
        &cleared
    ));
    assert!(
        !matches_change(&json!({ "release_id": "Phase 2" }), &cleared),
        "the move before it is history"
    );

    // Later writes to the ticket no longer match the release it left.
    env.status("KAN-2", "todo");
    assert!(!matches_change(
        &json!({ "release_id": "Phase 3" }),
        &event_of(&env.board, "KAN-2")
    ));

    // A ship names each moved ticket's release change the same way.
    let (_, shipped) = env.ship("Phase 2", Some(Some("Phase 3"))).unwrap();
    assert_eq!(shipped, [env.get("KAN-1").id]);
    let after_ship = event_of(&env.board, "KAN-1");
    assert!(matches_change(
        &json!({ "release_id": "Phase 2" }),
        &after_ship
    ));
    assert!(matches_change(
        &json!({ "release_id": "Phase 3" }),
        &after_ship
    ));
    assert!(matches_change(
        &json!({ "release_id": phase_3, "events": ["ticket.updated"] }),
        &after_ship
    ));
}

#[test]
fn the_comment_trigger_takes_a_ticket_or_a_parent() {
    let mut env = Env::new();
    for title in ["Parent", "Child", "Stranger"] {
        env.ticket(title);
    }
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    let parent_id = env.get("KAN-1").id;
    let on_parent = comment_event_of(&mut env, "KAN-1", "tech-lead");
    let on_child = comment_event_of(&mut env, "KAN-2", "tech-lead");
    let on_stranger = comment_event_of(&mut env, "KAN-3", "tech-lead");

    for reference in ["KAN-1", "kan-1", parent_id.as_str()] {
        let config = json!({ "parent_id": reference });
        assert!(matches_comment(&config, &on_parent), "{reference}");
        assert!(matches_comment(&config, &on_child), "{reference}");
        assert!(!matches_comment(&config, &on_stranger), "{reference}");
    }
    assert!(!matches_comment(
        &json!({ "parent_id": "KAN-404" }),
        &on_child
    ));

    // ticket_id still means exactly one ticket.
    assert!(matches_comment(&json!({ "ticket_id": "KAN-2" }), &on_child));
    assert!(!matches_comment(
        &json!({ "ticket_id": "KAN-1" }),
        &on_child
    ));

    // Neither: never fires. Both: both must hold. The other filters still apply.
    assert!(!matches_comment(&json!({}), &on_child));
    assert!(!matches_comment(
        &json!({ "parent_id": "  ", "ticket_id": "" }),
        &on_child
    ));
    assert!(matches_comment(
        &json!({ "ticket_id": "KAN-2", "parent_id": "KAN-1" }),
        &on_child
    ));
    assert!(!matches_comment(
        &json!({ "ticket_id": "KAN-3", "parent_id": "KAN-1" }),
        &on_child
    ));
    assert!(!matches_comment(
        &json!({ "parent_id": "KAN-1", "exclude_author": "tech-lead" }),
        &on_child
    ));
    assert!(matches_comment(
        &json!({ "parent_id": "KAN-1", "author": "tech-lead" }),
        &on_child
    ));
}

/* ── the function catalog ─────────────────────────────────────────────── */

#[test]
fn the_catalog_registers_the_planning_functions_with_typed_inputs() {
    let ids = functions::FUNCTION_IDS;
    for id in [
        "kanban::project::list",
        "kanban::project::create",
        "kanban::project::update",
        "kanban::project::archive",
        "kanban::project::restore",
        "kanban::release::list",
        "kanban::release::create",
        "kanban::release::update",
        "kanban::release::ship",
        "kanban::release::reopen",
        "kanban::release::delete",
        "kanban::label::convert",
    ] {
        assert!(ids.contains(&id), "{id}");
    }

    let schema = |s: schemars::schema::RootSchema| serde_json::to_value(s).unwrap();
    let create = schema(schemars::schema_for!(TicketCreateInput));
    let update = schema(schemars::schema_for!(TicketUpdateInput));
    for field in ["parent", "project", "release"] {
        assert!(create["properties"][field].is_object(), "create.{field}");
        assert_eq!(
            update["properties"][field]["type"],
            json!(["string", "null"]),
            "update.{field}"
        );
    }
    let convert = schema(schemars::schema_for!(LabelConvertInput));
    assert_eq!(convert["required"], json!(["into", "label"]));
    for field in [
        "id",
        "name",
        "description",
        "target_at",
        "replace",
        "remove_label",
        "mark_released_if_done",
        "dry_run",
    ] {
        assert!(convert["properties"][field].is_object(), "convert.{field}");
    }

    // Absent, null and a value are three different ship requests.
    let ship = |value: Value| {
        serde_json::from_value::<ReleaseShipInput>(value)
            .unwrap()
            .move_open_to
    };
    assert_eq!(ship(json!({ "id": "R" })), None);
    assert_eq!(ship(json!({ "id": "R", "move_open_to": null })), Some(None));
    assert_eq!(
        ship(json!({ "id": "R", "move_open_to": "Phase 3" })),
        Some(Some("Phase 3".into()))
    );
    let patch: TicketUpdateInput =
        serde_json::from_value(json!({ "id": "KAN-1", "parent": null })).unwrap();
    assert_eq!(patch.parent, Some(None));
    let patch: TicketUpdateInput = serde_json::from_value(json!({ "id": "KAN-1" })).unwrap();
    assert_eq!(
        (patch.parent, patch.project, patch.release),
        (None, None, None)
    );
    assert!(
        serde_json::from_value::<LabelConvertInput>(json!({ "label": "x", "into": "sprint" }))
            .is_err()
    );
}

#[test]
fn deliveries_carry_the_view_and_the_trigger_configs_declare_the_new_filters() {
    let mut env = Env::new();
    env.release("R", None);
    env.ticket("Parent");
    env.ticket("Child");
    env.update("KAN-2", parent_patch(Some("KAN-1"))).unwrap();
    env.set("KAN-2", None, Some("R"));
    let event = event_of(&env.board, "KAN-2");

    let (full, summary) = payloads(&event, &event.ticket).unwrap();
    assert!(
        full.get("releases").is_none(),
        "the matching aid is not delivered"
    );
    assert_eq!(full["ticket"]["parent"]["key"], "KAN-1");
    assert_eq!(full["ticket"]["progress"], json!({ "done": 0, "total": 0 }));
    let parent_event = event_of(&env.board, "KAN-1");
    let (parent_full, _) = payloads(&parent_event, &parent_event.ticket).unwrap();
    assert_eq!(parent_full["ticket"]["children"][0]["key"], "KAN-2");
    assert_eq!(
        parent_full["ticket"]["progress"],
        json!({ "done": 0, "total": 1 })
    );
    assert!(
        summary["ticket"].get("children").is_none(),
        "summary stays the board card"
    );
    assert_eq!(summary["ticket"]["parent_id"], json!(env.get("KAN-1").id));
    assert_eq!(
        summary["ticket"]["release_id"],
        json!(env.get("KAN-2").release_id)
    );

    let change = serde_json::to_value(schemars::schema_for!(ChangeConfig)).unwrap();
    for field in ["ticket_id", "parent_id", "release_id", "events"] {
        assert!(change["properties"][field].is_object(), "change.{field}");
    }
    let comment = serde_json::to_value(schemars::schema_for!(CommentConfig)).unwrap();
    for field in [
        "ticket_id",
        "parent_id",
        "author",
        "exclude_author",
        "root_only",
    ] {
        assert!(comment["properties"][field].is_object(), "comment.{field}");
    }
    assert!(
        comment.get("required").is_none(),
        "either ticket_id or parent_id: neither is required"
    );
    let parsed: CommentConfig = serde_json::from_value(json!({ "parent_id": "KAN-1" })).unwrap();
    assert_eq!(
        (parsed.ticket_id, parsed.parent_id.as_deref()),
        (None, Some("KAN-1"))
    );

    // Every output the new functions return has a schema.
    schemars::schema_for!(ChangeEvent);
    schemars::schema_for!(CommentEvent);
    schemars::schema_for!(TicketView);
    schemars::schema_for!(functions::ConvertResult);
    schemars::schema_for!(functions::ShipResult);
    schemars::schema_for!(functions::ProjectList);
    schemars::schema_for!(functions::ReleaseList);
    schemars::schema_for!(functions::CommentCreated);
}

/* ── canceled ─────────────────────────────────────────────────────────── */

fn canceled_context() -> BoardContext {
    let mut context = context();
    context.columns.push(Column {
        id: "canceled".into(),
        label: "Canceled".into(),
    });
    context.canceled_status = Some("canceled".into());
    context
}

#[test]
fn canceled_tickets_leave_progress_project_and_release_totals_and_never_hold_a_release_open() {
    let context = canceled_context();
    let mut board = Board::empty();
    let at = clock(1);
    let named = |name: &str| Some(name.to_string());
    plan::create_project(
        &mut board,
        &ProjectFields {
            name: named("P"),
            ..ProjectFields::default()
        },
        &context,
        &at,
    )
    .unwrap();
    plan::create_release(
        &mut board,
        &ReleaseFields {
            name: named("R"),
            ..ReleaseFields::default()
        },
        &context,
        &at,
    )
    .unwrap();
    let parent = create_ticket(
        &mut board,
        &CreateTicketInput {
            title: "parent".into(),
            ..CreateTicketInput::default()
        },
        &context,
        &at,
    )
    .unwrap();
    for (title, status) in [
        ("shipped", "done"),
        ("dropped", "canceled"),
        ("open", "todo"),
    ] {
        create_ticket(
            &mut board,
            &CreateTicketInput {
                title: title.into(),
                status: named(status),
                parent: Some(parent.key.clone()),
                project: named("P"),
                release: named("R"),
                ..CreateTicketInput::default()
            },
            &context,
            &at,
        )
        .unwrap();
    }

    let view = ticket_view(
        &board,
        find_ticket(&board, &parent.key).unwrap(),
        "done",
        Some("canceled"),
    );
    assert_eq!((view.progress.done, view.progress.total), (1, 2));
    assert_eq!(view.children.len(), 3, "a canceled child is still listed");

    let project = plan::project_views(&board, &context, true).remove(0);
    assert_eq!((project.done, project.total), (1, 2));
    assert_eq!(project.counts["canceled"], 1, "the column count keeps it");
    let release = plan::release_views(&board, &context).remove(0);
    assert_eq!((release.done, release.total), (1, 2));

    let error = plan::ship_release(&mut board, "R", None, &context, &clock(2)).unwrap_err();
    assert_eq!(
        error, "1 tickets are not done; choose where they go (move_open_to)",
        "only the todo ticket is open"
    );
    let (shipped, moved) =
        plan::ship_release(&mut board, "R", Some(None), &context, &clock(3)).unwrap();
    assert_eq!(moved.len(), 1, "the canceled ticket stays in the release");
    assert_eq!(shipped.status, ReleaseStatus::Released);
    assert_eq!((shipped.done, shipped.total), (1, 1));
}
