//! The opening message, pinned.
//!
//! It is the one artefact of this worker a model reads end to end, so its
//! wording is a decision rather than an accident: the golden makes changing
//! it deliberate, and reading the diff is how a reviewer sees what the agent
//! will actually be told.

mod support;

use std::collections::BTreeMap;

use sentinel::evidence::{
    EvidenceBundleV1, EvidenceEventV1, EvidenceLogV1, EvidenceSpanV1, EvidenceTruncatedV1,
    EvidenceWorkerV1,
};
use sentinel::investigation::message::{self, MessageContext, PreviousOccurrence};
use sentinel::{GroupStatusV1, RepositoryConfigV1};

const NOW: i64 = 1_790_339_696_789;

fn attributes(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn context() -> MessageContext {
    MessageContext {
        group_id: "grp_0199f1a2b3c44d5e8f9a0b1c2d3e4f50".into(),
        title: "UnknownOperation: unknown compose operation `<str>`".into(),
        fingerprint: "6a1f0c2d9e4b7a83c5d0e1f2a3b4c5d6".into(),
        service_name: "compose".into(),
        function_id: Some("compose::operation".into()),
        status: GroupStatusV1::New,
        occurrence_count: 7,
        sessions_affected: 3,
        first_seen_ms: NOW - 86_400_000,
        last_seen_ms: NOW,
        message_sample: "unknown compose operation `<str>`".into(),
        occurrence_id: "occ_0199f1a2b3c44d5e8f9a0b1c2d3e4f51".into(),
        occurrence_at_ms: NOW,
        worker_version: Some("0.24.0".into()),
        checkout_ref: Some("git:abc1234".into()),
        repository: Some(RepositoryConfigV1 {
            id: "workers".into(),
            path: "/home/dev/workers".into(),
            workers: vec!["compose".into()],
        }),
        evidence: Some(EvidenceBundleV1 {
            version: 1,
            captured_at_ms: NOW,
            settled: true,
            trace_id: "9f2b7c1d4e5a6b8c9d0e1f2a3b4c5d6e".into(),
            origin_span_id: "s1".into(),
            propagated_through: vec!["s0".into()],
            trace_tags: attributes(&[("iii.session.id", "s_9f2b7c1d")]),
            spans: vec![
                EvidenceSpanV1 {
                    span_id: "s0".into(),
                    parent_span_id: None,
                    name: "execute harness::turn".into(),
                    service_name: "harness".into(),
                    function_id: Some("harness::turn".into()),
                    start_time_unix_nano: 1,
                    end_time_unix_nano: 9,
                    status: "error".into(),
                    status_description: Some("a child span failed".into()),
                    attributes: BTreeMap::new(),
                    events: vec![],
                    depth: 0,
                },
                EvidenceSpanV1 {
                    span_id: "s1".into(),
                    parent_span_id: Some("s0".into()),
                    name: "execute compose::operation".into(),
                    service_name: "compose".into(),
                    function_id: Some("compose::operation".into()),
                    start_time_unix_nano: 2,
                    end_time_unix_nano: 8,
                    status: "error".into(),
                    status_description: Some("unknown compose operation `up`".into()),
                    attributes: attributes(&[
                        ("function_id", "compose::operation"),
                        ("iii.namespace", "my-project"),
                    ]),
                    events: vec![EvidenceEventV1 {
                        name: "exception".into(),
                        timestamp_unix_nano: 8,
                        attributes: attributes(&[
                            ("exception.message", "unknown compose operation `up`"),
                            ("exception.type", "UnknownOperation"),
                        ]),
                    }],
                    depth: 1,
                },
                EvidenceSpanV1 {
                    span_id: "s2".into(),
                    parent_span_id: Some("s0".into()),
                    name: "call state::get".into(),
                    service_name: "state".into(),
                    function_id: Some("state::get".into()),
                    start_time_unix_nano: 3,
                    end_time_unix_nano: 4,
                    status: "error".into(),
                    status_description: Some("key not found".into()),
                    attributes: BTreeMap::new(),
                    events: vec![],
                    depth: 1,
                },
            ],
            logs: vec![EvidenceLogV1 {
                timestamp_unix_nano: 8,
                severity_text: "ERROR".into(),
                body: "compose::operation failed: unknown compose operation `up`".into(),
                span_id: Some("s1".into()),
                attributes: BTreeMap::new(),
            }],
            worker: EvidenceWorkerV1 {
                service_name: "compose".into(),
                version: Some("0.24.0".into()),
            },
            truncated: EvidenceTruncatedV1::default(),
        }),
        previous: vec![
            PreviousOccurrence {
                at_ms: NOW - 3_600_000,
                worker_version: Some("0.24.0".into()),
                message: "unknown compose operation `down`".into(),
            },
            PreviousOccurrence {
                at_ms: NOW - 7_200_000,
                worker_version: Some("0.23.1".into()),
                message: "unknown compose operation `restart`".into(),
            },
        ],
    }
}

#[test]
fn the_first_pass_message_is_what_it_was() {
    let rendered = message::first_pass(&context(), 64 * 1024);
    support::check_golden("message/first_pass.md", &rendered).expect("golden");
}

#[test]
fn the_chat_evidence_is_what_it_was() {
    let rendered = message::chat_evidence(&context(), 64 * 1024);
    assert!(
        rendered.starts_with("## Sentinel · evidence — "),
        "the console recognises the entry by this heading"
    );
    support::check_golden("message/chat_evidence.md", &rendered).expect("golden");
}

#[test]
fn a_group_with_no_mapped_checkout_says_so_instead_of_pointing_nowhere() {
    let mut context = context();
    context.repository = None;
    context.checkout_ref = None;
    let rendered = message::first_pass(&context, 64 * 1024);
    assert!(
        rendered.contains("No repository is mapped to `compose`"),
        "{rendered}"
    );
    assert!(
        rendered.contains("missing_evidence"),
        "and says what to do about it: {rendered}"
    );
}

#[test]
fn a_budget_smaller_than_the_evidence_still_leaves_a_usable_message() {
    let rendered = message::first_pass(&context(), 700);
    assert!(rendered.len() <= 700, "{} bytes", rendered.len());
    assert!(
        rendered.contains("grp_0199f1a2b3c44d5e8f9a0b1c2d3e4f50"),
        "the group is in the header, which is never cut: {rendered}"
    );
}
