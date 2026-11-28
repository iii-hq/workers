//! The context-items rules, ported from the ADE's `chat-aside.test.ts`
//! (`collectChatLinks`, `collectChatImages`, `collectChatFiles`,
//! `dismissedPullRequests`, `summarizeChecks`), plus incremental scanning.

use super::*;

fn say(id: &str, role: &str, text: &str) -> Value {
    json!({ "entry_id": id, "message": {
        "role": role, "timestamp": 1,
        "content": [{ "type": "text", "text": text }],
    }})
}

fn result(id: &str, call: &str, function: &str, content: Value, details: Value) -> Value {
    json!({ "entry_id": id, "message": {
        "role": "function_result", "function_call_id": call, "function_id": function,
        "content": content, "details": details, "is_error": false, "timestamp": 1,
    }})
}

fn scan(items: &[Value]) -> SessionIndex {
    let mut index = SessionIndex::default();
    index.apply(items, false);
    index
}

type Lists = (
    Vec<PullRequestItem>,
    Vec<LinkItem>,
    Vec<AttachmentItem>,
    Vec<ImageItem>,
    Truncated,
);

fn lists(items: &[Value], metadata: Value) -> Lists {
    aggregate(&scan(items), metadata.as_object())
}

/// chat-aside: "splits pull requests from other links, newest first, deduped".
#[test]
fn splits_pull_requests_from_other_links_newest_first_deduped() {
    let items = [
        say("e1", "user", "see https://docs.example.com/console."),
        say(
            "e2",
            "assistant",
            "Opened https://github.com/iii-hq/workers/pull/1371 and https://docs.example.com/console",
        ),
        say(
            "e3",
            "assistant",
            "Again: https://github.com/iii-hq/workers/pull/1371/files",
        ),
    ];
    let (prs, links, ..) = lists(&items, json!({}));
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].repo, "iii-hq/workers");
    assert_eq!(prs[0].number, 1371);
    assert_eq!(prs[0].url, "https://github.com/iii-hq/workers/pull/1371");
    assert_eq!(prs[0].entry_id, "e3", "the newest mention");
    assert_eq!(prs[0].source, "mentioned");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://docs.example.com/console");
    assert_eq!(links[0].host, "docs.example.com");
    assert_eq!(links[0].count, 2, "two mentions, one row");
    assert_eq!(links[0].entry_id, "e2");
}

/// chat-aside: "reads the PR a github::pr::create call returned".
#[test]
fn reads_the_pr_a_github_pr_create_call_returned() {
    let items = [
        say("e1", "user", "Is https://github.com/o/r/pull/7 merged?"),
        result(
            "e2",
            "c1",
            "github::pr::create",
            json!([{ "type": "text", "text": "created" }]),
            json!({ "url": "https://github.com/o/r/pull/7" }),
        ),
        result(
            "e3",
            "c2",
            "github::pr::list",
            json!([{ "type": "text", "text": "https://github.com/o/r/pull/9" }]),
            Value::Null,
        ),
    ];
    let (prs, links, ..) = lists(&items, json!({}));
    assert_eq!(
        prs.iter()
            .map(|p| (p.number, p.source.as_str(), p.entry_id.as_str()))
            .collect::<Vec<_>>(),
        [(9, "mentioned", "e3"), (7, "created", "e2")]
    );
    assert!(links.is_empty(), "function results give PRs, never links");
}

/// chat-aside review fixes: a malformed escape keeps its link, and PR URLs
/// are case-insensitive.
#[test]
fn keeps_a_link_with_a_malformed_escape_and_reads_pr_urls_without_case() {
    let (_, links, ..) = lists(
        &[say(
            "e1",
            "assistant",
            "GET https://api.example.com/users/%s please",
        )],
        json!({}),
    );
    assert_eq!(links[0].url, "https://api.example.com/users/%s");
    assert_eq!(links[0].host, "api.example.com");

    let (prs, ..) = lists(
        &[
            say("e1", "user", "https://github.com/IIi-hq/Workers/pull/5"),
            say("e2", "user", "https://github.com/iii-hq/workers/pull/5"),
        ],
        json!({}),
    );
    assert_eq!(prs.len(), 1);
}

/// Spec §1.2.4 links: thinking blocks, call arguments and data URLs are left
/// out; trailing `).,;:!?'"` is trimmed; a Markdown link's text is its title.
#[test]
fn links_skip_thinking_and_call_arguments_and_take_markdown_titles() {
    let assistant = json!({ "entry_id": "e2", "message": {
        "role": "assistant", "timestamp": 1,
        "content": [
            { "type": "thinking", "text": "maybe https://thinking.example.com" },
            { "type": "text", "text": "Read [the guide](https://docs.example.com/guide). Also (https://b.example.com/x), 'https://c.example.com/y'! data:image/png;base64,AAAA" },
            { "type": "function_call", "id": "c1", "function_id": "agent_call",
              "arguments": { "function": "web::fetch", "payload": { "url": "https://args.example.com" } } },
        ],
    }});
    let (_, links, ..) = lists(&[assistant], json!({}));
    assert_eq!(
        links
            .iter()
            .map(|l| (l.url.as_str(), l.title.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("https://docs.example.com/guide", Some("the guide")),
            ("https://b.example.com/x", None),
            ("https://c.example.com/y", None),
        ]
    );
}

/// chat-aside: "reads the dismissed PRs off the session metadata".
#[test]
fn dismissed_pull_requests_are_left_out() {
    let items = [say(
        "e1",
        "user",
        "https://github.com/o/r/pull/7 and https://github.com/o/r/pull/8",
    )];
    let (prs, ..) = lists(&items, json!({ "context_dismissed_prs": ["o/r#7", 3] }));
    assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), [8]);
}

/// chat-aside: "lists attached and tool images, newest first" — by block:
/// sent in a user message, received in a function result.
#[test]
fn lists_attached_and_tool_images_newest_first() {
    let attached = json!({ "entry_id": "u1", "message": {
        "role": "user", "timestamp": 1,
        "content": [
            { "type": "text", "text": "look" },
            { "type": "image", "mime": "image/png", "data": "", "attachment_id": "a1" },
            { "type": "file", "attachment_id": "a2", "name": "notes.md", "mime": "text/markdown", "size": 10 },
        ],
    }});
    let tool = result(
        "c1r",
        "c1",
        "browser::screenshot",
        json!([
            { "type": "text", "text": "done" },
            { "type": "image", "mime": "image/jpeg", "data": "AAAA" },
        ]),
        Value::Null,
    );
    let (_, _, attachments, images, _) = lists(&[attached, tool], json!({}));
    assert_eq!(
        images
            .iter()
            .map(|i| (
                i.entry_id.as_str(),
                i.block_index,
                i.mime.as_str(),
                i.direction.as_str(),
                i.attachment_id.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("c1r", 1, "image/jpeg", "received", None),
            ("u1", 1, "image/png", "sent", Some("a1")),
        ]
    );
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].name, "notes.md");
    assert_eq!(attachments[0].direction, "sent");
    assert_eq!(attachments[0].size, Some(10));
}

/// collectChatFiles: attached files are sent; a file the agent wrote outside
/// the working folder is received; one inside it, a failed write, or a
/// sandbox write is not listed.
#[test]
fn files_the_agent_wrote_count_as_received() {
    let calls = json!({ "entry_id": "a1", "message": {
        "role": "assistant", "timestamp": 1,
        "content": [
            { "type": "function_call", "id": "w1", "function_id": "agent_call",
              "arguments": { "function": "shell::fs::write",
                             "payload": { "path": "/tmp/report.md", "content": "hello" } } },
            { "type": "function_call", "id": "w2", "function_id": "shell::fs::write",
              "arguments": { "files": [ { "path": "/work/proj/src/a.rs", "content": "x" },
                                        { "path": "/tmp/out.json", "content": "{}" } ] } },
            { "type": "function_call", "id": "w3", "function_id": "shell::fs::write",
              "arguments": { "path": "/tmp/failed.txt", "content": "no" } },
            { "type": "function_call", "id": "w4", "function_id": "sandbox::fs::write",
              "arguments": { "path": "/tmp/sandboxed.txt", "content": "no" } },
        ],
    }});
    let ok = |id: &str, call: &str| result(id, call, "shell::fs::write", json!([]), Value::Null);
    let mut failed = ok("r3", "w3");
    failed["message"]["is_error"] = json!(true);
    let items = [
        calls,
        ok("r1", "w1"),
        ok("r2", "w2"),
        failed,
        ok("r4", "w4"),
    ];
    let (_, _, attachments, ..) = lists(&items, json!({ "fs_scope": { "root": "/work/proj" } }));
    assert_eq!(
        attachments
            .iter()
            .map(|a| (
                a.name.as_str(),
                a.direction.as_str(),
                a.entry_id.as_str(),
                a.size,
                a.mime.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            ("out.json", "received", "r2", Some(2), "application/json"),
            ("report.md", "received", "r1", Some(5), "text/markdown"),
        ]
    );
    assert_eq!(attachments[1].path.as_deref(), Some("/tmp/report.md"));
}

/// Caps: 200 links, 100 attachments, 200 images, and `truncated` says so.
#[test]
fn caps_mark_the_lists_truncated() {
    let links: String = (0..205)
        .map(|i| format!("https://x{i}.example.com "))
        .collect();
    let blocks: Vec<Value> = std::iter::once(json!({ "type": "text", "text": links }))
        .chain((0..105).map(|i| {
            json!({ "type": "file", "attachment_id": format!("a{i}"),
                    "name": format!("f{i}.txt"), "mime": "text/plain", "size": 1 })
        }))
        .chain((0..205).map(|_| json!({ "type": "image", "mime": "image/png", "data": "" })))
        .collect();
    let user = json!({ "entry_id": "u", "message": {
        "role": "user", "timestamp": 1, "content": blocks,
    }});
    let (_, links, attachments, images, truncated) = lists(&[user], json!({}));
    assert_eq!(
        (links.len(), attachments.len(), images.len()),
        (200, 100, 200)
    );
    assert_eq!(
        truncated,
        Truncated {
            links: Some(true),
            attachments: Some(true),
            images: Some(true)
        }
    );
    let (.., truncated) = lists(&[say("e", "user", "https://a.example.com")], json!({}));
    assert_eq!(serde_json::to_value(truncated).unwrap(), json!({}));
}

/// chat-aside: "summarizes checks by their worst bucket", on `gh pr view`'s
/// `statusCheckRollup`.
#[test]
fn summarizes_checks_by_their_worst_state() {
    assert_eq!(summarize_checks(&[]), None);
    let pass = json!({ "__typename": "CheckRun", "status": "COMPLETED", "conclusion": "SUCCESS" });
    let skip = json!({ "__typename": "CheckRun", "status": "COMPLETED", "conclusion": "SKIPPED" });
    let running = json!({ "__typename": "CheckRun", "status": "IN_PROGRESS", "conclusion": "" });
    let failed =
        json!({ "__typename": "CheckRun", "status": "COMPLETED", "conclusion": "FAILURE" });
    let status_pending = json!({ "__typename": "StatusContext", "state": "PENDING" });
    let status_error = json!({ "__typename": "StatusContext", "state": "ERROR" });
    assert_eq!(summarize_checks(&[pass.clone(), skip]), Some("passing"));
    assert_eq!(summarize_checks(&[pass.clone(), running]), Some("pending"));
    assert_eq!(
        summarize_checks(&[pass.clone(), status_pending]),
        Some("pending")
    );
    assert_eq!(summarize_checks(&[pass.clone(), failed]), Some("failing"));
    assert_eq!(summarize_checks(&[pass, status_error]), Some("failing"));

    let view = r#"{"number":482,"title":"Login form","state":"OPEN","isDraft":true,"statusCheckRollup":[]}"#;
    assert_eq!(
        parse_pr_view(view),
        Some(PrReading {
            title: Some("Login form".into()),
            state: Some("draft".into()),
            checks: None
        })
    );
    assert_eq!(
        parse_pr_view(r#"{"state":"MERGED","isDraft":false}"#)
            .unwrap()
            .state
            .as_deref(),
        Some("merged")
    );
    assert_eq!(parse_pr_view("not json"), None);
}

#[test]
fn subagents_read_their_status_and_label() {
    let child = SessionSummary {
        session_id: "s_child".into(),
        title: "".into(),
        status: Some("working".into()),
        metadata: json!({ "parent_session_id": "s", "subagent_display": { "name": "Explore" } })
            .as_object()
            .cloned(),
        created_at: 10,
        updated_at: 20,
        ..SessionSummary::default()
    };
    let item = subagent_item(&child, 2);
    assert_eq!(item.status, "running");
    assert_eq!(item.display.as_deref(), Some("Explore"));
    assert_eq!(item.title, "Explore");
    assert_eq!(item.ended_at_ms, None);
    assert_eq!(item.descendants, 2);
    assert_eq!(subagent_status(Some("error")), "failed");
    assert_eq!(subagent_status(Some("idle")), "done");
    assert_eq!(subagent_status(Some("done")), "done");
}

/// The index scans only what was added since, plus the last entry again (it
/// may have been streaming): the result equals one full scan.
#[test]
fn incremental_scanning_equals_a_full_scan() {
    let streaming = say("e2", "assistant", "Working on https://a.example.com");
    let finished = say(
        "e2",
        "assistant",
        "Working on https://a.example.com, opened https://github.com/o/r/pull/1",
    );
    let call = json!({ "entry_id": "e3", "message": {
        "role": "assistant", "timestamp": 1,
        "content": [{ "type": "function_call", "id": "w", "function_id": "shell::fs::write",
                      "arguments": { "path": "/tmp/x.txt", "content": "abc" } }],
    }});
    let written = result("e4", "w", "shell::fs::write", json!([]), Value::Null);
    let first = say("e1", "user", "start");

    let mut index = SessionIndex::default();
    index.apply(&[first.clone(), streaming], false);
    assert_eq!(index.last_entry_id.as_deref(), Some("e2"));
    // Next call: from the last entry again (now finished) to the newest.
    index.apply(&[finished.clone(), call.clone()], true);
    index.apply(&[call.clone(), written.clone()], true);
    // And once more with nothing new but the last entry.
    index.apply(std::slice::from_ref(&written), true);

    let full = scan(&[first, finished, call, written]);
    assert_eq!(index.entries, full.entries);
    let (prs, links, attachments, ..) = aggregate(&index, None);
    assert_eq!(prs.len(), 1);
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].count, 1,
        "the rescanned entry is not counted twice"
    );
    assert_eq!(attachments.len(), 1);
}

#[test]
fn the_index_keeps_64_sessions() {
    let cache = ContextIndex::default();
    for i in 0..70 {
        cache.put(&format!("s{i}"), SessionIndex::default());
    }
    assert_eq!(cache.len(), 64);
    // A put refreshes recency: s6 survives the next eviction.
    cache.put(
        "s6",
        SessionIndex {
            scanned: true,
            ..SessionIndex::default()
        },
    );
    cache.put("s70", SessionIndex::default());
    assert!(cache.take("s6").scanned);
    assert_eq!(cache.len(), 64);
}

#[test]
fn hosts_and_urls() {
    assert_eq!(
        host_of("https://user@Example.com:8080/x?y").as_deref(),
        Some("example.com:8080")
    );
    assert_eq!(host_of("ftp://x"), None);
    assert_eq!(
        urls_in("(see https://a.example.com/b)."),
        ["https://a.example.com/b"]
    );
    assert_eq!(
        urls_in("\"https://a.example.com/q?x=1\";"),
        ["https://a.example.com/q?x=1"]
    );
    assert!(parse_pull_request_url("https://github.com/o/r/issues/1").is_none());
}
