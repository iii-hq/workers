//! The whole surface driven end to end over one store: create → get → list →
//! update → delete, asserting the two properties callers build on — the id is
//! stable for a canvas's whole life, and the side index never disagrees with
//! the records.
//!
//! Runs on the in-memory store (same key semantics as the state worker:
//! missing keys read as null, delete returns the old value), so it needs no
//! engine. The wire-level schema surface is golden-tested in
//! `tests/schemas.rs`.

use canvas::config::WorkerConfig;
use canvas::functions::{create, delete, get, list, update, validate};
use canvas::store::{CanvasFormat, Store};

#[tokio::test]
async fn a_canvas_lives_a_full_life_under_one_id() {
    let store = Store::in_memory();
    let cfg = WorkerConfig::default();

    // Validate before creating — the flow the function descriptions steer
    // agents toward.
    let verdict = validate::handle(
        &store,
        validate::Request {
            format: None,
            source: "flowchart TD\n  A[Draft] --> B[Review]\n".into(),
        },
        &cfg,
    )
    .await
    .expect("validate answers");
    assert!(verdict.valid, "issues: {:?}", verdict.issues);
    assert_eq!(verdict.family.as_deref(), Some("flowchart"));

    let created = create::handle(
        &store,
        create::Request {
            name: Some("release flow".into()),
            format: None,
            source: "flowchart TD\n  A[Draft] --> B[Review]\n".into(),
        },
        &cfg,
    )
    .await
    .expect("creates");
    let id = created.id.clone();

    let fetched = get::handle(&store, get::Request { id: id.clone() }, &cfg)
        .await
        .expect("gets");
    assert_eq!(fetched, created);

    let listed = list::handle(&store, list::Request::default(), &cfg)
        .await
        .expect("lists");
    assert_eq!(listed.count, 1);
    assert_eq!(listed.canvases[0].id, id);

    let updated = update::handle(
        &store,
        update::Request {
            id: id.clone(),
            name: None,
            source: Some("sequenceDiagram\n  A->>B: review please\n".into()),
        },
        &cfg,
    )
    .await
    .expect("updates");
    assert_eq!(updated.id, id, "the id must survive updates");
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(updated.family.as_deref(), Some("sequenceDiagram"));

    // The index row followed the update — list still shows exactly one
    // canvas, under the same id, with the new content.
    let relisted = list::handle(&store, list::Request::default(), &cfg)
        .await
        .expect("lists");
    assert_eq!(relisted.count, 1);
    assert_eq!(relisted.canvases[0], updated);

    let removed = delete::handle(&store, delete::Request { id: id.clone() }, &cfg)
        .await
        .expect("deletes");
    assert!(removed.deleted);

    // Index consistency after delete: get errors by name, list is empty.
    let gone = get::handle(&store, get::Request { id: id.clone() }, &cfg)
        .await
        .expect_err("deleted canvas no longer gets");
    assert!(gone.contains(&id), "{gone}");
    let empty = list::handle(&store, list::Request::default(), &cfg)
        .await
        .expect("lists");
    assert_eq!(empty.count, 0);
}

#[tokio::test]
async fn mermaid_and_freeform_coexist_and_filter_cleanly() {
    let store = Store::in_memory();
    let cfg = WorkerConfig::default();

    let diagram = create::handle(
        &store,
        create::Request {
            name: Some("erd".into()),
            format: None,
            source: "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n".into(),
        },
        &cfg,
    )
    .await
    .expect("creates mermaid");
    let board = create::handle(
        &store,
        create::Request {
            name: Some("sketch".into()),
            format: Some(CanvasFormat::Freeform),
            source: "{\"type\": \"excalidraw\", \"elements\": []}".into(),
        },
        &cfg,
    )
    .await
    .expect("creates freeform");

    assert_eq!(diagram.family.as_deref(), Some("erDiagram"));
    assert_eq!(board.family, None);
    assert_ne!(diagram.id, board.id);

    let boards_only = list::handle(
        &store,
        list::Request {
            format: Some(CanvasFormat::Freeform),
        },
        &cfg,
    )
    .await
    .expect("lists");
    assert_eq!(boards_only.count, 1);
    assert_eq!(boards_only.canvases[0].id, board.id);
}
