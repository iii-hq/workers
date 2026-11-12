//! The pure read contract and the real extraction path on synthetic loopback
//! and temp-dir fixtures: no engine, service, model or real data.
#[path = "../src/extract/history.rs"]
mod history;

use std::collections::VecDeque;

use history::{
    classify_root, Bounds, ClassificationReason, DeltaReader, FetchFailure, HistoryError, Outcome,
    RootClassification,
};
use serde_json::{json, Value};

fn entry(id: &str) -> Value {
    json!({"entry_id": id, "message": {"role": "user", "content": [
        {"type": "text", "text": "Synthetic fact kept whole."}
    ], "timestamp": 0}, "origin": {"synthetic": true}})
}

fn page(ids: &[&str], next: Option<&str>) -> Value {
    json!({"messages": ids.iter().map(|id| entry(id)).collect::<Vec<_>>(),
        "next_cursor": next})
}

fn reader(since: Option<&str>, until: Option<&str>) -> DeltaReader {
    DeltaReader::new(
        "synthetic-session",
        since,
        until,
        Bounds {
            max_pages: 100,
            max_entries: 10_000,
        },
    )
}

#[derive(Default)]
struct ScriptedTransport {
    replies: VecDeque<Result<Value, FetchFailure>>,
    requests: Vec<Value>,
}

impl ScriptedTransport {
    fn drive(&mut self, reader: &mut DeltaReader) {
        while let Some(request) = reader.next_request() {
            self.requests.push(request);
            let reply = self.replies.pop_front().expect("scripted synthetic reply");
            reader.accept(reply);
        }
    }
}

/// Only simulates server paging; never computes the expected result.
struct PagedTransport {
    active_path: Vec<Value>,
    requests: Vec<Value>,
    returned: usize,
}

impl PagedTransport {
    fn drive(&mut self, reader: &mut DeltaReader) {
        while let Some(request) = reader.next_request() {
            let start = if let Some(cursor) = request.get("cursor") {
                cursor
                    .as_str()
                    .unwrap()
                    .strip_prefix("opaque-page/")
                    .unwrap()
                    .parse::<usize>()
                    .unwrap()
            } else if let Some(after) = request.get("after_entry_id") {
                self.active_path
                    .iter()
                    .position(|item| item["entry_id"] == *after)
                    .expect("synthetic anchor exists")
                    + 1
            } else {
                0
            };
            let end =
                (start + request["limit"].as_u64().unwrap() as usize).min(self.active_path.len());
            let next = (end < self.active_path.len()).then(|| format!("opaque-page/{end}"));
            self.returned += end - start;
            self.requests.push(request);
            reader.accept(Ok(json!({"messages": self.active_path[start..end],
                "next_cursor": next})));
        }
    }
}

#[test]
fn thousand_entry_prefix_reads_only_thirty_new_entries() {
    let mut active_path: Vec<_> = (0..1000).map(|n| entry(&format!("old-{n}"))).collect();
    active_path.extend((0..30).map(|n| entry(&format!("delta-{n}"))));
    let mut transport = PagedTransport {
        active_path,
        requests: vec![],
        returned: 0,
    };
    let mut read = reader(Some("old-999"), Some("delta-29"));
    transport.drive(&mut read);

    assert_eq!(
        transport.requests,
        vec![json!({
            "session_id": "synthetic-session", "limit": 100,
            "roles": ["user", "assistant"], "include_image_data": false,
            "after_entry_id": "old-999"
        })]
    );
    assert_eq!(transport.returned, 30);
    let Outcome::Complete(delta) = read.outcome() else {
        panic!("expected a complete delta")
    };
    assert_eq!(delta.entries.len(), 30);
    assert_eq!(delta.watermark, "delta-29");
    assert_eq!(delta.entries[0].entry_id, "delta-0");
    assert_eq!(delta.entries[29].entry_id, "delta-29");
    assert!(delta
        .entries
        .iter()
        .all(|item| !item.entry_id.starts_with("old-")));
    assert_eq!(delta.entries[0].item["origin"], json!({"synthetic": true}));
}

#[test]
fn more_than_thousand_new_entries_keep_every_page_and_prefix() {
    let mut active_path = vec![entry("anchor")];
    active_path.extend((0..1230).map(|n| entry(&format!("new-{n}"))));
    let mut transport = PagedTransport {
        active_path,
        requests: vec![],
        returned: 0,
    };
    let mut read = reader(Some("anchor"), Some("new-1229"));
    transport.drive(&mut read);

    assert_eq!(transport.requests.len(), 13);
    assert_eq!(transport.returned, 1230);
    assert_eq!(
        transport.requests[1],
        json!({
            "session_id": "synthetic-session", "limit": 100,
            "roles": ["user", "assistant"], "include_image_data": false,
            "cursor": "opaque-page/101"
        })
    );
    assert!(transport.requests[1..]
        .iter()
        .all(|request| request.get("after_entry_id").is_none() && request.get("cursor").is_some()));
    assert!(transport
        .requests
        .iter()
        .all(|request| request.get("until").is_none() && request.get("from_entry_id").is_none()));
    let Outcome::Complete(delta) = read.outcome() else {
        panic!("expected a complete delta")
    };
    assert_eq!(delta.entries.len(), 1230);
    assert_eq!(delta.entries[0].entry_id, "new-0");
    assert_eq!(delta.entries[1000].entry_id, "new-1000");
    assert_eq!(delta.entries[1229].entry_id, "new-1229");
    assert_eq!(delta.watermark, "new-1229");
    for (actual, source) in delta.entries.iter().zip(&transport.active_path[1..]) {
        assert_eq!(&actual.item, source);
    }
}

#[test]
fn opaque_ids_keep_server_order_and_inclusive_client_cut() {
    let mut transport = ScriptedTransport {
        replies: [Ok(page(
            &["z/9", "a:1", "0-end", "later"],
            Some("not-an-entry-id"),
        ))]
        .into(),
        ..Default::default()
    };
    let mut read = reader(Some("middle-anchor"), Some("0-end"));
    transport.drive(&mut read);
    let Outcome::Complete(delta) = read.outcome() else {
        panic!("expected a complete cut")
    };
    assert_eq!(
        delta
            .entries
            .iter()
            .map(|item| item.entry_id.as_str())
            .collect::<Vec<_>>(),
        vec!["z/9", "a:1", "0-end"]
    );
    assert_eq!(delta.watermark, "0-end");
    assert_eq!(transport.requests.len(), 1);
    assert!(!delta.entries.iter().any(|item| item.entry_id == "later"));
}
#[test]
fn later_page_cut_keeps_prefix_and_never_requests_its_successor() {
    let mut transport = ScriptedTransport {
        replies: [
            Ok(page(&["z-first", "a-second"], Some("opaque-A"))),
            Ok(page(&["m-third", "0-end", "post-cut"], Some("opaque-B"))),
            Err(FetchFailure::Transient),
        ]
        .into(),
        ..Default::default()
    };
    let mut read = reader(Some("anchor"), Some("0-end"));
    transport.drive(&mut read);
    assert_eq!(transport.requests.len(), 2);
    assert_eq!(transport.replies.len(), 1);
    assert_eq!(transport.requests[0]["after_entry_id"], "anchor");
    assert_eq!(transport.requests[1]["cursor"], "opaque-A");
    assert!(transport.requests[1].get("after_entry_id").is_none());
    let Outcome::Complete(delta) = read.outcome() else {
        panic!("corte posterior completo")
    };
    assert_eq!(
        delta
            .entries
            .iter()
            .map(|item| item.entry_id.as_str())
            .collect::<Vec<_>>(),
        vec!["z-first", "a-second", "m-third", "0-end"]
    );
    assert_eq!(delta.watermark, "0-end");
    assert!(!delta.entries.iter().any(|item| item.entry_id == "post-cut"));
}

#[test]
fn malformed_suffix_does_not_release_an_unvalidated_page_prefix() {
    let mut read = reader(Some("anchor"), Some("end"));
    read.accept(Ok(json!({"messages": [entry("end"), {"entry_id": "bad"}]})));
    assert_eq!(
        read.outcome(),
        &Outcome::Failed(HistoryError::MalformedPage)
    );
    assert!(read.next_request().is_none());
}

#[test]
fn no_text_entries_and_unknown_blocks_remain_watermarks_without_text_filter() {
    let empty = json!({"entry_id": "empty", "message": {"role": "user", "content": []}});
    let non_text = json!({"entry_id": "image-end", "message": {"role": "assistant",
        "content": [{"type": "image", "data": ""}, {"type": "future_block"}]}});
    let mut read = reader(Some("anchor"), Some("image-end"));
    read.accept(Ok(json!({"messages": [empty.clone(), non_text.clone()]})));
    let Outcome::Complete(delta) = read.outcome() else {
        panic!("watermarks preservados")
    };
    assert_eq!(delta.entries.len(), 2);
    assert_eq!(delta.entries[0].item, empty);
    assert_eq!(delta.entries[1].item, non_text);
    assert_eq!(delta.watermark, "image-end");
}

#[test]
fn consumed_equal_boundary_is_noop_with_zero_requests() {
    let mut transport = ScriptedTransport::default();
    let mut read = reader(Some("same"), Some("same"));
    transport.drive(&mut read);
    assert_eq!(
        read.outcome(),
        &Outcome::AlreadyConsumed {
            watermark: "same".into()
        }
    );
    assert!(transport.requests.is_empty());
}

#[test]
fn missing_input_boundary_or_anchor_never_fetches_root_implicitly() {
    let mut no_until = reader(Some("anchor"), None);
    let mut no_anchor = reader(None, Some("end"));
    let mut transport = ScriptedTransport::default();
    transport.drive(&mut no_until);
    transport.drive(&mut no_anchor);
    assert_eq!(
        no_until.outcome(),
        &Outcome::NeedsClassification(ClassificationReason::MissingBoundary)
    );
    assert_eq!(
        no_anchor.outcome(),
        &Outcome::NeedsClassification(ClassificationReason::MissingAnchor)
    );
    assert!(transport.requests.is_empty());
}

#[test]
fn absent_until_at_leaf_discards_nonempty_delta_not_an_already_processed_job() {
    let mut transport = ScriptedTransport {
        replies: [
            Ok(page(&["new-1"], Some("cursor-A"))),
            Ok(page(&["new-2"], None)),
        ]
        .into(),
        ..Default::default()
    };
    let mut read = reader(Some("anchor"), Some("not-on-path"));
    transport.drive(&mut read);
    assert_eq!(
        read.outcome(),
        &Outcome::NeedsClassification(ClassificationReason::BoundaryNotFound)
    );
    assert!(read.next_request().is_none());
    assert_eq!(transport.requests.len(), 2);
}

#[test]
fn empty_terminal_page_does_not_infer_stale_or_complete() {
    let mut read = reader(Some("anchor"), Some("end"));
    read.accept(Ok(page(&[], None)));
    assert_eq!(
        read.outcome(),
        &Outcome::NeedsClassification(ClassificationReason::BoundaryNotFound)
    );
}

#[test]
fn root_classification_uses_path_positions_not_identifier_order() {
    assert_eq!(
        classify_root(Some("a"), "z", &["z", "a"], true),
        RootClassification::AlreadyCovered
    );
    assert_eq!(
        classify_root(Some("z"), "a", &["z", "a"], true),
        RootClassification::DeltaAvailable
    );
    assert_eq!(
        classify_root(Some("z"), "z", &["z", "a"], true),
        RootClassification::AlreadyCovered
    );
    assert_eq!(
        classify_root(Some("fork-parent"), "a", &["z", "a"], true),
        RootClassification::FallbackRequired
    );
    assert_eq!(
        classify_root(None, "a", &["z", "a"], true),
        RootClassification::FallbackRequired
    );
    assert_eq!(
        classify_root(Some("z"), "missing", &["z", "a"], true),
        RootClassification::BoundaryMissing
    );
    assert_eq!(
        classify_root(Some("a"), "z", &["z", "a"], false),
        RootClassification::Unknown
    );
    assert_eq!(
        classify_root(Some("a"), "z", &["z", "a", "z"], true),
        RootClassification::Malformed
    );
    assert_eq!(
        classify_root(Some("a"), "z", &["", "a"], true),
        RootClassification::Malformed
    );
    assert_eq!(
        classify_root(Some("a"), "", &["z", "a"], true),
        RootClassification::Malformed
    );
}

#[test]
fn invalid_cursor_requires_classification_without_retaining_partial_delta() {
    for failure_on_second_page in [false, true] {
        let mut read = reader(Some("anchor"), Some("end"));
        if failure_on_second_page {
            read.accept(Ok(page(&["prefix"], Some("cursor-A"))));
        }
        read.accept(Err(FetchFailure::InvalidCursor));
        assert_eq!(
            read.outcome(),
            &Outcome::NeedsClassification(ClassificationReason::InvalidCursor)
        );
        assert!(read.next_request().is_none());
    }
}

#[test]
fn transient_error_is_not_a_fork_or_empty_success_and_retry_starts_fresh() {
    let mut read = reader(Some("anchor"), Some("end"));
    read.accept(Ok(page(&["prefix"], Some("cursor-A"))));
    read.accept(Err(FetchFailure::Transient));
    assert_eq!(
        read.outcome(),
        &Outcome::Failed(HistoryError::Fetch(FetchFailure::Transient))
    );
    assert!(read.next_request().is_none());
    read.accept(Ok(page(&["end"], None)));
    assert_eq!(
        read.outcome(),
        &Outcome::Failed(HistoryError::Fetch(FetchFailure::Transient))
    );
    let mut retry = reader(Some("anchor"), Some("end"));
    assert_eq!(retry.next_request().unwrap()["after_entry_id"], "anchor");
    retry.accept(Ok(page(&["prefix", "end"], None)));
    let Outcome::Complete(delta) = retry.outcome() else {
        panic!("retry independente completo")
    };
    assert_eq!(delta.entries.len(), 2);
    assert_eq!(delta.entries[0].entry_id, "prefix");
}

#[test]
fn other_transport_error_does_not_authorize_fallback() {
    let mut read = reader(Some("anchor"), Some("end"));
    read.accept(Err(FetchFailure::Other));
    assert_eq!(
        read.outcome(),
        &Outcome::Failed(HistoryError::Fetch(FetchFailure::Other))
    );
}

#[test]
fn malformed_messages_and_ids_fail_after_a_valid_prefix() {
    let malformed = [
        Value::Null,
        json!({}),
        json!({"messages": null}),
        json!({"messages": {}}),
        json!({"messages": [null]}),
        json!({"messages": [{"message": {"role": "user", "content": []}}]}),
        json!({"messages": [{"entry_id": 1, "message": {"role": "user", "content": []}}]}),
        json!({"messages": [{"entry_id": "", "message": {"role": "user", "content": []}}]}),
        json!({"messages": [{"entry_id": "end"}]}),
        json!({"messages": [{"entry_id": "end", "message": {"role": "tool", "content": []}}]}),
        json!({"messages": [{"entry_id": "end", "message": {"role": "user", "content": "text"}}]}),
        json!({"messages": [{"entry_id": "end", "message": {"role": "user", "content": []}, "custom": {}}]}),
    ];
    for reply in malformed {
        let mut read = reader(Some("anchor"), Some("end"));
        read.accept(Ok(page(&["prefix"], Some("cursor-A"))));
        read.accept(Ok(reply.clone()));
        assert_eq!(
            read.outcome(),
            &Outcome::Failed(HistoryError::MalformedPage),
            "{reply}"
        );
        assert!(read.next_request().is_none());
    }
}

#[test]
fn malformed_cursor_shape_fails_even_when_until_is_present() {
    for cursor in [json!(""), json!(1), json!([]), json!({}), json!(false)] {
        let mut read = reader(Some("anchor"), Some("end"));
        read.accept(Ok(
            json!({"messages": [entry("end")], "next_cursor": cursor}),
        ));
        assert_eq!(
            read.outcome(),
            &Outcome::Failed(HistoryError::MalformedPage)
        );
    }
}

#[test]
fn missing_and_null_next_cursor_are_both_valid_terminal_shapes() {
    for reply in [json!({"messages": [entry("end")]}), page(&["end"], None)] {
        let mut read = reader(Some("anchor"), Some("end"));
        read.accept(Ok(reply));
        assert!(matches!(read.outcome(), Outcome::Complete(delta) if delta.watermark == "end"));
    }
}

#[test]
fn repeated_or_cyclic_cursors_discard_all_pages_instead_of_looping() {
    for last_cursor in ["cursor-A", "cursor-B"] {
        let mut read = reader(Some("anchor"), Some("end"));
        read.accept(Ok(page(&["first"], Some("cursor-A"))));
        read.accept(Ok(page(&["second"], Some("cursor-B"))));
        read.accept(Ok(page(&["end"], Some(last_cursor))));
        assert_eq!(
            read.outcome(),
            &Outcome::Failed(HistoryError::RepeatedCursor)
        );
        assert!(read.next_request().is_none());
    }
}

#[test]
fn duplicate_and_backtracking_ids_are_not_deduplicated_into_success() {
    let mut within_page = reader(Some("anchor"), Some("end"));
    within_page.accept(Ok(page(&["x", "x", "end"], None)));
    assert_eq!(
        within_page.outcome(),
        &Outcome::Failed(HistoryError::RepeatedEntry)
    );
    let mut cross_page = reader(Some("anchor"), Some("end"));
    cross_page.accept(Ok(page(&["z", "a"], Some("cursor-A"))));
    cross_page.accept(Ok(page(&["z", "end"], None)));
    assert_eq!(
        cross_page.outcome(),
        &Outcome::Failed(HistoryError::RepeatedEntry)
    );
    let mut anchor = reader(Some("anchor"), Some("end"));
    anchor.accept(Ok(page(&["anchor", "end"], None)));
    assert_eq!(
        anchor.outcome(),
        &Outcome::Failed(HistoryError::AnchorInDelta)
    );
}

#[test]
fn empty_page_with_continuation_is_no_progress() {
    let mut read = reader(Some("anchor"), Some("end"));
    read.accept(Ok(page(&[], Some("cursor-A"))));
    assert_eq!(read.outcome(), &Outcome::Failed(HistoryError::NoProgress));
}

#[test]
fn page_cap_is_failure_not_silent_complete_or_a_four_run_window() {
    let mut read = DeltaReader::new(
        "synthetic-session",
        Some("anchor"),
        Some("end"),
        Bounds {
            max_pages: 2,
            max_entries: 1000,
        },
    );
    let mut transport = ScriptedTransport {
        replies: [
            Ok(page(&["prefix"], Some("cursor-A"))),
            Ok(page(&["middle"], Some("cursor-B"))),
        ]
        .into(),
        ..Default::default()
    };
    transport.drive(&mut read);
    assert_eq!(transport.requests.len(), 2);
    assert_eq!(read.outcome(), &Outcome::Failed(HistoryError::PageLimit));
}

#[test]
fn entry_cap_rejects_partial_and_checks_the_whole_received_page() {
    let mut read = DeltaReader::new(
        "synthetic-session",
        Some("anchor"),
        Some("end"),
        Bounds {
            max_pages: 10,
            max_entries: 2,
        },
    );
    read.accept(Ok(page(&["prefix"], Some("cursor-A"))));
    read.accept(Ok(page(&["end", "later"], None)));
    assert_eq!(read.outcome(), &Outcome::Failed(HistoryError::EntryLimit));
    let mut exact_pending = DeltaReader::new(
        "synthetic-session",
        Some("anchor"),
        Some("end"),
        Bounds {
            max_pages: 10,
            max_entries: 2,
        },
    );
    exact_pending.accept(Ok(page(&["a", "b"], Some("cursor-A"))));
    assert_eq!(
        exact_pending.outcome(),
        &Outcome::Failed(HistoryError::EntryLimit)
    );
}

#[test]
fn exact_bounds_with_until_are_complete_and_terminal_results_are_immutable() {
    let mut read = DeltaReader::new(
        "synthetic-session",
        Some("anchor"),
        Some("end"),
        Bounds {
            max_pages: 1,
            max_entries: 2,
        },
    );
    read.accept(Ok(page(&["prefix", "end"], Some("unused-cursor"))));
    let snapshot = read.outcome().clone();
    assert!(matches!(&snapshot, Outcome::Complete(delta) if delta.entries.len() == 2));
    read.accept(Err(FetchFailure::Transient));
    assert_eq!(read.outcome(), &snapshot);
    assert!(read.next_request().is_none());
}

#[test]
fn oversized_page_is_a_protocol_error_not_truncated_success() {
    let mut read = reader(Some("anchor"), Some("entry-100"));
    let entries: Vec<_> = (0..101).map(|n| entry(&format!("entry-{n}"))).collect();
    read.accept(Ok(json!({"messages": entries})));
    assert_eq!(read.outcome(), &Outcome::Failed(HistoryError::PageTooLarge));
}

#[test]
fn invalid_inputs_and_zero_bounds_never_issue_requests() {
    for read in [
        DeltaReader::new(
            "",
            Some("a"),
            Some("b"),
            Bounds {
                max_pages: 1,
                max_entries: 1,
            },
        ),
        DeltaReader::new(
            "s",
            Some(""),
            Some("b"),
            Bounds {
                max_pages: 1,
                max_entries: 1,
            },
        ),
        DeltaReader::new(
            "s",
            Some("a"),
            Some(""),
            Bounds {
                max_pages: 1,
                max_entries: 1,
            },
        ),
        DeltaReader::new(
            "s",
            Some("a"),
            Some("b"),
            Bounds {
                max_pages: 0,
                max_entries: 1,
            },
        ),
        DeltaReader::new(
            "s",
            Some("a"),
            Some("b"),
            Bounds {
                max_pages: 1,
                max_entries: 0,
            },
        ),
    ] {
        assert_eq!(read.outcome(), &Outcome::Failed(HistoryError::InvalidInput));
        assert!(read.next_request().is_none());
    }
}

// The real path: SDK on an ephemeral loopback only, a temp store, embeddings
// off and a fixed model the fixture answers.
mod productive {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use memory::{config::WorkerConfig, deps::Deps, hooks::ExtractJobInput};
    use std::sync::{Arc, Mutex};

    // The envelope the producer emits: the session worker returns
    // `Error::Handler("session/invalid_cursor: ...")` (error.rs From<SessionError>);
    // iii-sdk 0.26.0 dispatch (`error_body`) sends a non-`Remote` error as code
    // `invocation_failed`, message = Display (`handler error: {0}`).
    const SDK_INVALID_CURSOR: (&str, &str) = (
        "invocation_failed",
        "handler error: session/invalid_cursor: anchor not on path",
    );

    #[test]
    fn fixture_envelope_is_the_sdk_display_of_the_producer_handler_error() {
        let produced = iii_sdk::errors::Error::Handler(
            "session/invalid_cursor: anchor not on path".to_string(),
        );
        assert_eq!(SDK_INVALID_CURSOR.1, produced.to_string());
    }

    type Reply = Result<Value, (&'static str, &'static str)>;
    #[derive(Clone, Default)]
    struct Script {
        path: Vec<Value>,
        cursor: Option<String>,
        calls: Vec<(String, Value)>,
        pages: VecDeque<Reply>,
        model_items: VecDeque<Value>,
        returned: Vec<usize>,
        state_read_error: bool,
        state_function_absent: bool,
        state_write_error: bool,
        session_error: bool,
        kind: Option<String>,
        cursor_reply: Option<Value>,
        model_failures: usize,
        // Real temporary files survive fixture SDK/store reconstruction; no CAS emulation.
        persistent_cursor: bool,
        observe_events: bool,
        cursor_file: Option<std::path::PathBuf>,
        model_pause: Option<Arc<(tokio::sync::Semaphore, tokio::sync::Semaphore)>>,
    }
    impl Script {
        fn answer(&mut self, function: &str, data: &Value) -> Reply {
            self.calls.push((function.into(), data.clone()));
            match function {
                "session::get" if self.session_error => {
                    Err(("invocation_error", "session/storage: unavailable"))
                }
                "session::get" => Ok(
                    json!({"meta":{"session_id":data["session_id"],"kind":self.kind,"metadata":{"memory_bank":"private"}}}),
                ),
                "session::list" => Ok(json!({"sessions":[],"next_cursor":null})),
                "state::get" | "state::set" if self.state_function_absent => {
                    Err(("function_not_found", "offline state worker absent"))
                }
                "state::get" if self.state_read_error => {
                    Err(("invocation_error", "state unavailable"))
                }
                "state::get" if self.cursor_file.is_some() => {
                    let path = self.cursor_file.as_ref().unwrap();
                    match std::fs::read(path) {
                        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
                        Err(e) => panic!("temporary cursor read: {e}"),
                    }
                }
                "state::get" => Ok(self.cursor_reply.clone().unwrap_or_else(|| {
                    self.cursor
                        .as_ref()
                        .map_or(Value::Null, |id| json!({"last_entry_id":id}))
                })),
                "state::set" if self.state_write_error => {
                    Err(("invocation_error", "state unavailable"))
                }
                "state::set" => {
                    if let Some(path) = &self.cursor_file {
                        std::fs::write(path, serde_json::to_vec(&data["value"]).unwrap()).unwrap();
                    }
                    self.cursor = Some(data["value"]["last_entry_id"].as_str().unwrap().into());
                    Ok(json!({"ok":true}))
                }
                "session::messages" => {
                    assert_eq!(data["roles"], json!(["user", "assistant"]));
                    assert!(data.get("until").is_none());
                    assert!(data.get("from_entry_id").is_none());
                    if let Some(reply) = self.pages.pop_front() {
                        return reply;
                    }
                    let start = if let Some(cursor) = data["cursor"].as_str() {
                        assert!(data.get("after_entry_id").is_none());
                        cursor
                            .strip_prefix("page/")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap()
                    } else if let Some(anchor) = data["after_entry_id"].as_str() {
                        let Some(at) = self.path.iter().position(|item| item["entry_id"] == anchor)
                        else {
                            return Err(SDK_INVALID_CURSOR);
                        };
                        at + 1
                    } else {
                        0
                    };
                    let end = (start + 100).min(self.path.len());
                    self.returned.push(end - start);
                    Ok(
                        json!({"messages":self.path[start..end],"next_cursor":(end < self.path.len()).then(||format!("page/{end}"))}),
                    )
                }
                "memory::on-item-changed" => {
                    assert!(self.observe_events);
                    let root = self.cursor_file.as_ref().unwrap().parent().unwrap();
                    let bytes = std::fs::read_to_string(
                        root.join(data["bank"].as_str().unwrap())
                            .join("memories.jsonl"),
                    )
                    .unwrap();
                    let persisted = bytes
                        .lines()
                        .map(|line| serde_json::from_str::<Value>(line).unwrap())
                        .rfind(|record| record["id"] == data["memory"]["id"])
                        .unwrap();
                    assert_eq!(
                        persisted, data["memory"],
                        "event only after actual store publication"
                    );
                    Ok(Value::Null)
                }
                "memory::on-bank-changed" => {
                    assert!(self.observe_events);
                    Ok(Value::Null)
                }
                "router::complete" => {
                    assert_eq!(data["model"], "fixture-only");
                    if self.model_failures > 0 {
                        self.model_failures -= 1;
                        return Ok(
                            json!({"message":{"stop_reason":"error","error_kind":"fixture","error_message":"retry required","content":[]}}),
                        );
                    }
                    let items = self.model_items.pop_front().expect("unexpected model call");
                    Ok(
                        json!({"message":{"role":"assistant","stop_reason":"stop","content":[{"type":"text","text":items.to_string()}]}}),
                    )
                }
                _ => Err(("unexpected_rpc", "RPC denied by default")),
            }
        }
        fn count(&self, function: &str) -> usize {
            self.calls.iter().filter(|(id, _)| id == function).count()
        }
        fn requests(&self) -> Vec<Value> {
            self.calls
                .iter()
                .filter(|(id, _)| id == "session::messages")
                .map(|(_, data)| data.clone())
                .collect()
        }
        fn publication_trace(&self) -> Vec<(String, String)> {
            self.calls
                .iter()
                .filter_map(|(id, data)| match id.as_str() {
                    "memory::on-item-changed" => {
                        Some(("event".into(), data["event_type"].as_str().unwrap().into()))
                    }
                    "state::set" => Some((
                        "cursor".into(),
                        data["value"]["last_entry_id"].as_str().unwrap().into(),
                    )),
                    _ => None,
                })
                .collect()
        }
        fn inputs(&self) -> Vec<String> {
            self.calls
                .iter()
                .filter(|(id, _)| id == "router::complete")
                .map(|(_, data)| {
                    data["messages"][0]["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .into()
                })
                .collect()
        }
    }
    struct Boundary {
        deps: Deps,
        script: Arc<Mutex<Script>>,
        server: tokio::task::JoinHandle<()>,
        dir: tempfile::TempDir,
    }
    impl Drop for Boundary {
        fn drop(&mut self) {
            self.deps.iii.shutdown();
            self.server.abort();
        }
    }
    impl Boundary {
        async fn new(script: Script) -> Self {
            Self::new_in(script, tempfile::tempdir().unwrap()).await
        }
        async fn new_in(mut script: Script, dir: tempfile::TempDir) -> Self {
            if script.persistent_cursor {
                let path = dir.path().join("synthetic-cursor.json");
                if !path.exists() {
                    if let Some(id) = &script.cursor {
                        std::fs::write(&path, json!({"last_entry_id":id}).to_string()).unwrap();
                    }
                }
                script.cursor_file = Some(path);
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            let observe_events = script.observe_events;
            let script = Arc::new(Mutex::new(script));
            let replies = script.clone();
            let (ready, wait) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
                // SDK observers become usable only after the engine feature
                // handshake; TCP readiness alone is not registration readiness.
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        json!({"type":"workerregistered","worker_id":"offline-memory",
                        "features":["observe.v2"]})
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                let _ = ready.send(());
                while let Some(Ok(frame)) = socket.next().await {
                    let Ok(text) = frame.to_text() else { continue };
                    let Ok(request) = serde_json::from_str::<Value>(text) else {
                        continue;
                    };
                    if request["type"] == "registerfunction" {
                        let id = request["id"].as_str().unwrap();
                        assert!(matches!(
                            id,
                            "memory::on-item-changed"
                                | "memory::on-bank-changed"
                                | memory::events::ON_EXTRACTION_DECISION_FN
                        ));
                        if id == memory::events::ON_EXTRACTION_DECISION_FN {
                            assert_eq!(request["private"], true);
                            assert!(request["request_format"].is_object());
                            assert_eq!(request["request_format"], request["response_format"]);
                        }
                        let registered = json!({"type":"functionregistered","id":id,
                            "namespace":"default","revision":1,"local_dispatch":false,
                            "observers":[{"id":format!("offline-{id}"),"target":"fixture::sink","filters":{}}]});
                        socket
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                registered.to_string().into(),
                            ))
                            .await
                            .unwrap();
                        continue;
                    }
                    if request["type"] != "invokefunction" {
                        continue;
                    }
                    let function = request["function_id"].as_str().unwrap();
                    let mut reply = json!({"type":"invocationresult","invocation_id":request["invocation_id"],"function_id":function});
                    if function == "router::complete" {
                        let pause = replies.lock().unwrap().model_pause.clone();
                        if let Some(pause) = pause {
                            pause.0.add_permits(1);
                            pause.1.acquire().await.unwrap().forget();
                        }
                    }
                    if function == "fixture::capture-barrier" {
                        // This response follows every preceding source call on
                        // this SDK connection; it is not observer delivery proof.
                        reply["result"] = request["data"].clone();
                    } else if function == "engine::workers::register" {
                        reply["result"] = json!({"success":true});
                    } else {
                        match replies.lock().unwrap().answer(function, &request["data"]) {
                            Ok(value) => reply["result"] = value,
                            Err((code, message)) => {
                                reply["error"] = json!({"code":code,"message":message})
                            }
                        }
                    }
                    if request["invocation_id"].is_null() {
                        continue; // SDK Void events have no response channel.
                    }
                    if socket
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            reply.to_string().into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let iii = Arc::new(iii_sdk::register_worker(
                &url,
                iii_sdk::InitOptions::default(),
            ));
            tokio::time::timeout(std::time::Duration::from_secs(5), wait)
                .await
                .expect("bounded mock connection")
                .unwrap();
            if observe_events {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while iii.engine_supports_observe() != Some(true) {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("offline feature handshake bounded");
                memory::events::register_all(&iii);
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while !iii.is_observed("memory::on-item-changed")
                        || !iii.is_observed("memory::on-bank-changed")
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("offline event registration bounded");
            }
            let deps = Deps {
                hook_binding: Arc::default(),
                emitter: Arc::new(memory::events::Emitter::new(iii.clone())),
                iii,
                store: Arc::new(tokio::sync::RwLock::new(Arc::new(
                    memory::store::Store::open(dir.path().to_path_buf()).unwrap(),
                ))),
                config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
                    embeddings_enabled: false,
                    extraction_enabled: true,
                    extraction_model: "fixture-only".into(),
                    default_bank: "public".into(),
                    ..WorkerConfig::default()
                }))),
            };
            Self {
                deps,
                script,
                server,
                dir,
            }
        }
        // Void completion means queued, not captured by the loopback server.
        // Await an ordered, correlated RPC before inspecting zero/exact counts
        // or handing capture to a restarted SDK. Never swallow a failed barrier.
        async fn capture_barrier(&self, phase: &str, until: Option<&str>) {
            if !self.script.lock().unwrap().observe_events {
                return;
            }
            let correlation = json!({"session_id":"incremental-fixture", "phase":phase,
                "until":until});
            let captured = self
                .deps
                .iii
                .call(iii_sdk::protocol::CallRequest::new(
                    "fixture::capture-barrier",
                    correlation.clone(),
                ))
                .await
                .expect("SDK source capture barrier must succeed");
            assert_eq!(captured, correlation);
        }
        async fn run(&self, job: &ExtractJobInput) -> Result<(), String> {
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                let result = memory::extract::try_run(&self.deps, job).await;
                self.capture_barrier("run-terminal", job.until.as_deref())
                    .await;
                result
            })
            .await
            .expect("bounded fixture");
            self.assert_rpc_scope();
            result
        }
        fn commits(&self) -> usize {
            std::fs::read_to_string(self.dir.path().join("private/memories.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(strict_memory_record)
                .count()
        }
        fn assert_rpc_scope(&self) {
            for (id, data) in &self.script.lock().unwrap().calls {
                assert!(
                    matches!(
                        id.as_str(),
                        "state::get"
                            | "state::set"
                            | "session::get"
                            | "session::messages"
                            | "session::list"
                            | "router::complete"
                            | "memory::on-item-changed"
                            | "memory::on-bank-changed"
                    ),
                    "unexpected RPC: {id}"
                );
                if matches!(id.as_str(), "session::get" | "session::messages") {
                    assert_eq!(data["session_id"], "incremental-fixture");
                }
                if matches!(id.as_str(), "state::get" | "state::set") {
                    assert_eq!(data["key"], "incremental-fixture");
                    assert_eq!(data["scope"], "memory_cursor");
                }
            }
        }
    }
    // Only the exact typed internal receipt is excluded. Invalid JSON and
    // malformed memory records remain hard failures, never missing records.
    fn strict_memory_record(line: &str) -> Option<memory::types::Memory> {
        let value: Value = serde_json::from_str(line).expect("valid JSONL record");
        if value.as_object().is_some_and(|object| {
            (object.len() == 1
                || (object.len() == 2
                    && object.get("disposition") == Some(&json!("expected_conflict"))))
                && object
                    .get("extraction_receipt")
                    .is_some_and(Value::is_string)
        }) {
            None
        } else {
            Some(serde_json::from_value(value).expect("valid memory record"))
        }
    }
    #[test]
    fn strict_log_assertion_excludes_only_typed_receipts() {
        assert!(strict_memory_record(r#"{"extraction_receipt":"source"}"#).is_none());
        assert!(strict_memory_record(
            r#"{"extraction_receipt":"source","disposition":"expected_conflict"}"#
        )
        .is_none());
        for malformed in [
            "not-json",
            r#"{"extraction_receipt":null}"#,
            r#"{"extraction_receipt":"source","unexpected":true}"#,
            r#"{"extraction_receipt":"source","disposition":"unknown"}"#,
            r#"{"text":"missing-id"}"#,
        ] {
            assert!(std::panic::catch_unwind(|| strict_memory_record(malformed)).is_err());
        }
    }
    fn message(id: &str, text: &str) -> Value {
        json!({"entry_id":id,"message":{"role":"user","content":[{"type":"text","text":text}],"timestamp":1_760_443_200_000_u64}})
    }
    fn job(until: &str) -> ExtractJobInput {
        serde_json::from_value(
            json!({"session_id":"incremental-fixture","bank":"private","until":until}),
        )
        .unwrap()
    }
    fn fact(text: &str) -> Value {
        json!([{"text":text,"entities":["fixture"],"scope":"project","kind":"memory","confidence":"extracted"}])
    }

    #[tokio::test]
    async fn productive_delta_thirty_and_twelve_thirty_use_only_new_pages() {
        for (delta, pages, models, end) in [(30, 1, 1, "new-29"), (1230, 13, 2, "new-1229")] {
            let mut path: Vec<_> = (0..1000)
                .map(|n| message(&format!("old-{n}"), "OLD_PREFIX"))
                .collect();
            path.extend((0..delta).map(|n| message(&format!("new-{n}"), "delta")));
            let stack = Boundary::new(Script {
                path,
                cursor: Some("old-999".into()),
                model_items: VecDeque::from([
                    fact("Amber instruments require copper calibration."),
                    fact("Birch archives retain monthly invoices."),
                ]),
                ..Script::default()
            })
            .await;
            stack.run(&job(end)).await.unwrap();
            {
                let script = stack.script.lock().unwrap();
                let requests = script.requests();
                assert_eq!(requests.len(), pages);
                assert_eq!(requests[0]["after_entry_id"], "old-999");
                assert!(requests.iter().all(|r| r["include_image_data"] == false));
                for r in &requests[1..] {
                    assert!(r.get("after_entry_id").is_none());
                    assert!(r["cursor"].is_string());
                }
                assert_eq!(script.count("router::complete"), models);
                assert_eq!(script.count("state::set"), models);
                assert_eq!(script.returned.iter().sum::<usize>(), delta);
                assert_eq!(script.returned.last(), Some(&30));
                assert_eq!(script.cursor.as_deref(), Some(end));
                assert!(!script.inputs()[0].contains("OLD_PREFIX"));
                assert_eq!(script.count("session::get"), 1);
                assert_eq!(script.count("state::get"), 1);
                assert_eq!(
                    script
                        .inputs()
                        .iter()
                        .map(|input| input.matches("delta").count())
                        .sum::<usize>(),
                    delta
                );
            }
            assert_eq!(stack.commits(), models);
            let store = stack.deps.store().await;
            let bank = store.bank("private").await.unwrap();
            let mut texts: Vec<_> = bank
                .list(10, 0, false, None)
                .await
                .0
                .into_iter()
                .map(|m| m.text)
                .collect();
            texts.sort();
            let expected = if models == 1 {
                vec!["Amber instruments require copper calibration."]
            } else {
                vec![
                    "Amber instruments require copper calibration.",
                    "Birch archives retain monthly invoices.",
                ]
            };
            assert_eq!(texts, expected);
            assert!(store.bank("public").await.is_err());
            stack.assert_rpc_scope();
        }
    }
    #[tokio::test]
    async fn valid_baseline_and_delta_preserve_the_same_fixed_facts() {
        let mut outcomes = Vec::new();
        for incremental in [false, true] {
            let mut path = if incremental {
                (0..1000)
                    .map(|n| message(&format!("old-{n}"), "OLD_PREFIX"))
                    .collect()
            } else {
                Vec::new()
            };
            path.extend(
                (0..30).map(|n| message(&format!("new-{n}"), "a durable synthetic delta line")),
            );
            let stack = Boundary::new(Script {
                path,
                cursor: incremental.then(|| "old-999".into()),
                model_items: VecDeque::from([fact(
                    "Amber instruments require copper calibration.",
                )]),
                ..Script::default()
            })
            .await;
            // A valid root baseline: an explicit window covers all 30 lines,
            // without claiming the default 12-line bootstrap recovers them.
            *stack.deps.config.write().await = Arc::new(WorkerConfig {
                embeddings_enabled: false,
                extraction_enabled: true,
                extraction_window: 30,
                extraction_model: "fixture-only".into(),
                default_bank: "public".into(),
                ..WorkerConfig::default()
            });
            stack.run(&job("new-29")).await.unwrap();
            let store = stack.deps.store().await;
            let facts = store
                .bank("private")
                .await
                .unwrap()
                .list(10, 0, false, None)
                .await
                .0;
            let texts: Vec<_> = facts.into_iter().map(|m| m.text).collect();
            assert_eq!(texts, vec!["Amber instruments require copper calibration."]);
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 1);
            assert_eq!(s.returned, vec![30]);
            assert_eq!(s.count("router::complete"), 1);
            assert_eq!(s.count("state::set"), 1);
            assert_eq!(s.cursor.as_deref(), Some("new-29"));
            assert_eq!(stack.commits(), 1);
            outcomes.push(texts);
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }

    #[tokio::test]
    async fn productive_model_sees_only_the_inclusive_cut_and_keeps_filtered_end_watermark() {
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", "old"),
                message("fact", "ELIGIBLE_DURABLE_FACT"),
                message(
                    "end",
                    "<memory bank=\"synthetic\">filtered injection</memory>",
                ),
                message("later", "AFTER_UNTIL_NOT_ELIGIBLE"),
            ],
            cursor: Some("anchor".into()),
            model_items: VecDeque::from([fact("Amber instruments require copper calibration.")]),
            ..Script::default()
        })
        .await;
        stack.run(&job("end")).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 1);
        assert_eq!(s.returned, vec![3]);
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("state::set"), 1);
        assert_eq!(s.cursor.as_deref(), Some("end"));
        assert!(s.inputs()[0].contains("ELIGIBLE_DURABLE_FACT"));
        assert!(!s.inputs()[0].contains("filtered injection"));
        assert!(!s.inputs()[0].contains("AFTER_UNTIL_NOT_ELIGIBLE"));
        assert_eq!(stack.commits(), 1);
    }

    #[tokio::test]
    async fn no_text_avoids_model_and_empty_model_reply_does_not_create_memories() {
        for no_text in [true, false] {
            let stack = Boundary::new(Script {
                path: vec![
                    message("anchor", ""),
                    message(
                        "end",
                        if no_text {
                            ""
                        } else {
                            "a durable synthetic excerpt with nothing new"
                        },
                    ),
                ],
                cursor: Some("anchor".into()),
                model_items: VecDeque::from([json!([])]),
                ..Script::default()
            })
            .await;
            stack.run(&job("end")).await.unwrap();
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 1);
            assert_eq!(s.count("router::complete"), usize::from(!no_text));
            assert_eq!(s.count("state::set"), 1);
            assert_eq!(s.cursor.as_deref(), Some("end"));
            assert_eq!(stack.commits(), 0);
        }
    }

    #[tokio::test]
    async fn missing_boundary_fails_and_equal_boundary_resolves_bank_without_requests() {
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", "old"),
                message("leaf", "must not extract durable leaf"),
            ],
            cursor: Some("anchor".into()),
            ..Script::default()
        })
        .await;
        assert!(stack.run(&job("missing")).await.is_err());
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.cursor.as_deref(), Some("anchor"));
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 0);
        }
        stack.script.lock().unwrap().calls.clear();
        let mut equal = job("anchor");
        equal.bank = None;
        stack.run(&equal).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert!(s.count("session::get") >= 1);
        assert_eq!(s.requests().len(), 0);
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("state::set"), 0);
        assert_eq!(stack.commits(), 0);
    }
    #[tokio::test]
    async fn inclusive_cut_keeps_empty_watermark_and_trivia_avoids_model() {
        for empty in [true, false] {
            let mut end = message("end", if empty { "" } else { "ok" });
            if empty {
                end["message"]["content"] = json!([{"type":"future-block","data":{}}]);
            }
            let stack = Boundary::new(Script {
                path: vec![
                    message("anchor", "old"),
                    message("tiny", "hi"),
                    end,
                    message("later", "LATER_BOUNDARY"),
                ],
                cursor: Some("anchor".into()),
                ..Script::default()
            })
            .await;
            stack.run(&job("end")).await.unwrap();
            let s = stack.script.lock().unwrap();
            assert_eq!(s.cursor.as_deref(), Some("end"));
            assert_eq!(s.requests().len(), 1);
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 1);
            assert_eq!(stack.commits(), 0);
        }
    }
    #[tokio::test]
    async fn invalid_anchor_classifies_root_and_old_job_requires_proven_order() {
        let stack = Boundary::new(Script {
            path: vec![
                message("first", "old prefix"),
                message("end", "new durable boundary fact"),
                message("later", "LATER_NOT_ELIGIBLE"),
            ],
            cursor: Some("forked-away".into()),
            model_items: VecDeque::from([fact("A fork uses the bounded legacy bootstrap.")]),
            ..Script::default()
        })
        .await;
        stack.run(&job("end")).await.unwrap();
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 2);
            assert_eq!(s.requests()[0]["after_entry_id"], "forked-away");
            assert!(s.requests()[1].get("after_entry_id").is_none());
            assert!(!s.inputs()[0].contains("LATER_NOT_ELIGIBLE"));
            assert_eq!(s.cursor.as_deref(), Some("end"));
        }
        stack.script.lock().unwrap().calls.clear();
        stack.run(&job("first")).await.unwrap();
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 2, "delta ausente exige root completo");
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 0);
            assert_eq!(s.cursor.as_deref(), Some("end"));
        }
        assert_eq!(stack.commits(), 1);
        stack.assert_rpc_scope();
    }
    #[tokio::test]
    async fn malformed_transient_repeat_and_partial_invalid_delta_fail_closed() {
        let scripts: Vec<VecDeque<Reply>> = vec![
            VecDeque::from([Ok(json!({"messages":"malformed"}))]),
            VecDeque::from([Err(("invocation_error", "transient service failure"))]),
            // The same SDK code without the exact `handler error: ` prefix.
            VecDeque::from([Err((
                "invocation_failed",
                "session/invalid_cursor: anchor not on path",
            ))]),
            // Exact prefix, but another producer's code.
            VecDeque::from([Err((
                "invocation_error",
                "handler error: session/invalid_cursor: anchor not on path",
            ))]),
            // A free substring inside a transient SDK error.
            VecDeque::from([Err((
                "invocation_failed",
                "handler error: temporary: session/invalid_cursor: anchor",
            ))]),
            // Another session-manager handler error.
            VecDeque::from([Err((
                "invocation_failed",
                "handler error: session/storage: unavailable",
            ))]),
            VecDeque::from([
                Ok(json!({"messages":[message("a","partial fact")],"next_cursor":"repeat"})),
                Ok(json!({"messages":[message("b","second fact")],"next_cursor":"repeat"})),
            ]),
            VecDeque::from([
                Ok(json!({"messages":[message("partial","must discard")],"next_cursor":"page/1"})),
                Err(SDK_INVALID_CURSOR),
                Ok(json!({"messages":[message("root-only","boundary absent")],"next_cursor":null})),
            ]),
        ];
        for pages in scripts {
            let stack = Boundary::new(Script {
                pages,
                cursor: Some("anchor".into()),
                ..Script::default()
            })
            .await;
            assert!(stack.run(&job("end")).await.is_err());
            let s = stack.script.lock().unwrap();
            assert_eq!(s.cursor.as_deref(), Some("anchor"));
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 0);
            assert_eq!(stack.commits(), 0);
        }
    }
    #[tokio::test]
    async fn state_read_session_and_cursor_write_errors_are_observable() {
        for (read, session, write) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let stack = Boundary::new(Script {
                path: vec![message("anchor", ""), message("end", "ok")],
                cursor: Some("anchor".into()),
                state_read_error: read,
                session_error: session,
                state_write_error: write,
                ..Script::default()
            })
            .await;
            assert!(stack.run(&job("end")).await.is_err());
            let s = stack.script.lock().unwrap();
            assert_eq!(s.cursor.as_deref(), Some("anchor"));
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), usize::from(write));
            assert_eq!(stack.commits(), 0);
        }
    }
    #[tokio::test]
    async fn backlog_drains_oldest_four_then_retry_without_replaying_persisted_runs() {
        let mut path = vec![message("anchor", "")];
        path.extend((1..=6).map(|n| {
            message(
                &format!("paste-{n}"),
                &format!("PASTE_{n} {}", "x".repeat(12_000)),
            )
        }));
        path.push(message("end", ""));
        let model_items = [
            "Amber instruments require copper calibration.",
            "Birch archives retain monthly invoices.",
            "Cobalt dispatch uses a northern warehouse.",
            "Dahlia workers prefer ceramic containers.",
            "Emerald routers expose twelve sockets.",
            "Fuchsia ledgers record quarterly balances.",
        ]
        .into_iter()
        .map(fact)
        .collect();
        let stack = Boundary::new(Script {
            path,
            cursor: Some("anchor".into()),
            model_items,
            ..Script::default()
        })
        .await;
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("pending"));
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 4);
            assert_eq!(s.count("state::set"), 4);
            assert_eq!(s.cursor.as_deref(), Some("paste-4"));
            for (n, input) in s.inputs().iter().enumerate() {
                assert!(input.contains(&format!("PASTE_{}", n + 1)));
            }
        }
        assert_eq!(stack.commits(), 4);
        stack.run(&job("end")).await.unwrap();
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 6);
            assert_eq!(s.count("state::set"), 6);
            assert_eq!(s.cursor.as_deref(), Some("end"));
            assert_eq!(s.requests()[1]["after_entry_id"], "paste-4");
            assert!(s.inputs()[4].contains("PASTE_5"));
            assert!(s.inputs()[5].contains("PASTE_6"));
        }
        assert_eq!(stack.commits(), 6);
        stack.run(&job("end")).await.unwrap();
        assert_eq!(stack.script.lock().unwrap().count("router::complete"), 6);
        assert_eq!(stack.commits(), 6);
        stack.assert_rpc_scope();
    }
    #[tokio::test]
    async fn model_error_retries_the_same_span_then_duplicate_is_a_noop() {
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", "old"),
                message("end", "a durable synthetic line for extraction"),
            ],
            cursor: Some("anchor".into()),
            model_failures: 1,
            model_items: VecDeque::from([fact("Amber instruments require copper calibration.")]),
            ..Script::default()
        })
        .await;
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("provider error"));
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 1);
            assert_eq!(s.count("router::complete"), 1);
            assert_eq!(s.count("state::set"), 0);
            assert_eq!(s.cursor.as_deref(), Some("anchor"));
        }
        assert_eq!(stack.commits(), 0);
        stack.run(&job("end")).await.unwrap();
        stack.run(&job("end")).await.unwrap();
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 2);
            assert_eq!(s.count("router::complete"), 2);
            assert_eq!(s.count("state::set"), 1);
            assert_eq!(s.cursor.as_deref(), Some("end"));
            assert_eq!(s.requests()[0]["after_entry_id"], "anchor");
            assert_eq!(s.requests()[1]["after_entry_id"], "anchor");
        }
        assert_eq!(stack.commits(), 1);
        stack.assert_rpc_scope();
    }

    #[tokio::test]
    async fn malformed_cursor_state_is_not_an_empty_bootstrap() {
        for value in [
            json!({}),
            json!({"last_entry_id":42}),
            json!({"last_entry_id":""}),
        ] {
            let stack = Boundary::new(Script {
                cursor_reply: Some(value),
                cursor: Some("anchor".into()),
                ..Script::default()
            })
            .await;
            assert!(stack
                .run(&job("end"))
                .await
                .unwrap_err()
                .contains("state::get"));
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 0);
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 0);
            assert_eq!(s.cursor.as_deref(), Some("anchor"));
            assert_eq!(stack.commits(), 0);
        }
    }

    #[tokio::test]
    async fn partial_delta_is_discarded_before_a_successful_root_fallback() {
        let pages = VecDeque::from([
            Ok(
                json!({"messages":[message("poison","PARTIAL_POISON_NOT_ON_ROOT")],"next_cursor":"page/1"}),
            ),
            Err(SDK_INVALID_CURSOR),
            Ok(json!({"messages":[message("end","ROOT_DURABLE_FACT_ONLY")],"next_cursor":null})),
        ]);
        let stack = Boundary::new(Script {
            pages,
            cursor: Some("forked-away".into()),
            model_items: VecDeque::from([fact("Root recovery retains only classified envelopes.")]),
            ..Script::default()
        })
        .await;
        stack.run(&job("end")).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 3);
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("state::set"), 1);
        assert_eq!(s.cursor.as_deref(), Some("end"));
        assert!(s.inputs()[0].contains("ROOT_DURABLE_FACT_ONLY"));
        assert!(!s.inputs()[0].contains("PARTIAL_POISON_NOT_ON_ROOT"));
        assert_eq!(stack.commits(), 1);
    }

    #[tokio::test]
    async fn missing_cursor_bootstraps_only_legacy_window_not_all_initial_history() {
        let mut path: Vec<_> = (0..20)
            .map(|n| message(&format!("old-{n}"), "INITIAL_PREFIX_NOT_RECOVERED"))
            .collect();
        path.extend((0..12).map(|n| message(&format!("new-{n}"), "kept window fact")));
        let stack = Boundary::new(Script {
            path,
            model_items: VecDeque::from([fact("Bootstrap keeps its explicit bounded window.")]),
            ..Script::default()
        })
        .await;
        stack.run(&job("new-11")).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 1);
        assert!(s.requests()[0].get("after_entry_id").is_none());
        assert_eq!(s.inputs()[0].matches("kept window fact").count(), 12);
        assert!(!s.inputs()[0].contains("INITIAL_PREFIX_NOT_RECOVERED"));
        assert_eq!(s.cursor.as_deref(), Some("new-11"));
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("state::set"), 1);
        assert_eq!(stack.commits(), 1);
    }

    #[tokio::test]
    async fn failed_cursor_confirmation_after_a_commit_is_not_success() {
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", "old"),
                message("end", "a durable synthetic line for extraction"),
            ],
            cursor: Some("anchor".into()),
            state_write_error: true,
            model_items: VecDeque::from([fact("Persisted facts do not imply a confirmed cursor.")]),
            ..Script::default()
        })
        .await;
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("state::set"));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 1);
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("state::set"), 1);
        assert_eq!(s.cursor.as_deref(), Some("anchor"));
        assert_eq!(stack.commits(), 1);
        // The source receipt, not the independent cursor, prevents replay.
    }

    impl Boundary {
        async fn restart(mut self) -> Self {
            assert!(self.script.lock().unwrap().persistent_cursor);
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.capture_barrier("restart-handoff", None),
            )
            .await
            .expect("bounded SDK capture handoff before restart");
            let mut script = self.script.lock().unwrap().clone();
            script.calls.clear();
            script.returned.clear();
            // The next SDK reads the file, never a copied in-memory watermark.
            script.cursor = None;
            let dir = std::mem::replace(&mut self.dir, tempfile::tempdir().unwrap());
            drop(self);
            Self::new_in(script, dir).await
        }
        fn disk_cursor(&self) -> Value {
            serde_json::from_slice(
                &std::fs::read(self.dir.path().join("synthetic-cursor.json")).unwrap(),
            )
            .unwrap()
        }
        fn log_records(&self, bank: &str) -> Vec<memory::types::Memory> {
            std::fs::read_to_string(self.dir.path().join(bank).join("memories.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(strict_memory_record)
                .collect()
        }
    }

    #[tokio::test]
    async fn absent_state_worker_bootstraps_but_cannot_claim_cursor_confirmation() {
        let stack = Boundary::new(Script {
            path: vec![message(
                "end",
                "Amber instruments require copper calibration.",
            )],
            state_function_absent: true,
            model_items: VecDeque::from([fact("Amber instruments require copper calibration.")]),
            ..Script::default()
        })
        .await;
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("state::set"));
        let s = stack.script.lock().unwrap();
        assert_eq!(
            s.requests(),
            vec![json!({"session_id":"incremental-fixture","limit":100,
            "roles":["user","assistant"],"include_image_data":false})]
        );
        assert_eq!(s.count("state::get"), 1);
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("state::set"), 1);
        assert_eq!(s.cursor, None);
        assert_eq!(stack.commits(), 1);
    }

    #[tokio::test]
    async fn root_boundary_absent_preserves_persistent_cursor_without_any_publication() {
        for bootstrap in [false, true] {
            let stack = Boundary::new(Script {
                path: vec![
                    message("anchor", ""),
                    message("other", "not the completed boundary"),
                ],
                cursor: (!bootstrap).then(|| "anchor".into()),
                persistent_cursor: true,
                ..Script::default()
            })
            .await;
            assert!(stack
                .run(&job("missing-until"))
                .await
                .unwrap_err()
                .contains("boundary pending"));
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), if bootstrap { 1 } else { 2 });
            assert_eq!(s.count("state::get"), 1);
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 0);
            assert_eq!(stack.commits(), 0);
            if bootstrap {
                assert!(!stack.dir.path().join("synthetic-cursor.json").exists());
            } else {
                assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
                assert_eq!(s.requests()[0]["after_entry_id"], "anchor");
                assert!(s.requests()[1].get("after_entry_id").is_none());
            }
        }
    }

    #[tokio::test]
    async fn off_on_multibank_reopen_and_out_of_order_never_sweep_disabled_messages() {
        let amber = "Amber instruments require copper calibration.";
        let birch = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("a", amber),
                message("off", "OFF_SECRET_FACT_NOT_ELIGIBLE"),
                message("b", birch),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            model_items: VecDeque::from([fact(amber), fact(birch)]),
            ..Script::default()
        })
        .await;
        stack.run(&job("a")).await.unwrap();
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"a"}));
        assert_eq!(stack.commits(), 1);
        let mut off = job("off");
        off.skip = true;
        off.bank = None;
        stack.run(&off).await.unwrap();
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 2);
            assert_eq!(s.count("router::complete"), 1);
            assert_eq!(s.count("state::set"), 2);
        }
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"off"}));
        let stack = stack.restart().await;
        let mut on = job("b");
        on.bank = Some("second".into());
        stack.run(&on).await.unwrap();
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(
                s.requests(),
                vec![json!({"session_id":"incremental-fixture","limit":100,
                "roles":["user","assistant"],"include_image_data":false,"after_entry_id":"off"})]
            );
            assert_eq!(s.count("router::complete"), 1);
            assert_eq!(s.count("state::set"), 1);
            assert!(s.inputs()[0].contains(birch));
            assert!(!s.inputs()[0].contains("OFF_SECRET_FACT_NOT_ELIGIBLE"));
            assert!(!s.inputs()[0].contains(amber));
        }
        assert_eq!(stack.log_records("private").len(), 1);
        assert_eq!(stack.log_records("second").len(), 1);
        assert_eq!(stack.log_records("private")[0].text, amber);
        assert_eq!(stack.log_records("second")[0].text, birch);
        assert!(!stack.dir.path().join("public").exists());
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"b"}));
        // Duplicate is a zero-fetch no-op; older job requires delta + root proof.
        stack.run(&on).await.unwrap();
        stack.run(&job("a")).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 3);
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("state::set"), 1);
        assert_eq!(stack.log_records("private").len(), 1);
        assert_eq!(stack.log_records("second").len(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"b"}));
    }

    #[tokio::test]
    async fn transport_failure_then_reopen_retries_exact_delta_without_losing_text() {
        let text = "Cobalt dispatch uses a northern warehouse.";
        let stack = Boundary::new(Script {
            path: vec![message("anchor", ""), message("end", text)],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            pages: VecDeque::from([Err(("invocation_error", "temporary offline fixture error"))]),
            model_items: VecDeque::from([fact(text)]),
            ..Script::default()
        })
        .await;
        assert!(stack.run(&job("end")).await.is_err());
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 1);
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 0);
        }
        assert_eq!(stack.commits(), 0);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
        let stack = stack.restart().await;
        stack.run(&job("end")).await.unwrap();
        stack.run(&job("end")).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 1);
        assert_eq!(s.requests()[0]["after_entry_id"], "anchor");
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("state::set"), 1);
        assert!(s.inputs()[0].contains(text));
        assert_eq!(stack.commits(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
    }

    async fn correction_stack() -> Boundary {
        let old = "Amber instruments require copper calibration.";
        let new = "Amber instruments now require silver calibration.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("old", old),
                message("end", new),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            model_items: VecDeque::from([
                fact(old),
                json!([{"text":new,"entities":["fixture"],
                "scope":"project","kind":"memory","replaces":1}]),
            ]),
            ..Script::default()
        })
        .await;
        stack.run(&job("old")).await.unwrap();
        stack
    }

    #[tokio::test]
    async fn numbered_reference_resolves_actual_record_and_atomic_correction_survives_reopen() {
        let stack = correction_stack().await;
        let old = stack.log_records("private")[0].clone();
        stack.run(&job("end")).await.unwrap();
        {
            let s = stack.script.lock().unwrap();
            assert!(s.inputs()[1].contains(&format!("[1] {}", old.text)));
            assert_eq!(s.count("memory::on-bank-changed"), 1);
            assert_eq!(s.count("memory::on-item-changed"), 3);
            assert_eq!(
                s.publication_trace(),
                vec![
                    ("event".into(), "created".into()),
                    ("cursor".into(), "old".into()),
                    ("event".into(), "created".into()),
                    ("event".into(), "superseded".into()),
                    ("cursor".into(), "end".into()),
                ]
            );
            assert_eq!(s.requests().len(), 2);
            assert_eq!(s.count("router::complete"), 2);
            assert_eq!(s.count("state::set"), 2);
        }
        // One original plus the jointly published replacement/tombstone pair.
        assert_eq!(stack.commits(), 3);
        let stack = stack.restart().await;
        let store = stack.deps.store().await;
        let bank = store.bank("private").await.unwrap();
        let retired = bank.get(&old.id).await.unwrap();
        assert!(!retired.is_live());
        let replacement = bank
            .get(retired.superseded_by.as_deref().unwrap())
            .await
            .unwrap();
        assert!(replacement.is_live());
        assert_eq!(
            replacement.text,
            "Amber instruments now require silver calibration."
        );
        assert_eq!(
            replacement.source.as_ref().unwrap().entry_id.as_deref(),
            Some("end")
        );
        assert_eq!(bank.list(10, 0, false, None).await.0.len(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        stack.run(&job("end")).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 0);
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("state::set"), 0);
        assert_eq!(stack.commits(), 3);
    }

    #[tokio::test]
    async fn a_correction_of_a_record_stored_before_redaction_completes() {
        let stack = correction_stack().await;
        // The record predates a redaction pattern: its text holds a key.
        let key = format!("AKIA{}", "Q".repeat(16));
        let path = stack.dir.path().join("private/memories.jsonl");
        let old = "Amber instruments require copper calibration.";
        let log = std::fs::read_to_string(&path)
            .unwrap()
            .replace(old, &format!("{old} Key {key}."));
        std::fs::write(&path, &log).unwrap();
        let stack = stack.restart().await;
        stack.run(&job("end")).await.unwrap();
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        let after = std::fs::read_to_string(&path).unwrap();
        let written: Vec<_> = after.lines().skip(log.lines().count()).collect();
        assert_eq!(written.len(), 3, "replacement, retired record, receipt");
        assert!(written.iter().all(|line| !line.contains(&key)));
        let records = stack.log_records("private");
        let retired = records.last().unwrap();
        assert!(!retired.is_live() && retired.text.starts_with(old));
    }

    #[tokio::test]
    async fn correction_io_failure_publishes_neither_record_nor_cursor_then_retry_is_atomic() {
        let stack = correction_stack().await;
        let path = stack.dir.path().join("private/memories.jsonl");
        let before = std::fs::read(&path).unwrap();
        let old = stack.log_records("private")[0].clone();
        let read_only = |on: bool| {
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(on);
            std::fs::set_permissions(&path, permissions).unwrap();
        };
        read_only(true);
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("memory commit failed"));
        read_only(false);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let bank = stack.deps.store().await.bank("private").await.unwrap();
        assert_eq!(bank.get(&old.id).await.unwrap(), old);
        assert_eq!(bank.list(10, 0, false, None).await.0.len(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"old"}));
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 2);
            assert_eq!(s.count("router::complete"), 2);
            assert_eq!(s.count("state::set"), 1);
            assert_eq!(s.count("memory::on-item-changed"), 1);
            assert!(s.inputs()[1].contains(&format!("[1] {}", old.text)));
        }
        stack.script.lock().unwrap().model_items.push_back(json!([{
            "text":"Amber instruments now require silver calibration.","entities":["fixture"],
            "scope":"project","kind":"memory","replaces":1}]));
        let stack = stack.restart().await;
        stack.run(&job("end")).await.unwrap();
        assert_eq!(stack.commits(), 3);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 1);
        assert_eq!(s.requests()[0]["after_entry_id"], "old");
        assert_eq!(
            s.count("router::complete"),
            0,
            "durable original decision reused"
        );
        assert_eq!(s.count("state::set"), 1);
    }

    #[tokio::test]
    async fn correction_snapshot_conflict_skips_only_stale_effect_and_survives_cursor_failure() {
        for mutation in ["pin", "edit", "reinforce"] {
            let stack = correction_stack().await;
            let old = stack.log_records("private")[0].clone();
            let second = "Birch archives retain monthly invoices.";
            {
                let mut script = stack.script.lock().unwrap();
                script.model_items[0]
                    .as_array_mut()
                    .unwrap()
                    .push(fact(second)[0].clone());
                script.state_write_error = true;
            }
            let pause = Arc::new((
                tokio::sync::Semaphore::new(0),
                tokio::sync::Semaphore::new(0),
            ));
            stack.script.lock().unwrap().model_pause = Some(pause.clone());
            let input = job("end");
            let work = stack.run(&input);
            let mutate = async {
                tokio::time::timeout(std::time::Duration::from_secs(5), pause.0.acquire())
                    .await
                    .unwrap()
                    .unwrap()
                    .forget();
                // Another session writes the record after the extraction read it.
                let bank = stack.deps.store().await.bank("private").await.unwrap();
                let mut changed = bank.get(&old.id).await.unwrap();
                changed.revision += 1;
                match mutation {
                    "pin" => changed.pinned = true,
                    "edit" => changed.text = "Human calibration edit remains authoritative.".into(),
                    "reinforce" => {
                        changed.corroboration += 1;
                        changed.source.as_mut().unwrap().session_id = Some("other-session".into());
                    }
                    _ => unreachable!(),
                }
                bank.commit_expected(changed.clone(), Some(&old))
                    .await
                    .unwrap()
                    .unwrap();
                pause.1.add_permits(1);
                changed
            };
            let (result, changed) = tokio::join!(work, mutate);
            assert!(result.unwrap_err().contains("state::set"));
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"old"}));
            let source =
                json!(["incremental-fixture", "private", false, null, null, ["end"]]).to_string();
            let receipt = json!([source, "private", 0]).to_string();
            let decision = stack
                .deps
                .store()
                .await
                .extraction_plan(&source)
                .unwrap()
                .unwrap();
            assert_eq!(decision["known"][0], serde_json::to_value(&old).unwrap());
            assert_eq!(decision["items"].as_array().unwrap().len(), 2);
            let bank = stack.deps.store().await.bank("private").await.unwrap();
            assert!(bank.extraction_seen(&receipt).await);
            let records = stack.log_records("private");
            assert_eq!(records.len(), 3, "seed, independent change, other effect");
            assert_eq!(records[1], changed);
            assert_eq!(records[2].text, second);
            assert_eq!(records[2].corroboration, 0);
            assert_eq!(
                stack
                    .script
                    .lock()
                    .unwrap()
                    .count("memory::on-item-changed"),
                2
            );
            {
                let mut script = stack.script.lock().unwrap();
                script.state_write_error = false;
                script.model_pause = None;
            }
            let stack = stack.restart().await;
            stack.run(&input).await.unwrap();
            stack.run(&input).await.unwrap();
            let bank = stack.deps.store().await.bank("private").await.unwrap();
            assert_eq!(bank.get(&old.id).await.unwrap(), changed);
            assert_eq!(stack.log_records("private"), records);
            // Past the cursor the run never replays: its decision is gone.
            assert_eq!(
                stack.deps.store().await.extraction_plan(&source).unwrap(),
                None
            );
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
            assert!(stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap()
                .is_none());
            let script = stack.script.lock().unwrap();
            assert_eq!(script.count("router::complete"), 0);
            assert_eq!(script.count("memory::on-item-changed"), 0);
            assert_eq!(script.count("state::set"), 1);
        }
    }

    #[tokio::test]
    async fn cursor_failure_reopen_recovers_committed_receipt_without_reinforcement() {
        let text = "Amber instruments require copper calibration.";
        let stack = Boundary::new(Script {
            path: vec![message("anchor", ""), message("end", text)],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            state_write_error: true,
            observe_events: true,
            model_items: VecDeque::from([fact(text)]),
            ..Script::default()
        })
        .await;
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("state::set"));
        let original = stack.log_records("private")[0].clone();
        assert_eq!(original.corroboration, 0);
        assert_eq!(
            stack
                .script
                .lock()
                .unwrap()
                .count("memory::on-item-changed"),
            1
        );
        assert_eq!(stack.commits(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
        {
            let mut s = stack.script.lock().unwrap();
            assert_eq!(s.requests().len(), 1);
            assert_eq!(s.count("router::complete"), 1);
            assert_eq!(s.count("state::set"), 1);
            s.state_write_error = false;
        }
        let stack = stack.restart().await;
        stack.run(&job("end")).await.unwrap();
        stack.run(&job("end")).await.unwrap();
        let bank = stack.deps.store().await.bank("private").await.unwrap();
        let records = bank.list(10, 0, false, None).await.0;
        assert_eq!(records.len(), 1, "fingerprint identity survives retry");
        assert_eq!(records[0].id, original.id);
        assert_eq!(
            records[0].corroboration, 0,
            "durable receipt prevents a second effect"
        );
        assert_eq!(records[0].revision, original.revision);
        assert_eq!(
            stack.script.lock().unwrap().publication_trace(),
            vec![("cursor".into(), "end".into())]
        );
        assert_eq!(stack.commits(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.requests().len(), 1);
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("memory::on-item-changed"), 0);
        assert_eq!(s.count("state::set"), 1);
    }

    #[tokio::test]
    async fn durable_reinforcement_receipt_and_correction_survive_cursor_failure_reopen() {
        for correction in [false, true] {
            let stack = correction_stack().await;
            let original = stack.log_records("private")[0].clone();
            if !correction {
                let mut s = stack.script.lock().unwrap();
                s.path[2] = message("end", &original.text);
                s.model_items = VecDeque::from([fact(&original.text)]);
            }
            stack.script.lock().unwrap().state_write_error = true;
            assert!(stack
                .run(&job("end"))
                .await
                .unwrap_err()
                .contains("state::set"));
            let committed = stack.log_records("private");
            assert_eq!(committed.len(), if correction { 3 } else { 2 });
            if !correction {
                assert_eq!(committed[1].corroboration, original.corroboration + 1);
                assert_eq!(committed[1].revision, original.revision + 1);
            } else {
                assert!(!committed[2].is_live());
                assert_eq!(committed[2].revision, original.revision + 1);
            }
            stack.script.lock().unwrap().state_write_error = false;
            let stack = stack.restart().await;
            stack.run(&job("end")).await.unwrap();
            stack.run(&job("end")).await.unwrap();
            assert_eq!(stack.log_records("private"), committed);
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("memory::on-item-changed"), 0);
            assert_eq!(s.count("state::set"), 1);
        }
    }

    fn stamped(id: &str, turn: &str, bank: &str, enabled: bool) -> Value {
        let mut item = message(id, "");
        item["message"]["role"] = json!("assistant");
        item["origin"] = json!({"turn_id":turn,"memory_bank":bank,"memory_extraction":enabled});
        item
    }

    #[tokio::test]
    async fn pending_off_overtaken_by_on_and_bank_backlog_is_resolved_before_model_after_restart() {
        let private = "Amber instruments require copper calibration.";
        let other = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("off-user", "OFF_SECRET_MUST_NOT_REACH_MODEL"),
                stamped("off-end", "off", "private", false),
                message("on-user", private),
                stamped("on-end", "on", "private", true),
                message("bank-user", other),
                stamped("end", "bank", "other", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            model_items: VecDeque::from([fact(private), fact(other)]),
            ..Script::default()
        })
        .await;
        // A pending OFF callback never ran; simulate worker/store reconstruction
        // before the latest ON job arrives first.
        let stack = stack.restart().await;
        let mut latest = job("end");
        latest.turn_id = Some("bank".into());
        latest.bank = Some("other".into());
        stack.run(&latest).await.unwrap();
        let mut off = job("off-end");
        off.turn_id = Some("off".into());
        off.skip = true;
        stack.run(&off).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 2);
        assert_eq!(s.count("state::set"), 3);
        assert_eq!(s.inputs().len(), 2);
        assert!(s.inputs().iter().all(|input| !input.contains("OFF_SECRET")));
        assert!(s.inputs()[0].contains(private));
        assert!(!s.inputs()[0].contains(other));
        assert!(s.inputs()[1].contains(other));
        assert!(!s.inputs()[1].contains(private));
        drop(s);
        assert_eq!(stack.log_records("private")[0].text, private);
        assert_eq!(stack.log_records("other")[0].text, other);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
    }

    #[tokio::test]
    async fn newer_job_recovers_pending_effect_before_widening_delta_after_reopen() {
        let first = "Amber instruments require copper calibration.";
        let second = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("first", first),
                stamped("first-end", "first-turn", "private", true),
                message("second-request", second),
                stamped("end", "second-turn", "private", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            state_write_error: true,
            model_items: VecDeque::from([fact(first), fact(second)]),
            ..Script::default()
        })
        .await;
        let mut earlier = job("first-end");
        earlier.turn_id = Some("first-turn".into());
        assert!(stack
            .run(&earlier)
            .await
            .unwrap_err()
            .contains("state::set"));
        let original = stack.log_records("private")[0].clone();
        stack.script.lock().unwrap().state_write_error = false;
        let stack = stack.restart().await;
        let mut later = job("end");
        later.turn_id = Some("second-turn".into());
        stack.run(&later).await.unwrap();
        let records = stack.log_records("private");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], original);
        assert_eq!(records[1].text, second);
        assert_eq!(records[1].revision, 0);
        assert_eq!(records[1].corroboration, 0);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("memory::on-item-changed"), 1);
        assert_eq!(s.count("state::set"), 2);
    }

    #[tokio::test]
    async fn a_long_message_is_read_from_its_head_and_tail() {
        let head = "Amber instruments require copper calibration.";
        let tail = "Birch archives retain monthly invoices.";
        let long = format!("{head} {} {tail}", "filler ".repeat(150_000));
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("large", &long),
                stamped("large-end", "large-turn", "private", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            model_items: VecDeque::from([fact(head)]),
            ..Script::default()
        })
        .await;
        let mut owner = job("large-end");
        owner.turn_id = Some("large-turn".into());
        stack.run(&owner).await.unwrap();
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 1);
            let input = &s.inputs()[0];
            assert!(input.contains(head) && input.contains(tail));
            assert!(input.len() < 16_000, "{} bytes", input.len());
        }
        // A redelivered job asks the model nothing.
        let stack = stack.restart().await;
        stack.run(&owner).await.unwrap();
        assert_eq!(stack.script.lock().unwrap().count("router::complete"), 0);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"large-end"}));
        assert_eq!(stack.log_records("private").len(), 1);
        let store = stack.deps.store().await;
        assert!(store
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
        assert!(store
            .pending_extraction("incremental-fixture")
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn a_long_message_never_stops_the_turns_after_it() {
        let long = "Amber instruments require copper calibration.";
        let mut path = vec![
            message("anchor", ""),
            message("large", &format!("{long} {}", "a".repeat(97 * 1024))),
            stamped("large-end", "large-turn", "private", true),
        ];
        let texts: Vec<String> = (0..130)
            .map(|i| format!("Synthetic fixture fact number {i} holds."))
            .collect();
        for (i, text) in texts.iter().enumerate() {
            path.push(message(&format!("user-{i}"), text));
            path.push(stamped(
                &format!("end-{i}"),
                &format!("turn-{i}"),
                "private",
                true,
            ));
        }
        let stack = Boundary::new(Script {
            path,
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            model_items: std::iter::once(long)
                .chain(texts.iter().map(String::as_str))
                .map(fact)
                .collect(),
            ..Script::default()
        })
        .await;
        let mut owner = job("large-end");
        owner.turn_id = Some("large-turn".into());
        stack.run(&owner).await.unwrap();
        for i in 0..130 {
            let mut turn = job(&format!("end-{i}"));
            turn.turn_id = Some(format!("turn-{i}"));
            stack.run(&turn).await.unwrap();
        }
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end-129"}));
        let records = stack.log_records("private");
        assert_eq!(records.len(), 131);
        assert_eq!(records[0].text, long);
        assert_eq!(stack.script.lock().unwrap().count("router::complete"), 131);
        // Every range was acknowledged: the ledger is empty.
        assert!(stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_dead_lettered_or_edited_run_never_stops_the_turns_after_it() {
        for edited in [false, true] {
            let mut path = vec![
                message("anchor", ""),
                message(
                    "first-user",
                    "Amber instruments require copper calibration.",
                ),
                stamped("first-end", "first-turn", "private", true),
            ];
            let texts: Vec<String> = (0..130)
                .map(|i| format!("Synthetic fixture fact number {i} holds."))
                .collect();
            for (i, text) in texts.iter().enumerate() {
                path.push(message(&format!("user-{i}"), text));
                path.push(stamped(
                    &format!("end-{i}"),
                    &format!("turn-{i}"),
                    "private",
                    true,
                ));
            }
            let mut items: VecDeque<Value> = texts.iter().map(|text| fact(text)).collect();
            items.push_front(json!([]));
            let stack = Boundary::new(Script {
                path,
                cursor: Some("anchor".into()),
                persistent_cursor: true,
                model_failures: 3,
                model_items: items,
                ..Script::default()
            })
            .await;
            let mut owner = job("first-end");
            owner.turn_id = Some("first-turn".into());
            // A provider outage outlasts the owner's retries: dead-lettered.
            for _ in 0..3 {
                assert!(stack.run(&owner).await.is_err());
            }
            if edited {
                stack.script.lock().unwrap().path[1] =
                    message("first-user", "Edited: amber instruments need copper.");
            }
            for i in 0..130 {
                let mut turn = job(&format!("end-{i}"));
                turn.turn_id = Some(format!("turn-{i}"));
                stack.run(&turn).await.unwrap();
                let ranges = stack
                    .deps
                    .store()
                    .await
                    .extraction_ranges("incremental-fixture")
                    .unwrap();
                assert!(ranges.is_empty(), "turn {i}: {ranges:?}");
            }
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end-129"}));
            assert_eq!(stack.log_records("private").len(), 130);
            // Three failures, the dead-lettered run once, then every turn.
            assert_eq!(stack.script.lock().unwrap().count("router::complete"), 134);
            assert!(stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap()
                .is_none());
        }
    }

    #[tokio::test]
    async fn public_fetch_identifier_bounds_preserve_recovery_and_allow_attested_retry() {
        for root_recovery in [false, true] {
            let first = "Amber instruments require copper calibration.";
            let second = "Birch archives retain monthly invoices.";
            let path = vec![
                message("anchor", ""),
                message("first", first),
                stamped("first-end", "first-turn", "private", true),
                message("second-request", second),
                stamped("end", "second-turn", "private", true),
            ];
            let stack = Boundary::new(Script {
                path: path.clone(),
                cursor: Some("anchor".into()),
                persistent_cursor: true,
                observe_events: true,
                state_write_error: true,
                model_items: VecDeque::from([fact(first), fact(second)]),
                ..Script::default()
            })
            .await;
            let mut earlier = job("first-end");
            earlier.turn_id = Some("first-turn".into());
            assert!(stack
                .run(&earlier)
                .await
                .unwrap_err()
                .contains("state::set"));
            stack.script.lock().unwrap().state_write_error = false;
            let stack = stack.restart().await;
            let store = stack.deps.store().await;
            let pending = store.pending_extraction("incremental-fixture").unwrap();
            let ranges = store.extraction_ranges("incremental-fixture").unwrap();
            let original = stack.log_records("private");
            let snapshot = || {
                let mut files = std::fs::read_dir(stack.dir.path().join(".extraction"))
                    .unwrap()
                    .map(|entry| {
                        let p = entry.unwrap().path();
                        (p.clone(), std::fs::read(p).unwrap())
                    })
                    .collect::<Vec<_>>();
                files.sort();
                files
            };
            let before = snapshot();
            // Identifiers past their bound fail the read before any effect;
            // sizes never do (see large_sources_never_stop_extraction).
            let mut cases = Vec::<Vec<Value>>::new();
            cases.push(vec![
                json!({"messages":[message(&"i".repeat(257), "safe")]}),
            ]);
            cases.push(vec![
                json!({"messages":[message("end", "safe")], "next_cursor":"c".repeat(257)}),
            ]);
            let mut origin = message("end", "safe");
            origin["origin"] = json!({"turn_id":"t".repeat(257)});
            cases.push(vec![json!({"messages":[origin]})]);
            for pages in cases {
                {
                    let mut script = stack.script.lock().unwrap();
                    script.pages.clear();
                    if root_recovery {
                        script.pages.push_back(Err(SDK_INVALID_CURSOR));
                    }
                    script.pages.extend(pages.into_iter().map(Ok));
                }
                assert!(stack.run(&job("end")).await.unwrap_err().contains("bound"));
                assert_eq!(snapshot(), before);
                assert_eq!(
                    store.pending_extraction("incremental-fixture").unwrap(),
                    pending
                );
                assert_eq!(
                    store.extraction_ranges("incremental-fixture").unwrap(),
                    ranges
                );
                assert_eq!(stack.log_records("private"), original);
                assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
                let script = stack.script.lock().unwrap();
                for function in [
                    "router::complete",
                    "state::set",
                    "memory::on-item-changed",
                    "memory::on-bank-changed",
                ] {
                    assert_eq!(script.count(function), 0);
                }
            }
            {
                let mut script = stack.script.lock().unwrap();
                script.pages.clear();
                if root_recovery {
                    script.pages.push_back(Err(SDK_INVALID_CURSOR));
                }
                script.path = path;
            }
            let mut later = job("end");
            later.turn_id = Some("second-turn".into());
            stack.run(&later).await.unwrap();
            assert_eq!(stack.log_records("private").len(), 2);
            assert_eq!(stack.log_records("private")[0], original[0]);
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
            assert_eq!(stack.script.lock().unwrap().count("router::complete"), 1);
        }
    }

    #[tokio::test]
    async fn large_sources_never_stop_extraction() {
        // A tool call of 1.2 MB in each reply (a delta over 4 MiB) and a
        // request over 1 MiB of text: every run is read, the long request
        // from its head and tail, and the cursor reaches the last turn.
        let reply = |id: &str, turn: &str| {
            let mut item = stamped(id, turn, "private", true);
            item["message"]["content"] = json!([
                {"type":"text","text":"Done."},
                {"type":"function_call","id":"call","name":"write","arguments":"x".repeat(1_200_000)}
            ]);
            item
        };
        let first = "Amber instruments require copper calibration.";
        let last = "Birch archives retain monthly invoices.";
        for root in [false, true] {
            let stack = Boundary::new(Script {
                path: vec![
                    message("anchor", ""),
                    message("u0", first),
                    reply("a0", "t0"),
                    message("u1", &"y".repeat(1_100_000)),
                    reply("a1", "t1"),
                    message("u2", last),
                    reply("a2", "t2"),
                    reply("a3", "t2"),
                ],
                cursor: Some("anchor".into()),
                model_items: VecDeque::from([fact(first), json!([]), fact(last)]),
                ..Script::default()
            })
            .await;
            if root {
                stack
                    .script
                    .lock()
                    .unwrap()
                    .pages
                    .push_back(Err(SDK_INVALID_CURSOR));
            }
            let mut input = job("a3");
            input.turn_id = Some("t2".into());
            stack.run(&input).await.unwrap();
            assert_eq!(
                stack.script.lock().unwrap().cursor.as_deref(),
                Some("a3"),
                "root {root}"
            );
            assert_eq!(stack.script.lock().unwrap().count("router::complete"), 3);
            assert!(stack.script.lock().unwrap().inputs()[1].len() < 16_000);
            assert!(stack
                .deps
                .store()
                .await
                .extraction_ranges("incremental-fixture")
                .unwrap()
                .is_empty());
            let texts: Vec<_> = stack
                .log_records("private")
                .into_iter()
                .map(|r| r.text)
                .collect();
            assert_eq!(texts, [first, last]);
        }
    }

    #[tokio::test]
    async fn public_source_boundaries_accept_full_text_and_reject_oversized_job_before_rpc() {
        const TEXT: usize = 96 * 1024;
        for root in [false, true] {
            let id = "i".repeat(256);
            let stack = Boundary::new(Script {
                path: vec![message("anchor", ""), message(&id, &"x".repeat(TEXT))],
                cursor: (!root).then(|| "anchor".into()),
                model_items: VecDeque::from([json!([])]),
                ..Script::default()
            })
            .await;
            stack.run(&job(&id)).await.unwrap();
            assert_eq!(stack.script.lock().unwrap().count("router::complete"), 1);
            assert_eq!(
                stack.script.lock().unwrap().cursor.as_deref(),
                Some(id.as_str())
            );
        }
        let stack = Boundary::new(Script::default()).await;
        for field in ["session", "until", "turn", "bank", "agent"] {
            let mut invalid = job("end");
            let huge = "x".repeat(257);
            match field {
                "session" => invalid.session_id = huge,
                "until" => invalid.until = Some(huge),
                "turn" => invalid.turn_id = Some(huge),
                "bank" => invalid.bank = Some(huge),
                _ => invalid.agent = Some(huge),
            }
            assert!(memory::extract::try_run(&stack.deps, &invalid)
                .await
                .unwrap_err()
                .contains("byte bound"));
        }
        assert!(stack.script.lock().unwrap().calls.is_empty());
        stack.script.lock().unwrap().cursor_reply = Some(json!({"last_entry_id":"c".repeat(257)}));
        assert!(stack.run(&job("end")).await.unwrap_err().contains("cursor"));
        assert_eq!(stack.script.lock().unwrap().count("session::messages"), 0);
        assert_eq!(stack.script.lock().unwrap().count("state::set"), 0);
    }

    #[tokio::test]
    async fn learned_rule_receipt_survives_cursor_failure_reopen_and_later_human_edit() {
        let text = "Always calibrate Amber instruments with copper before shipping.";
        let stack = Boundary::new(Script {
            path: vec![message("anchor", ""), message("end", text)],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            state_write_error: true,
            model_items: VecDeque::from([json!([{"text":text,"scope":"project","kind":"rule"}])]),
            ..Script::default()
        })
        .await;
        let mut cfg = (*stack.deps.config().await).clone();
        cfg.rule_learning_enabled = true;
        *stack.deps.config.write().await = Arc::new(cfg);
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("state::set"));
        let bank = stack.deps.store().await.bank("private").await.unwrap();
        let learned = bank.list_rules().unwrap();
        assert_eq!(learned.len(), 1);
        assert!(learned[0].content.contains(text));
        assert!(!learned[0].content.contains("extraction-receipts"));
        assert_eq!(
            stack
                .script
                .lock()
                .unwrap()
                .count("memory::on-bank-changed"),
            2
        );
        assert_eq!(
            stack
                .script
                .lock()
                .unwrap()
                .count("memory::on-item-changed"),
            0
        );
        // A human removes/replaces the learned text after publication, before ack.
        bank.set_rule(
            "learned",
            "Human-authored replacement remains authoritative.",
        )
        .unwrap();
        stack.script.lock().unwrap().state_write_error = false;
        let stack = stack.restart().await;
        let mut cfg = (*stack.deps.config().await).clone();
        cfg.rule_learning_enabled = true;
        *stack.deps.config.write().await = Arc::new(cfg);
        stack.run(&job("end")).await.unwrap();
        stack.run(&job("end")).await.unwrap();
        let bank = stack.deps.store().await.bank("private").await.unwrap();
        assert_eq!(
            bank.list_rules().unwrap()[0].content,
            "Human-authored replacement remains authoritative."
        );
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("memory::on-bank-changed"), 0);
        assert_eq!(s.count("memory::on-item-changed"), 0);
        assert_eq!(s.count("state::set"), 1);
    }

    #[tokio::test]
    async fn learned_rule_io_failure_reopens_original_decision_and_emits_only_after_publication() {
        let text = "Always calibrate Amber instruments with copper before shipping.";
        let stack = Boundary::new(Script {
            path: vec![message("anchor", ""), message("end", text)],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            model_items: VecDeque::from([json!([{"text":text,"scope":"project","kind":"rule"}])]),
            ..Script::default()
        })
        .await;
        let mut cfg = (*stack.deps.config().await).clone();
        cfg.rule_learning_enabled = true;
        *stack.deps.config.write().await = Arc::new(cfg);
        let (bank, _) = stack
            .deps
            .store()
            .await
            .ensure_bank("private", None)
            .await
            .unwrap();
        bank.set_rule("learned", "- Existing human instruction.\n")
            .unwrap();
        let path = bank.dir().join("rules/learned.md");
        let before = std::fs::read(&path).unwrap();
        let blocker = bank.dir().join("rules/.learned.md.tmp");
        std::fs::create_dir(&blocker).unwrap();
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("learned rule write failed"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
        assert_eq!(
            stack
                .script
                .lock()
                .unwrap()
                .count("memory::on-bank-changed"),
            0
        );
        assert_eq!(stack.script.lock().unwrap().count("state::set"), 0);
        assert_eq!(stack.script.lock().unwrap().count("router::complete"), 1);
        std::fs::rename(blocker, bank.dir().join("rules/synthetic-blocker-backup")).unwrap();
        let stack = stack.restart().await;
        let mut cfg = (*stack.deps.config().await).clone();
        cfg.rule_learning_enabled = true;
        *stack.deps.config.write().await = Arc::new(cfg);
        stack.run(&job("end")).await.unwrap();
        stack.run(&job("end")).await.unwrap();
        let bank = stack.deps.store().await.bank("private").await.unwrap();
        let content = &bank.list_rules().unwrap()[0].content;
        assert!(content.contains("Existing human instruction."));
        assert_eq!(content.matches(text).count(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("memory::on-bank-changed"), 1);
        assert_eq!(s.count("memory::on-item-changed"), 0);
        assert_eq!(s.count("state::set"), 1);
    }

    #[tokio::test]
    async fn pending_recovery_rejects_changed_source_text_without_ack_or_repeat() {
        let text = "Amber instruments require copper calibration.";
        let stack = Boundary::new(Script {
            path: vec![message("anchor", ""), message("end", text)],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            state_write_error: true,
            model_items: VecDeque::from([fact(text)]),
            ..Script::default()
        })
        .await;
        assert!(stack.run(&job("end")).await.is_err());
        let original = stack.log_records("private");
        {
            let mut s = stack.script.lock().unwrap();
            s.state_write_error = false;
            s.path[1] = message(
                "end",
                "Different text with the same identifier must fail closed.",
            );
        }
        let stack = stack.restart().await;
        assert!(stack
            .run(&job("end"))
            .await
            .unwrap_err()
            .contains("source no longer on path"));
        assert_eq!(stack.log_records("private"), original);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("state::set"), 0);
    }

    #[tokio::test]
    async fn causal_backlog_retains_each_turn_across_four_run_limit_and_restart() {
        let mut path = vec![message("anchor", "")];
        let mut decisions = VecDeque::new();
        for i in 0..6 {
            let text =
                format!("Synthetic instrument {i} requires copper calibration before shipping.");
            path.push(message(&format!("user-{i}"), &text));
            path.push(stamped(
                &format!("end-{i}"),
                &format!("turn-{i}"),
                "private",
                true,
            ));
            decisions.push_back(fact(&text));
        }
        let stack = Boundary::new(Script {
            path,
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            model_items: decisions,
            ..Script::default()
        })
        .await;
        let mut latest = job("end-5");
        latest.turn_id = Some("turn-5".into());
        assert!(stack
            .run(&latest)
            .await
            .unwrap_err()
            .contains("remaining runs require retry"));
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end-3"}));
        assert_eq!(stack.script.lock().unwrap().count("router::complete"), 4);
        let stack = stack.restart().await;
        stack.run(&latest).await.unwrap();
        stack.run(&latest).await.unwrap();
        let records = stack.log_records("private");
        assert_eq!(records.len(), 6);
        assert!(records
            .iter()
            .all(|m| m.corroboration == 0 && m.revision == 0));
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end-5"}));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 2);
        assert_eq!(s.count("state::set"), 2);
        let inputs = s.inputs();
        let first_delta = inputs[0].split("Conversation excerpt:\n").last().unwrap();
        let second_delta = inputs[1].split("Conversation excerpt:\n").last().unwrap();
        assert!(!first_delta.contains("instrument 0"));
        assert!(first_delta.contains("instrument 4"));
        assert!(second_delta.contains("instrument 5"));
    }

    #[tokio::test]
    async fn mismatched_job_turn_cannot_publish_another_boundary() {
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("user", "Amber instruments require copper calibration."),
                stamped("end", "actual-turn", "private", true),
            ],
            cursor: Some("anchor".into()),
            ..Script::default()
        })
        .await;
        let mut input = job("end");
        input.turn_id = Some("different-turn".into());
        assert!(stack
            .run(&input)
            .await
            .unwrap_err()
            .contains("job/source turn mismatch"));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("state::set"), 0);
        assert_eq!(stack.commits(), 0);
    }

    #[tokio::test]
    async fn missing_source_policy_never_sweeps_turn_bound_backlog() {
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("end", "unclassified backlog must not leak"),
            ],
            cursor: Some("anchor".into()),
            ..Script::default()
        })
        .await;
        let mut input = job("end");
        input.turn_id = Some("new-turn".into());
        // A request with no reply yet waits for the pass that reads its end.
        stack.run(&input).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("state::set"), 0);
        assert_eq!(stack.commits(), 0);
    }

    #[tokio::test]
    async fn unknown_policy_and_unattributed_entries_pass_unextracted_after_restart() {
        // The second element: whether the turn's own request reaches the model.
        for (position, asked) in [
            ("prefix", true),
            ("gap", false),
            ("suffix", true),
            ("requests", true),
            ("turn", false),
            ("conflict", false),
        ] {
            let text = "Amber instruments require copper calibration.";
            let answered = "The turn answered this later request.";
            let unknown = || {
                let mut item = message("unknown", "OFF_UNKNOWN_MUST_NOT_REACH_MODEL");
                item["message"]["role"] = json!("assistant");
                item["origin"] = json!({"turn_id":"unconfirmed-off"});
                item
            };
            let mut path = vec![message("anchor", "")];
            if position == "prefix" {
                path.push(unknown());
            }
            path.push(message("user", text));
            if position == "gap" {
                path.push(unknown());
            }
            if position == "requests" {
                path.push(message("unbound", answered));
            }
            if position == "turn" {
                path[1]["origin"] = json!({"turn_id":"unconfirmed-off"});
            }
            if position == "conflict" {
                path.push(stamped("earlier", "on", "private", false));
            }
            path.push(stamped("end", "on", "private", true));
            if position == "suffix" {
                path.push(unknown());
            }
            let stack = Boundary::new(Script {
                path,
                cursor: Some("anchor".into()),
                persistent_cursor: true,
                observe_events: true,
                model_items: VecDeque::from([json!([])]),
                ..Script::default()
            })
            .await
            .restart()
            .await;
            let until = if position == "suffix" {
                "unknown"
            } else {
                "end"
            };
            let mut input = job(until);
            input.turn_id = Some("on".into());
            stack.run(&input).await.unwrap();
            assert_eq!(
                stack.disk_cursor(),
                json!({"last_entry_id":until}),
                "{position}"
            );
            assert_eq!(stack.commits(), 0);
            assert!(stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap()
                .is_none());
            let s = stack.script.lock().unwrap();
            assert_eq!(
                s.count("router::complete"),
                usize::from(asked),
                "{position}"
            );
            assert!(s.inputs().iter().all(|i| !i.contains("OFF_UNKNOWN")));
            if position == "requests" {
                // The earlier request failed before a reply: never this turn's.
                assert!(s.inputs()[0].contains(answered));
                assert!(!s.inputs()[0].contains(text));
            } else if asked {
                assert!(s.inputs()[0].contains(text));
            }
        }
    }

    #[tokio::test]
    async fn a_missed_hook_on_the_jobs_own_turn_still_extracts_its_request() {
        // The pre-generate hook timed out, or its bank lookup failed, on the
        // turn's only step: no stamp. The job resolves the bank itself, and a
        // failed lookup retries instead of passing the request unextracted.
        let text = "Amber instruments require copper calibration.";
        let mut end = stamped("end", "on", "private", true);
        end["origin"] = json!({"turn_id":"on"});
        let stack = Boundary::new(Script {
            path: vec![message("anchor", ""), message("user", text), end],
            cursor: Some("anchor".into()),
            session_error: true,
            model_items: VecDeque::from([fact(text)]),
            ..Script::default()
        })
        .await;
        let mut input = job("end");
        input.turn_id = Some("on".into());
        input.bank = None;
        assert!(stack.run(&input).await.is_err());
        assert_eq!(stack.script.lock().unwrap().count("router::complete"), 0);
        assert_eq!(
            stack.script.lock().unwrap().cursor.as_deref(),
            Some("anchor")
        );
        stack.script.lock().unwrap().session_error = false;
        stack.run(&input).await.unwrap();
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 1);
        assert!(s.inputs()[0].contains(text));
        assert_eq!(s.cursor.as_deref(), Some("end"));
        drop(s);
        assert_eq!(stack.log_records("private")[0].text, text);
    }

    #[tokio::test]
    async fn failed_turn_without_assistant_cannot_inherit_later_turn_policy_or_bank() {
        // Direct send uses opaque e_ IDs, not just queued e_q_ identities.
        // Neither session metadata nor the next assistant proves the first
        // request's turn policy: it passes unextracted, never into this turn.
        let later = "Later request belongs to the turn that answered it.";
        for (bank, enabled) in [("private", true), ("other", true), ("private", false)] {
            let stack = Boundary::new(Script {
                path: vec![
                    message("anchor", ""),
                    message(
                        "e_earlier_direct",
                        "EARLIER_FAILED_REQUEST requires its own extraction policy.",
                    ),
                    message("e_later_direct", later),
                    stamped("end", "later", bank, enabled),
                ],
                cursor: Some("anchor".into()),
                persistent_cursor: true,
                observe_events: true,
                model_items: VecDeque::from([fact(later)]),
                ..Script::default()
            })
            .await
            .restart()
            .await;
            let original_path = stack.script.lock().unwrap().path.clone();
            let mut input = job("end");
            input.turn_id = Some("later".into());
            input.bank = Some(bank.into());
            input.skip = !enabled;
            stack.run(&input).await.unwrap();
            let stack = stack.restart().await;
            stack.run(&input).await.unwrap();
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
            let records = stack.log_records(bank);
            assert_eq!(records.len(), usize::from(enabled));
            assert!(records.iter().all(|r| r.text == later));
            assert!(stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap()
                .is_none());
            let script = stack.script.lock().unwrap();
            assert_eq!(script.path, original_path);
            // The restart cleared the first run's calls; the rerun is a no-op.
            assert_eq!(script.count("router::complete"), 0);
            assert_eq!(script.count("state::set"), 0);
        }
    }

    #[tokio::test]
    async fn pending_is_not_authority_for_automation_skip_wrong_bank_or_wrong_turn() {
        for published in [false, true] {
            for denied in ["automation", "skip", "bank", "turn", "policy"] {
                let text = "Amber instruments require copper calibration.";
                let stack = Boundary::new(Script {
                    path: vec![
                        message("anchor", ""),
                        message("user", text),
                        stamped("end", "on", "private", true),
                    ],
                    cursor: Some("anchor".into()),
                    persistent_cursor: true,
                    observe_events: true,
                    state_write_error: published,
                    model_failures: usize::from(!published),
                    model_items: VecDeque::from([fact(text)]),
                    ..Script::default()
                })
                .await;
                let mut input = job("end");
                input.turn_id = Some("on".into());
                assert!(stack.run(&input).await.is_err());
                let records = stack.log_records("private");
                assert_eq!(records.len(), usize::from(published));
                let pending = stack
                    .deps
                    .store()
                    .await
                    .pending_extraction("incremental-fixture")
                    .unwrap();
                assert!(pending.is_some());
                {
                    let mut s = stack.script.lock().unwrap();
                    s.state_write_error = false;
                    if denied == "automation" {
                        s.kind = Some("automation".into());
                    }
                    if denied == "policy" {
                        s.path[2]["origin"]["memory_extraction"] = json!(false);
                    }
                }
                let stack = stack.restart().await;
                let mut wrong: ExtractJobInput =
                    serde_json::from_value(serde_json::to_value(&input).unwrap()).unwrap();
                match denied {
                    "skip" => wrong.skip = true,
                    "bank" => wrong.bank = Some("other".into()),
                    "turn" => wrong.turn_id = Some("other-turn".into()),
                    _ => {}
                }
                let rejection = stack.run(&wrong).await;
                if denied == "automation" {
                    rejection.unwrap();
                } else {
                    assert!(rejection.is_err());
                }
                assert_eq!(stack.log_records("private"), records);
                assert!(stack.log_records("other").is_empty());
                assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
                assert_eq!(
                    stack
                        .deps
                        .store()
                        .await
                        .pending_extraction("incremental-fixture")
                        .unwrap(),
                    pending
                );
                {
                    let s = stack.script.lock().unwrap();
                    assert_eq!(s.count("router::complete"), 0);
                    assert_eq!(s.count("state::set"), 0);
                    assert_eq!(s.count("memory::on-item-changed"), 0);
                    assert_eq!(s.count("memory::on-bank-changed"), 0);
                }
                {
                    let mut s = stack.script.lock().unwrap();
                    s.kind = None;
                    s.path[2]["origin"]["memory_extraction"] = json!(true);
                }
                let stack = stack.restart().await;
                stack.run(&input).await.unwrap();
                stack.run(&input).await.unwrap();
                let recovered = stack.log_records("private");
                assert_eq!(recovered.len(), 1);
                assert_eq!(recovered[0].text, text);
                assert_eq!(recovered[0].revision, 0);
                assert_eq!(recovered[0].corroboration, 0);
                if published {
                    assert_eq!(recovered, records);
                }
                assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
                assert!(stack
                    .deps
                    .store()
                    .await
                    .pending_extraction("incremental-fixture")
                    .unwrap()
                    .is_none());
                let s = stack.script.lock().unwrap();
                assert_eq!(s.count("router::complete"), usize::from(!published));
                assert_eq!(s.count("memory::on-item-changed"), usize::from(!published));
                assert_eq!(s.count("state::set"), 1);
            }
        }
    }

    #[tokio::test]
    async fn automation_cannot_replay_pending_corroboration_or_correction_after_reopen() {
        for correction in [false, true] {
            let stack = correction_stack().await;
            let original = stack.log_records("private")[0].clone();
            if !correction {
                let mut s = stack.script.lock().unwrap();
                s.path[2] = message("end", &original.text);
                s.model_items = VecDeque::from([fact(&original.text)]);
            }
            stack.script.lock().unwrap().state_write_error = true;
            assert!(stack.run(&job("end")).await.is_err());
            let committed = stack.log_records("private");
            assert_eq!(committed.len(), if correction { 3 } else { 2 });
            assert_eq!(committed.last().unwrap().revision, original.revision + 1);
            if !correction {
                assert_eq!(committed[1].corroboration, original.corroboration + 1);
            } else {
                assert!(!committed[2].is_live());
            }
            let pending = stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap();
            {
                let mut s = stack.script.lock().unwrap();
                s.state_write_error = false;
                s.kind = Some("automation".into());
            }
            let stack = stack.restart().await;
            stack.run(&job("end")).await.unwrap();
            assert_eq!(stack.log_records("private"), committed);
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"old"}));
            assert_eq!(
                stack
                    .deps
                    .store()
                    .await
                    .pending_extraction("incremental-fixture")
                    .unwrap(),
                pending
            );
            {
                let s = stack.script.lock().unwrap();
                assert_eq!(s.count("state::get"), 0);
                assert_eq!(s.count("state::set"), 0);
                assert_eq!(s.count("router::complete"), 0);
                assert_eq!(s.count("memory::on-item-changed"), 0);
            }
            stack.script.lock().unwrap().kind = None;
            let stack = stack.restart().await;
            stack.run(&job("end")).await.unwrap();
            stack.run(&job("end")).await.unwrap();
            assert_eq!(stack.log_records("private"), committed);
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("memory::on-item-changed"), 0);
            assert_eq!(s.count("state::set"), 1);
        }
    }

    #[tokio::test]
    async fn later_job_recovers_an_earlier_turns_saved_run_under_that_turns_bank() {
        let first = "Amber instruments require copper calibration.";
        let second = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("first", first),
                stamped("first-end", "first-turn", "private", true),
                message("second", second),
                stamped("end", "second-turn", "other", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            state_write_error: true,
            model_items: VecDeque::from([fact(first), fact(second)]),
            ..Script::default()
        })
        .await;
        let mut first_job = job("first-end");
        first_job.turn_id = Some("first-turn".into());
        assert!(stack.run(&first_job).await.is_err());
        let original = stack.log_records("private");
        stack.script.lock().unwrap().state_write_error = false;
        let stack = stack.restart().await;
        let mut later = job("end");
        later.turn_id = Some("second-turn".into());
        later.bank = Some("other".into());
        // The later job finishes the earlier turn's saved run first, from its
        // saved decision and under its own bank, then runs its own.
        stack.run(&later).await.unwrap();
        assert_eq!(stack.log_records("private"), original);
        let other = stack.log_records("other");
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].text, second);
        assert_eq!(other[0].revision, 0);
        assert_eq!(other[0].corroboration, 0);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 1);
            assert_eq!(s.count("memory::on-item-changed"), 1);
            assert_eq!(s.count("state::set"), 2);
            assert!(s.inputs()[0].contains(second));
            assert!(!s.inputs()[0].contains(first));
        }
        let stack = stack.restart().await;
        stack.run(&first_job).await.unwrap();
        stack.run(&later).await.unwrap();
        assert_eq!(stack.log_records("private"), original);
        assert_eq!(stack.log_records("other"), other);
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("state::set"), 0);
    }

    #[tokio::test]
    async fn exhausted_owner_run_waits_through_off_and_the_next_eligible_job_runs_it_across_restart(
    ) {
        for later_bank in ["private", "other"] {
            let first = "ORIGINAL_SOURCE_CANARY copper calibration remains mandatory.";
            let second = "Birch archives retain monthly invoices.";
            let path = vec![
                message("anchor", ""),
                message("first-user", first),
                stamped("first-end", "first-turn", "private", true),
                message("off-user", "OFF_SOURCE_CANARY must never enter extraction"),
                stamped("off-end", "off-turn", "private", false),
                message("later-user", second),
                stamped("later-end", "later-turn", later_bank, true),
            ];
            let stack = Boundary::new(Script {
                path: path.clone(),
                cursor: Some("anchor".into()),
                persistent_cursor: true,
                observe_events: true,
                model_failures: 3,
                model_items: VecDeque::from([fact(first), fact(second)]),
                ..Script::default()
            })
            .await;
            let mut first_job = job("first-end");
            first_job.turn_id = Some("first-turn".into());
            for _ in 0..3 {
                assert!(stack
                    .run(&first_job)
                    .await
                    .unwrap_err()
                    .contains("provider error"));
            }
            assert_eq!(stack.script.lock().unwrap().count("router::complete"), 3);
            let saved = stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap()
                .unwrap();
            assert_eq!(saved["attempts"], 3);
            assert_eq!(saved["entries"][0][2], first);
            let stack = stack.restart().await;
            // An OFF job never runs eligible text: it keeps the run and the
            // cursor before it, for the next eligible job.
            let mut off = job("off-end");
            off.turn_id = Some("off-turn".into());
            off.skip = true;
            stack.run(&off).await.unwrap();
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
            assert!(stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap()
                .is_none());
            let ranges = stack
                .deps
                .store()
                .await
                .extraction_ranges("incremental-fixture")
                .unwrap();
            assert_eq!(ranges.len(), 1);
            assert!(!ranges[0].completed);
            assert_eq!(ranges[0].pending.entries[0].2, first);
            assert_eq!(ranges[0].pending.bank, "private");
            assert_eq!(ranges[0].pending.turn_id.as_deref(), Some("first-turn"));
            assert_eq!(stack.script.lock().unwrap().count("router::complete"), 0);
            assert_eq!(stack.script.lock().unwrap().count("state::set"), 0);
            let stack = stack.restart().await;
            // The next eligible job runs the exhausted run under its own
            // turn's bank, passes the OFF turn and runs its own.
            let mut later = job("later-end");
            later.turn_id = Some("later-turn".into());
            later.bank = Some(later_bank.into());
            stack.run(&later).await.unwrap();
            stack.run(&later).await.unwrap();
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"later-end"}));
            assert!(stack
                .deps
                .store()
                .await
                .extraction_ranges("incremental-fixture")
                .unwrap()
                .is_empty());
            let private = stack.log_records("private");
            assert!(private
                .iter()
                .any(|r| r.text == first && r.revision == 0 && r.corroboration == 0));
            assert_eq!(private.len(), if later_bank == "private" { 2 } else { 1 });
            assert_eq!(stack.log_records(later_bank).last().unwrap().text, second);
            {
                let s = stack.script.lock().unwrap();
                assert_eq!(s.path, path);
                assert_eq!(s.count("router::complete"), 2);
                assert_eq!(s.count("memory::on-item-changed"), 2);
                assert!(s.inputs()[0].contains(first));
                assert!(!s.inputs()[0].contains(second));
                assert!(s.inputs()[1].contains(second));
                assert!(s.inputs().iter().all(|i| !i.contains("OFF_SOURCE_CANARY")));
            }
            // The exhausted turn's own job, redelivered, finds nothing left.
            let stack = stack.restart().await;
            stack.run(&first_job).await.unwrap();
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 0);
        }
    }

    #[tokio::test]
    async fn bank_deleted_while_model_waits_is_not_recreated_or_published_after_reopen() {
        let old = "DELETED_DURING_MODEL_CANARY requires copper calibration.";
        let new = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("old", old),
                stamped("old-end", "old-turn", "private", true),
                message("new", new),
                stamped("end", "new-turn", "other", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            model_items: VecDeque::from([fact(old), fact(new)]),
            ..Script::default()
        })
        .await;
        stack
            .deps
            .store()
            .await
            .ensure_bank("private", None)
            .await
            .unwrap();
        let pause = Arc::new((
            tokio::sync::Semaphore::new(0),
            tokio::sync::Semaphore::new(0),
        ));
        stack.script.lock().unwrap().model_pause = Some(pause.clone());
        let mut owner = job("old-end");
        owner.turn_id = Some("old-turn".into());
        let work = stack.run(&owner);
        let delete = async {
            tokio::time::timeout(std::time::Duration::from_secs(5), pause.0.acquire())
                .await
                .unwrap()
                .unwrap()
                .forget();
            stack
                .deps
                .store()
                .await
                .trash_bank("private")
                .await
                .unwrap();
            pause.1.add_permits(1);
        };
        let (result, _) = tokio::join!(work, delete);
        assert!(result.unwrap_err().contains("deleted during run"));
        assert!(!stack.dir.path().join("private").exists());
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
        assert_eq!(
            stack
                .script
                .lock()
                .unwrap()
                .count("memory::on-item-changed"),
            0
        );
        assert_eq!(
            stack
                .script
                .lock()
                .unwrap()
                .count("memory::on-bank-changed"),
            0
        );
        stack.script.lock().unwrap().model_pause = None;
        let stack = stack.restart().await;
        let mut later = job("end");
        later.turn_id = Some("new-turn".into());
        later.bank = Some("other".into());
        stack.run(&later).await.unwrap();
        assert!(!stack.dir.path().join("private").exists());
        assert_eq!(stack.log_records("other").len(), 1);
        assert_eq!(stack.log_records("other")[0].text, new);
        // A deleted bank's run is closed unextracted: nothing pins the cursor
        // and none of its text stays retained.
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        assert!(stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 1);
        assert_eq!(s.count("memory::on-item-changed"), 1);
        assert_eq!(s.count("memory::on-bank-changed"), 1);
        let item = s
            .calls
            .iter()
            .find(|(id, _)| id == "memory::on-item-changed")
            .unwrap();
        assert_eq!(item.1["event_type"], "created");
        assert_eq!(item.1["bank"], "other");
        assert_eq!(
            item.1["memory"],
            serde_json::to_value(&stack.log_records("other")[0]).unwrap()
        );
        let bank = s
            .calls
            .iter()
            .find(|(id, _)| id == "memory::on-bank-changed")
            .unwrap();
        assert_eq!(bank.1, json!({"event_type":"created", "bank":"other"}));
        assert!(!s.inputs()[0].contains("DELETED_DURING_MODEL_CANARY"));
    }

    #[tokio::test]
    async fn exhausted_legacy_prefix_never_grants_policy_to_an_unstamped_later_job() {
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message(
                    "old",
                    "RETAINED_CANARY copper calibration remains mandatory.",
                ),
                message(
                    "end",
                    "UNKNOWN_POLICY_CANARY has no independently attested boundary.",
                ),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            model_failures: 3,
            ..Script::default()
        })
        .await;
        for _ in 0..3 {
            assert!(stack.run(&job("old")).await.is_err());
        }
        let pending = stack
            .deps
            .store()
            .await
            .pending_extraction("incremental-fixture")
            .unwrap();
        let stack = stack.restart().await;
        // Nothing after the legacy run is stamped: no later job may run the
        // old run under its own bank, so they all wait.
        for bank in ["private", "other"] {
            let mut later = job("end");
            later.bank = Some(bank.into());
            stack.run(&later).await.unwrap();
        }
        let mut off = job("end");
        off.skip = true;
        off.bank = None;
        stack.run(&off).await.unwrap();
        assert_eq!(
            stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap(),
            pending
        );
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
        assert!(stack.log_records("private").is_empty());
        assert!(stack.log_records("other").is_empty());
        {
            let s = stack.script.lock().unwrap();
            for function in [
                "router::complete",
                "state::set",
                "memory::on-item-changed",
                "memory::on-bank-changed",
            ] {
                assert_eq!(s.count(function), 0);
            }
        }
        let stack = stack.restart().await;
        stack
            .script
            .lock()
            .unwrap()
            .model_items
            .push_back(json!([]));
        // Replace only the previously unknown suffix with its own real stamp.
        // The legacy request has no stamped turn: it passes unextracted.
        stack.script.lock().unwrap().path = vec![
            message("anchor", ""),
            message(
                "old",
                "RETAINED_CANARY copper calibration remains mandatory.",
            ),
            message(
                "new-user",
                "The independently stamped suffix remains eligible.",
            ),
            stamped("end", "new-turn", "other", true),
        ];
        let mut later = job("end");
        later.turn_id = Some("new-turn".into());
        later.bank = Some("other".into());
        stack.run(&later).await.unwrap();
        assert_eq!(stack.script.lock().unwrap().count("router::complete"), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        assert!(stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
        assert!(stack
            .deps
            .store()
            .await
            .pending_extraction("incremental-fixture")
            .unwrap()
            .is_none());
        assert!(stack
            .script
            .lock()
            .unwrap()
            .inputs()
            .iter()
            .all(|p| !p.contains("RETAINED_CANARY")));
    }

    #[tokio::test]
    async fn retained_prefix_and_duplicate_pending_survive_bounded_later_run_recovery() {
        let old = "RETAINED_PREFIX_CANARY copper calibration remains mandatory.";
        let mut path = vec![
            message("anchor", ""),
            message("old-user", old),
            stamped("old-end", "old-turn", "private", true),
        ];
        for n in 0..49 {
            // Ten ~2.2KB entries per 24KB call: 49 entries make five runs.
            let mut item = message(&format!("new-{n}"), &"later eligible source ".repeat(100));
            item["origin"]["turn_id"] = json!("later-turn");
            path.push(item);
        }
        path.push(stamped("end", "later-turn", "other", true));
        let stack = Boundary::new(Script {
            path,
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            model_failures: 3,
            model_items: VecDeque::from(vec![json!([]); 6]),
            ..Script::default()
        })
        .await;
        let mut owner = job("old-end");
        owner.turn_id = Some("old-turn".into());
        for _ in 0..3 {
            assert!(stack.run(&owner).await.is_err());
        }
        let pending = memory::store::PendingExtraction::parse(
            stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        // Simulate a crash after retention persisted but before the pending
        // slot was cleared; reopen must merge, not evict or duplicate source.
        stack
            .deps
            .store()
            .await
            .retain_extraction_range("incremental-fixture", pending.clone(), false)
            .unwrap();
        let stack = stack.restart().await;
        let mut later = job("end");
        later.turn_id = Some("later-turn".into());
        later.bank = Some("other".into());
        // The exhausted run goes first, under its own bank, then three of the
        // later turn's five runs: four calls, then a retry.
        assert!(stack
            .run(&later)
            .await
            .unwrap_err()
            .contains("remaining runs"));
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 4);
            assert!(s.inputs()[0].contains("RETAINED_PREFIX_CANARY"));
            assert!(s.inputs()[1..]
                .iter()
                .all(|input| !input.contains("RETAINED_PREFIX_CANARY")));
        }
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"new-32"}));
        assert!(stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
        let stack = stack.restart().await;
        stack.run(&later).await.unwrap();
        stack.run(&later).await.unwrap();
        assert!(stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        assert!(stack.log_records("private").is_empty());
        assert!(stack.log_records("other").is_empty());
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 2);
        assert_eq!(s.count("memory::on-item-changed"), 0);
        assert_eq!(s.count("memory::on-bank-changed"), 0);
        assert_eq!(s.count("state::set"), 2);
    }

    #[tokio::test]
    async fn raw_source_edit_hidden_by_redaction_runs_as_edited_without_blocking_later_turn() {
        let key = |letter: &str| format!("sk-ant-api03-{}", letter.repeat(40));
        let original = format!("Copper calibration is mandatory; pasted key {}", key("a"));
        let edited = format!("Copper calibration is mandatory; pasted key {}", key("b"));
        assert_eq!(
            memory::redact::redact(&original).text,
            memory::redact::redact(&edited).text
        );
        let later_text = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("old", &original),
                stamped("old-end", "old-turn", "private", true),
                message("new", later_text),
                stamped("end", "new-turn", "other", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            model_failures: 3,
            model_items: VecDeque::from([json!([]), fact(later_text)]),
            ..Script::default()
        })
        .await;
        let mut owner = job("old-end");
        owner.turn_id = Some("old-turn".into());
        for _ in 0..3 {
            assert!(stack.run(&owner).await.is_err());
        }
        let saved = stack
            .deps
            .store()
            .await
            .pending_extraction("incremental-fixture")
            .unwrap()
            .unwrap();
        assert_eq!(saved["version"], 1);
        assert_eq!(saved["source_digest"].as_str().unwrap().len(), 32);
        assert!(!saved.to_string().contains(&key("a")));
        stack.script.lock().unwrap().path[1] = message("old", &edited);
        let stack = stack.restart().await;
        let mut later = job("end");
        later.turn_id = Some("new-turn".into());
        later.bank = Some("other".into());
        // The saved run no longer matches the raw source: it is dropped, and
        // the edited run runs as the path holds it now.
        stack.run(&later).await.unwrap();
        assert!(stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
        assert_eq!(stack.log_records("other")[0].text, later_text);
        assert_eq!(stack.log_records("other").len(), 1);
        assert!(stack.log_records("private").is_empty());
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 2);
            assert!(s.inputs()[0].contains("Copper calibration is mandatory"));
            assert!(s.inputs().iter().all(|i| !i.contains(&key("b"))));
        }
        let stack = stack.restart().await;
        stack.run(&owner).await.unwrap();
        let s = stack.script.lock().unwrap();
        for function in [
            "router::complete",
            "state::set",
            "memory::on-item-changed",
            "memory::on-bank-changed",
        ] {
            assert_eq!(s.count(function), 0);
        }
    }

    #[tokio::test]
    async fn stored_decision_unknown_schema_or_source_mismatch_is_not_replayed() {
        for mutation in ["version", "unknown", "source", "item"] {
            let text = "Amber instruments require copper calibration.";
            let stack = Boundary::new(Script {
                path: vec![message("anchor", ""), message("end", text)],
                cursor: Some("anchor".into()),
                persistent_cursor: true,
                observe_events: true,
                state_write_error: true,
                model_items: VecDeque::from([fact(text)]),
                ..Script::default()
            })
            .await;
            assert!(stack
                .run(&job("end"))
                .await
                .unwrap_err()
                .contains("state::set"));
            let records = stack.log_records("private");
            let source =
                json!(["incremental-fixture", "private", false, null, null, ["end"]]).to_string();
            // Locate the persisted envelope by exact source, not its hashed filename.
            let (path, mut envelope) = std::fs::read_dir(stack.dir.path().join(".extraction"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter_map(|path| {
                    let value: Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    (value["source"].as_str() == Some(source.as_str())).then_some((path, value))
                })
                .next()
                .unwrap();
            match mutation {
                "version" => envelope["plan"]["version"] = json!(999),
                "unknown" => envelope["plan"]["authority"] = json!(true),
                "source" => {
                    envelope["plan"]["capture"]["entries"][0][2] =
                        json!("Changed source under the same opaque ID")
                }
                _ => envelope["plan"]["items"][0]["authority"] = json!(true),
            }
            std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
            stack.script.lock().unwrap().state_write_error = false;
            let stack = stack.restart().await;
            assert!(stack.run(&job("end")).await.is_err());
            assert_eq!(stack.log_records("private"), records);
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("memory::on-item-changed"), 0);
            assert_eq!(s.count("memory::on-bank-changed"), 0);
            assert_eq!(s.count("state::set"), 0);
        }
    }

    #[tokio::test]
    async fn stale_retained_source_cannot_block_a_new_independently_stamped_turn() {
        for mutation in ["text", "timestamp", "policy", "unknown-policy"] {
            let old = "ORIGINAL_RETAINED_CANARY requires copper calibration.";
            let new = "Birch archives retain monthly invoices.";
            // A changed source runs again as it is now; a changed policy
            // passes it unextracted.
            let reruns = matches!(mutation, "text" | "timestamp");
            let mut items = VecDeque::from([fact(new)]);
            if reruns {
                items.push_front(json!([]));
            }
            let stack = Boundary::new(Script {
                path: vec![
                    message("anchor", ""),
                    message("first", old),
                    stamped("first-end", "first-turn", "private", true),
                    message("new", new),
                    stamped("end", "new-turn", "other", true),
                ],
                cursor: Some("anchor".into()),
                persistent_cursor: true,
                observe_events: true,
                model_failures: 3,
                model_items: items,
                ..Script::default()
            })
            .await;
            let mut owner = job("first-end");
            owner.turn_id = Some("first-turn".into());
            for _ in 0..3 {
                assert!(stack.run(&owner).await.is_err());
            }
            {
                let mut s = stack.script.lock().unwrap();
                match mutation {
                    "text" => {
                        s.path[1] = message("first", "Edited source cannot authorize the old plan.")
                    }
                    "timestamp" => s.path[1]["message"]["timestamp"] = json!(999999),
                    "policy" => s.path[2]["origin"]["memory_extraction"] = json!(false),
                    _ => s.path[2]["origin"]["memory_extraction"] = json!("unknown"),
                }
            }
            let stack = stack.restart().await;
            let mut later = job("end");
            later.turn_id = Some("new-turn".into());
            later.bank = Some("other".into());
            stack.run(&later).await.unwrap();
            assert!(stack
                .deps
                .store()
                .await
                .extraction_ranges("incremental-fixture")
                .unwrap()
                .is_empty());
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
            assert!(stack.log_records("private").is_empty());
            assert_eq!(stack.log_records("other").len(), 1);
            assert_eq!(stack.log_records("other")[0].text, new);
            {
                let s = stack.script.lock().unwrap();
                assert_eq!(s.count("router::complete"), 1 + usize::from(reruns));
                if mutation != "timestamp" {
                    assert!(s.inputs().iter().all(|i| !i.contains("ORIGINAL_RETAINED")));
                }
            }
            let stack = stack.restart().await;
            stack.run(&owner).await.unwrap();
            assert_eq!(stack.script.lock().unwrap().count("router::complete"), 0);
            assert_eq!(
                stack
                    .script
                    .lock()
                    .unwrap()
                    .count("memory::on-item-changed"),
                0
            );
        }
    }

    #[tokio::test]
    async fn published_owner_deadletter_reuses_original_receipt_without_repeat_events_or_model() {
        let first = "ORIGINAL_PUBLISHED_CANARY copper calibration is mandatory.";
        let second = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("first-user", first),
                stamped("first-end", "first-turn", "private", true),
                message("later-user", second),
                stamped("end", "later-turn", "other", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            state_write_error: true,
            model_items: VecDeque::from([fact(first), fact(second)]),
            ..Script::default()
        })
        .await;
        let mut first_job = job("first-end");
        first_job.turn_id = Some("first-turn".into());
        for _ in 0..3 {
            assert!(stack
                .run(&first_job)
                .await
                .unwrap_err()
                .contains("state::set"));
        }
        let original = stack.log_records("private");
        assert_eq!(original.len(), 1);
        assert_eq!(stack.script.lock().unwrap().count("router::complete"), 1);
        assert_eq!(
            stack
                .script
                .lock()
                .unwrap()
                .count("memory::on-item-changed"),
            1
        );
        stack.script.lock().unwrap().state_write_error = false;
        let stack = stack.restart().await;
        let mut later = job("end");
        later.turn_id = Some("later-turn".into());
        later.bank = Some("other".into());
        // The dead-lettered run finishes from its receipts: no model call and
        // no second event for it; then the later turn runs.
        stack.run(&later).await.unwrap();
        assert_eq!(stack.log_records("private"), original);
        assert_eq!(stack.log_records("other").len(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        {
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 1);
            assert!(s.inputs()[0].contains(second));
            assert!(!s.inputs()[0].contains("ORIGINAL_PUBLISHED_CANARY"));
            assert_eq!(s.count("memory::on-item-changed"), 1);
            assert_eq!(s.count("state::set"), 2);
        }
        let stack = stack.restart().await;
        stack.run(&first_job).await.unwrap();
        stack.run(&later).await.unwrap();
        assert_eq!(stack.log_records("private"), original);
        assert_eq!(stack.log_records("other").len(), 1);
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("memory::on-item-changed"), 0);
        assert_eq!(s.count("memory::on-bank-changed"), 0);
        assert_eq!(s.count("state::set"), 0);
    }

    #[tokio::test]
    async fn completed_retention_receipt_survives_failed_cursor_confirmation_without_model_replay()
    {
        let first = "Original source requires copper calibration.";
        let second = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("first-user", first),
                stamped("first-end", "first-turn", "private", true),
                message("later-user", second),
                stamped("end", "later-turn", "other", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            model_failures: 3,
            model_items: VecDeque::from([fact(first), fact(second)]),
            ..Script::default()
        })
        .await;
        let mut owner = job("first-end");
        owner.turn_id = Some("first-turn".into());
        for _ in 0..3 {
            assert!(stack.run(&owner).await.is_err());
        }
        let mut later = job("end");
        later.turn_id = Some("later-turn".into());
        later.bank = Some("other".into());
        // The exhausted run completes, but its cursor confirmation fails.
        stack.script.lock().unwrap().state_write_error = true;
        assert!(stack.run(&later).await.unwrap_err().contains("state::set"));
        let records = stack.log_records("private");
        assert_eq!(records.len(), 1);
        let ranges = stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap();
        assert_eq!(ranges.len(), 1);
        assert!(ranges[0].completed);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
        stack.script.lock().unwrap().state_write_error = false;
        let stack = stack.restart().await;
        stack.run(&later).await.unwrap();
        assert_eq!(stack.log_records("private"), records);
        assert_eq!(stack.log_records("other").len(), 1);
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        assert!(stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
        let s = stack.script.lock().unwrap();
        // Only the later turn asks the model: the completed run is not replayed.
        assert_eq!(s.count("router::complete"), 1);
        assert!(s.inputs()[0].contains(second));
        assert_eq!(s.count("state::set"), 2);
        assert_eq!(s.count("memory::on-item-changed"), 1);
    }

    #[tokio::test]
    async fn exhausted_run_cannot_be_published_by_a_wrong_callback() {
        for mutation in ["bank", "turn", "skip"] {
            let first = "ORIGINAL_EXHAUSTED_CANARY requires copper calibration.";
            let second = "Birch archives retain monthly invoices.";
            let stack = Boundary::new(Script {
                path: vec![
                    message("anchor", ""),
                    message("first-user", first),
                    stamped("first-end", "first-turn", "private", true),
                    message("later-user", second),
                    stamped("end", "later-turn", "other", true),
                ],
                cursor: Some("anchor".into()),
                persistent_cursor: true,
                observe_events: true,
                model_failures: 3,
                ..Script::default()
            })
            .await;
            let mut owner = job("first-end");
            owner.turn_id = Some("first-turn".into());
            for _ in 0..3 {
                assert!(stack.run(&owner).await.is_err());
            }
            let pending = stack
                .deps
                .store()
                .await
                .pending_extraction("incremental-fixture")
                .unwrap();
            let mut later = job("end");
            later.turn_id = Some("later-turn".into());
            later.bank = Some("other".into());
            match mutation {
                "bank" => later.bank = Some("private".into()),
                "turn" => later.turn_id = Some("wrong-turn".into()),
                _ => later.skip = true,
            }
            let stack = stack.restart().await;
            assert!(stack.run(&later).await.is_err(), "{mutation}");
            assert_eq!(
                stack
                    .deps
                    .store()
                    .await
                    .pending_extraction("incremental-fixture")
                    .unwrap(),
                pending
            );
            assert!(stack.log_records("other").is_empty());
            assert!(stack.log_records("private").is_empty());
            assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"anchor"}));
            let s = stack.script.lock().unwrap();
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), 0);
            assert_eq!(s.count("memory::on-item-changed"), 0);
        }
    }

    #[tokio::test]
    async fn exhausted_deleted_bank_run_closes_without_recreating_or_blocking_later_bank() {
        let first = "ORIGINAL_DELETED_CANARY requires copper calibration.";
        let later_text = "Birch archives retain monthly invoices.";
        let stack = Boundary::new(Script {
            path: vec![
                message("anchor", ""),
                message("first-user", first),
                stamped("first-end", "first-turn", "private", true),
                message("later-user", later_text),
                stamped("end", "later-turn", "other", true),
            ],
            cursor: Some("anchor".into()),
            persistent_cursor: true,
            observe_events: true,
            model_failures: 3,
            model_items: VecDeque::from([fact(later_text)]),
            ..Script::default()
        })
        .await;
        let (bank, _) = stack
            .deps
            .store()
            .await
            .ensure_bank("private", None)
            .await
            .unwrap();
        bank.set_rule("canary", "Original hand policy must remain unchanged.")
            .unwrap();
        let before = std::fs::read(bank.dir().join("rules/canary.md")).unwrap();
        let mut first_job = job("first-end");
        first_job.turn_id = Some("first-turn".into());
        for _ in 0..3 {
            assert!(stack.run(&first_job).await.is_err());
        }
        let trash = stack
            .deps
            .store()
            .await
            .trash_bank("private")
            .await
            .unwrap();
        let stack = stack.restart().await;
        let mut later = job("end");
        later.turn_id = Some("later-turn".into());
        later.bank = Some("other".into());
        stack.run(&later).await.unwrap();
        let records = stack.log_records("other");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, later_text);
        assert!(!stack.dir.path().join("private").exists());
        assert_eq!(
            std::fs::read(std::path::Path::new(&trash).join("rules/canary.md")).unwrap(),
            before
        );
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        let stack = stack.restart().await;
        stack.run(&later).await.unwrap();
        assert_eq!(stack.log_records("other"), records);
        // The deleted bank's text is not retained anywhere.
        assert!(stack
            .deps
            .store()
            .await
            .extraction_ranges("incremental-fixture")
            .unwrap()
            .is_empty());
        assert_eq!(stack.disk_cursor(), json!({"last_entry_id":"end"}));
        let s = stack.script.lock().unwrap();
        assert_eq!(s.count("router::complete"), 0);
        assert_eq!(s.count("state::set"), 0);
        assert_eq!(s.count("memory::on-item-changed"), 0);
        assert_eq!(s.count("memory::on-bank-changed"), 0);
    }

    #[tokio::test]
    async fn skip_and_automation_advance_only_the_completed_boundary() {
        for skip in [true, false] {
            let stack = Boundary::new(Script {
                path: vec![
                    message("anchor", ""),
                    message("end", "not eligible durable message"),
                    message("later", "next job"),
                ],
                cursor: Some("anchor".into()),
                kind: (!skip).then(|| "automation".into()),
                ..Script::default()
            })
            .await;
            let mut input = job("end");
            input.skip = skip;
            stack.run(&input).await.unwrap();
            let s = stack.script.lock().unwrap();
            assert_eq!(
                s.cursor.as_deref(),
                Some(if skip { "end" } else { "anchor" })
            );
            assert_eq!(s.count("router::complete"), 0);
            assert_eq!(s.count("state::set"), usize::from(skip));
            assert_eq!(stack.commits(), 0);
        }
    }
}
