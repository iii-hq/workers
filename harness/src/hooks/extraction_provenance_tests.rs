//! Extraction provenance tests over the in-memory SDK loopback stack.
//! Transport/registry fixtures are not engine authentication or operator ACL proof.
use super::*;

/// The transcript as the turn loop loads it for a step.
async fn transcript(stack: &Stack, session: &str) -> Vec<LoadedEntry> {
    stack
        .deps
        .session()
        .await
        .messages_strict(session)
        .await
        .unwrap()
}

async fn delivered(stack: &Stack, session: &str, id: &str, text: &str) -> TurnRecord {
    stack
        .reset_turn(session, crate::types::turn::TurnStatus::Running)
        .await;
    let message = AgentMessage::user_text(text);
    record_delivery(
        &stack.deps,
        session,
        &Delivery {
            message: &message,
            entry_id: Some(id),
            origin: None,
            channel: DeliveryChannel::Direct,
            continuation_entry_id: None,
            lineage: &crate::functions::send::TurnLineage::default(),
            caller_holds_session_lock: false,
            skills_explicit: false,
        },
    )
    .await
    .unwrap();
    stack.data.lock().unwrap().messages.entry(session.into()).or_default()
        .push(json!({"entry_id":id,"message":message,"origin":{"memory_extraction":true,"memory_bank":"forged"}}));
    let mut record = stack.record(session).await;
    record.turn_id = format!("opaque-turn-{id}");
    let entries = transcript(stack, session).await;
    record.recall_context = resolve(&stack.deps, &record, &entries).await;
    record.recall_sent = record.recall_context.is_some();
    record
}

async fn acknowledge(stack: &Stack, record: &TurnRecord, annotations: Value) {
    stack.data.lock().unwrap().memory_response =
        json!({"decision":"continue","annotations":annotations});
    let entries = transcript(stack, &record.session_id).await;
    let outcome = stack
        .deps
        .hooks
        .run_pre_generate(record, 0, None, &[], 0, &[], &entries)
        .await;
    assert!(matches!(
        outcome,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
}

async fn history(stack: &Stack, session: &str) -> Value {
    let entries = transcript(stack, session).await;
    extraction_history(&stack.deps.iii, session, &entries, 1000)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_preassistant_on_off_bank_changes_and_restart_retain_actual_ack() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut first = delivered(&stack, "s", "opaque-a", "Sensitive first request").await;
    acknowledge(&stack, &first, json!({"memory_extraction":true,"memory_bank":"bank-a","memory_layer":"derived","memory_agent":"agent-bank"})).await;
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty(),
        "no terminal boundary yet"
    );
    // Delivery redelivery must not conflict with newly attached producer proof.
    let original = AgentMessage::user_text("Sensitive first request");
    record_delivery(
        &stack.deps,
        "s",
        &Delivery {
            message: &original,
            entry_id: Some("opaque-a"),
            origin: None,
            channel: DeliveryChannel::Direct,
            continuation_entry_id: None,
            lineage: &crate::functions::send::TurnLineage::default(),
            caller_holds_session_lock: false,
            skills_explicit: false,
        },
    )
    .await
    .unwrap();
    first.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &first, true).await;
    // A repeated/out-of-order acknowledgement cannot attribute bank-b to a.
    acknowledge(
        &stack,
        &first,
        json!({"memory_extraction":true,"memory_bank":"bank-b"}),
    )
    .await;
    let mut second = delivered(&stack, "s", "opaque-b", "Second request").await;
    acknowledge(
        &stack,
        &second,
        json!({"memory_extraction":false,"memory_bank":"bank-b"}),
    )
    .await;
    second.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &second, true).await;
    finish_extraction(&stack.deps, &first, false).await;
    let h = history(&stack, "s").await;
    assert_eq!(h["rows"].as_array().unwrap().len(), 2);
    assert_eq!(h["rows"][0]["attestation"]["policy"], "on");
    assert_eq!(h["rows"][0]["attestation"]["bank"], "bank-a");
    assert_eq!(h["rows"][0]["attestation"]["agent"], "agent-bank");
    assert_eq!(
        h["rows"][0]["attestation"]["terminal"],
        json!({"status":"failed","exchange_finished":true})
    );
    assert_eq!(h["rows"][1]["attestation"]["policy"], "off");
    assert_eq!(h["rows"][1]["attestation"]["bank"], "bank-b");
    assert!(
        stack.data.lock().unwrap().messages["s"]
            .iter()
            .all(|e| e["message"]["role"] != "assistant")
    );
    let stored = stack.data.lock().unwrap().get(SCOPE, "s");
    assert!(!stored.to_string().contains("Sensitive first request"));
    let persisted_state = stack.data.lock().unwrap().state.clone();
    let persisted_messages = stack.data.lock().unwrap().messages.clone();
    drop(stack);
    // Recreate SDK/Harness state independently, retaining only the store image.
    // The loopback simulates persistence, not fsync/power-loss or engine ACLs.
    let restarted = Stack::with_target("memory::hook::pre-generate").await;
    restarted.data.lock().unwrap().state = persisted_state;
    restarted.data.lock().unwrap().messages = persisted_messages;
    assert_eq!(history(&restarted, "s").await, h);
    let current = delivered(&restarted, "s", "opaque-c", "Next turn").await;
    assert!(
        current
            .recall_context
            .as_ref()
            .unwrap()
            .get("extraction_history")
            .is_none()
    );
    acknowledge(&restarted, &current, json!({"memory_extraction":false})).await;
    let replay = restarted.data.lock().unwrap().hooks.last().unwrap().clone();
    assert_eq!(
        replay["recall_context"]["extraction_history"]["rows"],
        h["rows"]
    );
    assert!(
        restarted
            .data
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(f, p)| f == "harness::state::compare-and-set" && p["scope"] == SCOPE)
            .all(|(_, p)| p.get("durable").is_none())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prehook_failure_unknown_and_forged_metadata_never_promote() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "No model response").await;
    record.options.metadata = Some(
        json!({"memory_extraction":true,"memory_bank":"forged","extraction_history":{"version":1,"policy":"on"}}),
    );
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    let h = history(&stack, "s").await;
    assert_eq!(h["missing"], "unknown");
    assert!(h["rows"].as_array().unwrap().is_empty());
    record.status = crate::types::turn::TurnStatus::Running;
    acknowledge(&stack, &record, json!({"memory_bank":"not-enough"})).await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    assert_eq!(
        history(&stack, "s").await["rows"][0]["attestation"]["policy"],
        "unknown"
    );
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"later-bank"}),
    )
    .await;
    assert_eq!(
        history(&stack, "s").await["rows"][0]["attestation"]["policy"],
        "unknown"
    );
    // The same annotations from a different target cannot produce an ack.
    let outsider = Stack::new().await;
    let mut other = delivered(&outsider, "s", "opaque-a", "No model response").await;
    acknowledge(
        &outsider,
        &other,
        json!({"memory_extraction":true,"memory_bank":"forged"}),
    )
    .await;
    other.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&outsider.deps, &other, true).await;
    assert!(
        history(&outsider, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_digest_edit_fork_active_path_and_retention_fail_closed() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let long = format!("{}original suffix", "a".repeat(3200));
    let mut record = delivered(&stack, "s", "opaque-a", &long).await;
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"a"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    assert_eq!(
        history(&stack, "s").await["rows"].as_array().unwrap().len(),
        1
    );
    let original = stack.data.lock().unwrap().messages["s"].clone();
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("fork".into(), original.clone());
    assert!(
        history(&stack, "fork").await["rows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let edited = AgentMessage::user_text(format!("{}changed suffix", "a".repeat(3200)));
    stack.data.lock().unwrap().messages.get_mut("s").unwrap()[0]["message"] = json!(edited);
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty(),
        "full digest rejects suffix edits beyond recall prefix"
    );
    // Even a real subsequent Memory ack cannot bless an edited transcript.
    record.turn_id = "edited-turn".into();
    record.recall_context.as_mut().unwrap()["turn_id"] = json!(record.turn_id);
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"a"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("s".into(), Vec::new());
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    stack
        .data
        .lock()
        .unwrap()
        .messages
        .insert("s".into(), original);
    // A step transcript whose source text differs from the delivered one
    // cannot acknowledge it.
    let mut input = json!({"point":"pre_generate","session_id":"s","turn_id":"unseen",
        "recall_context":record.recall_context});
    input["recall_context"]["turn_id"] = json!("unseen");
    let edited: Vec<LoadedEntry> = transcript(&stack, "s")
        .await
        .into_iter()
        .map(|mut entry| {
            entry.message = Some(AgentMessage::user_text("edited source"));
            entry
        })
        .collect();
    acknowledge_extraction(
        &stack.deps.iii,
        &input,
        &json!({"decision":"continue","annotations":{"memory_extraction":true,"memory_bank":"a"}}),
        &edited,
        1000,
    )
    .await
    .unwrap();
    assert!(
        !stack
            .data
            .lock()
            .unwrap()
            .get(SCOPE, "s")
            .to_string()
            .contains("unseen")
    );
    // Eviction is visible as absence/UNKNOWN, never as next bank inheritance.
    for n in 0..MAX_RECEIPTS {
        let id = format!("opaque-retained-{n}");
        let message = AgentMessage::user_text("new");
        record_delivery(
            &stack.deps,
            "s",
            &Delivery {
                message: &message,
                entry_id: Some(&id),
                origin: None,
                channel: DeliveryChannel::Direct,
                continuation_entry_id: None,
                lineage: &crate::functions::send::TurnLineage::default(),
                caller_holds_session_lock: false,
                skills_explicit: false,
            },
        )
        .await
        .unwrap();
    }
    let h = history(&stack, "s").await;
    assert_eq!(h["retention"]["delivery_entries"], 128);
    assert_eq!(h["missing"], "unknown");
    assert!(h["rows"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_turns_on_same_source_bound_retention_and_freeze_unknown() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Repeated request").await;
    for n in 0..10 {
        record.turn_id = format!("turn-{n}");
        record.recall_context.as_mut().unwrap()["turn_id"] = json!(record.turn_id);
        record.status = crate::types::turn::TurnStatus::Running;
        acknowledge(
            &stack,
            &record,
            json!({"memory_extraction":true,"memory_bank":format!("bank-{n}")}),
        )
        .await;
        record.status = crate::types::turn::TurnStatus::Failed;
        finish_extraction(&stack.deps, &record, true).await;
    }
    let h = history(&stack, "s").await;
    assert_eq!(h["rows"].as_array().unwrap().len(), MAX_EXTRACTION_TURNS);
    assert_eq!(h["rows"][0]["attestation"]["turn_id"], "turn-2");
    assert_eq!(h["rows"][7]["attestation"]["bank"], "bank-9");
    // An envelope whose claimed session/turn does not match is not accepted.
    let forged = json!({"point":"pre_generate","session_id":"s","turn_id":"forged","recall_context":record.recall_context});
    acknowledge_extraction(
        &stack.deps.iii,
        &forged,
        &json!({"decision":"continue","annotations":{"memory_extraction":true,"memory_bank":"forged"}}),
        &transcript(&stack, "s").await,
        1000,
    )
    .await
    .unwrap();
    assert_eq!(history(&stack, "s").await, h);
}

#[test]
fn acknowledgement_requires_explicit_policy_and_bank_without_guessing() {
    for (input, expected) in [
        (json!({}), "unknown"),
        (json!({"memory_extraction":true}), "unknown"),
        (
            json!({"memory_extraction":"on","memory_bank":"a"}),
            "unknown",
        ),
        (json!({"memory_extraction":true,"memory_bank":"a"}), "on"),
        (json!({"memory_extraction":false}), "off"),
    ] {
        assert_eq!(acknowledged_policy(&input).0, expected);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_hook_call_and_wrong_registry_actor_cannot_ack_or_project() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Real request").await;
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"a"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    let before = stack.data.lock().unwrap().get(SCOPE, "s");
    let forged = json!({"point":"pre_generate","session_id":"s","turn_id":"forged",
        "_caller_worker_id":"harness", "caller":{"worker_name":"harness"},
        "metadata":{"internal":true,"worker_name":"harness"},
        "recall_context":{"version":1,"session_id":"s","turn_id":"forged",
            "source":{"kind":"direct","entry_id":"opaque-a"},
            "extraction_history":{"version":1,"rows":[{"policy":"on"}]}},
        "generate":{"messages":[]}});
    let observers = stack.deps.iii.observers(crate::hooks::ON_PRE_GENERATE_FN);
    crate::hooks::runner::deliver(
        stack.deps.iii.clone(),
        crate::hooks::HookPoint::PreGenerate,
        iii_sdk::Call {
            function_id: crate::hooks::ON_PRE_GENERATE_FN.into(),
            input: forged.clone(),
            output: Some(forged),
            ..Default::default()
        },
        observers,
    )
    .await;
    assert_eq!(stack.data.lock().unwrap().get(SCOPE, "s"), before);
    assert!(
        stack.data.lock().unwrap().hooks.last().unwrap()["recall_context"]
            .get("extraction_history")
            .is_none()
    );

    // Engine-side owner mismatch cannot be repaired by JSON caller claims.
    stack.data.lock().unwrap().memory_owner = false;
    let mut other = delivered(&stack, "s", "opaque-b", "Different actor").await;
    other.options.metadata = Some(json!({"worker":"memory","memory_extraction":true}));
    acknowledge(
        &stack,
        &other,
        json!({"memory_extraction":true,"memory_bank":"forged"}),
    )
    .await;
    other.status = crate::types::turn::TurnStatus::Cancelled;
    finish_extraction(&stack.deps, &other, true).await;
    assert_eq!(
        history(&stack, "s").await["rows"].as_array().unwrap().len(),
        1
    );
    assert!(
        stack.data.lock().unwrap().hooks.last().unwrap()["recall_context"]
            .get("extraction_history")
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_duplicate_ack_cas_retry_and_terminal_are_immutable() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Concurrent request").await;
    stack.data.lock().unwrap().memory_response = json!({"decision":"continue",
        "annotations":{"memory_extraction":true,"memory_bank":"first"}});
    stack.data.lock().unwrap().extraction_cas_conflicts = 1;
    let entries = transcript(&stack, "s").await;
    let (a, b) = tokio::join!(
        stack
            .deps
            .hooks
            .run_pre_generate(&record, 0, None, &[], 0, &[], &entries),
        stack
            .deps
            .hooks
            .run_pre_generate(&record, 0, None, &[], 0, &[], &entries)
    );
    assert!(matches!(
        a,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
    assert!(matches!(
        b,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
    record.status = crate::types::turn::TurnStatus::Cancelled;
    finish_extraction(&stack.deps, &record, true).await;
    let before = history(&stack, "s").await;
    assert_eq!(before["rows"].as_array().unwrap().len(), 1);
    assert_eq!(
        before["rows"][0]["attestation"]["terminal"]["status"],
        "cancelled"
    );
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":false,"memory_bank":"second"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Completed;
    finish_extraction(&stack.deps, &record, false).await;
    assert_eq!(history(&stack, "s").await, before);
    let stored = stack.data.lock().unwrap().get(SCOPE, "s");
    assert_eq!(stored[0]["extraction"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_purge_legacy_and_malformed_rows_never_inherit_policy() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Original request").await;
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"a"}),
    )
    .await;
    record.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &record, true).await;
    let edited = AgentMessage::user_text("Edited request");
    edit_direct(
        &stack.deps,
        "s",
        "opaque-a",
        &AgentMessage::user_text("Original request"),
        &edited,
    )
    .await;
    stack.data.lock().unwrap().messages.get_mut("s").unwrap()[0]["message"] = json!(edited);
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"b"}),
    )
    .await;
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        stack.data.lock().unwrap().get(SCOPE, "s")[0]["extraction"][0]["bank"],
        "a"
    );
    purge(&stack.deps, "s").await;
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let legacy = json!({"session_id":"s","entry_id":"opaque-a","channel":"direct",
        "query":null,"continuation":null});
    let receipt: Receipt = serde_json::from_value(legacy.clone()).unwrap();
    assert!(receipt.extraction.is_empty());
    stack
        .data
        .lock()
        .unwrap()
        .state
        .insert((SCOPE.into(), "s".into()), json!([legacy]));
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"c"}),
    )
    .await;
    assert!(
        history(&stack, "s").await["rows"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    stack.data.lock().unwrap().state.insert(
        (SCOPE.into(), "s".into()),
        json!([{"extraction":"malformed"}]),
    );
    let entries = stack
        .deps
        .session()
        .await
        .messages_strict("s")
        .await
        .unwrap();
    assert!(
        extraction_history(&stack.deps.iii, "s", &entries, 1000)
            .await
            .is_err()
    );
    // Generation still resolves its ordinary recall envelope, without proof.
    assert!(
        resolve(&stack.deps, &record, &entries)
            .await
            .unwrap()
            .get("extraction_history")
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_ack_cannot_be_copied_to_a_later_source_in_the_same_turn() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut first = delivered(&stack, "s", "opaque-first", "First source").await;
    acknowledge(
        &stack,
        &first,
        json!({"memory_extraction":true,"memory_bank":"first"}),
    )
    .await;
    let mut second = delivered(&stack, "s", "opaque-second", "Second source").await;
    second.turn_id = first.turn_id.clone();
    second.recall_context.as_mut().unwrap()["turn_id"] = json!(first.turn_id);
    acknowledge(
        &stack,
        &second,
        json!({"memory_extraction":true,"memory_bank":"second"}),
    )
    .await;
    first.status = crate::types::turn::TurnStatus::Failed;
    finish_extraction(&stack.deps, &first, true).await;
    let h = history(&stack, "s").await;
    assert_eq!(h["rows"].as_array().unwrap().len(), 1);
    assert_eq!(h["rows"][0]["attestation"]["entry_id"], "opaque-first");
    assert_eq!(h["rows"][0]["attestation"]["bank"], "first");
    // Same historical ledger, but an unrelated hook target receives no proof.
    let outsider = Stack::new().await;
    outsider.data.lock().unwrap().state = stack.data.lock().unwrap().state.clone();
    outsider.data.lock().unwrap().messages = stack.data.lock().unwrap().messages.clone();
    acknowledge(
        &outsider,
        &second,
        json!({"memory_extraction":true,"memory_bank":"forged"}),
    )
    .await;
    assert!(
        outsider.data.lock().unwrap().hooks.last().unwrap()["recall_context"]
            .get("extraction_history")
            .is_none()
    );
}

/// Per-step cost of the Memory hook: one registry lookup per turn, no
/// transcript read (the step's entries are handed in), one acknowledgement per
/// turn and no durable flush.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_memory_hook_turn_lists_once_reads_no_transcript_and_acks_once() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let record = delivered(&stack, "s", "opaque-a", "Cost request").await;
    let entries = transcript(&stack, "s").await;
    stack.data.lock().unwrap().memory_response = json!({"decision":"continue",
        "annotations":{"memory_extraction":true,"memory_bank":"a"}});
    for step in 0..2 {
        stack.data.lock().unwrap().calls.clear();
        let outcome = stack
            .deps
            .hooks
            .run_pre_generate(&record, step, None, &[], 0, &[], &entries)
            .await;
        assert!(matches!(
            outcome,
            crate::hooks::runner::PreGenerateOutcome::Continue { .. }
        ));
        let calls = stack.data.lock().unwrap().calls.clone();
        let count = |id: &str| calls.iter().filter(|(f, _)| f == id).count();
        assert_eq!(count("memory::hook::pre-generate"), 1, "step {step}");
        assert_eq!(
            count("engine::functions::list"),
            usize::from(step == 0),
            "step {step}"
        );
        assert_eq!(count("session::messages"), 0, "step {step}");
        let cas: Vec<_> = calls
            .iter()
            .filter(|(f, p)| f == "harness::state::compare-and-set" && p["scope"] == SCOPE)
            .collect();
        assert_eq!(cas.len(), usize::from(step == 0), "step {step}");
        assert!(cas.iter().all(|(_, p)| p.get("durable").is_none()));
    }
    let ledger = stack.data.lock().unwrap().get(SCOPE, "s");
    assert_eq!(ledger[0]["extraction"].as_array().unwrap().len(), 1);
}

/// A failed Memory call (a restart, or a worker taking its name over) drops the
/// ownership the turn proved; a refused ownership is never remembered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_memory_call_makes_the_next_step_ask_the_registry_again() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let record = delivered(&stack, "s", "opaque-a", "Restart request").await;
    let entries = transcript(&stack, "s").await;
    stack.data.lock().unwrap().memory_response = json!({"decision":"continue"});
    let step = |n: u64| {
        stack.data.lock().unwrap().calls.clear();
        stack
            .deps
            .hooks
            .run_pre_generate(&record, n, None, &[], 0, &[], &entries)
    };
    let lists = || {
        let calls = stack.data.lock().unwrap().calls.clone();
        calls
            .iter()
            .filter(|(f, _)| f == "engine::functions::list")
            .count()
    };
    let projected = || {
        stack.data.lock().unwrap().hooks.last().unwrap()["recall_context"]
            .get("extraction_history")
            .is_some()
    };
    step(0).await;
    assert_eq!((lists(), projected()), (1, true));
    stack.data.lock().unwrap().memory_fails = true;
    let failed = step(1).await;
    assert!(matches!(
        failed,
        crate::hooks::runner::PreGenerateOutcome::Deny(_)
    ));
    assert_eq!(lists(), 0, "still proven when the call was made");
    stack.data.lock().unwrap().memory_fails = false;
    stack.data.lock().unwrap().memory_owner = false;
    for n in 2..4 {
        step(n).await;
        assert_eq!((lists(), projected()), (1, false), "step {n}");
    }
}

/// A degraded state worker costs the Memory hook at most one more of its
/// timeouts: the history read and the acknowledgement share that bound, and
/// generation goes on without the proof.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_recall_ledger_holds_the_memory_hook_one_timeout_at_most() {
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let record = delivered(&stack, "s", "opaque-a", "Stalled request").await;
    let entries = transcript(&stack, "s").await;
    stack.data.lock().unwrap().memory_response = json!({"decision":"continue",
        "annotations":{"memory_extraction":true,"memory_bank":"a"}});
    stack.data.lock().unwrap().ledger_stalls = true;
    let started = std::time::Instant::now();
    let outcome = stack
        .deps
        .hooks
        .run_pre_generate(&record, 0, None, &[], 0, &[], &entries)
        .await;
    let held = started.elapsed();
    assert!(matches!(
        outcome,
        crate::hooks::runner::PreGenerateOutcome::Continue { .. }
    ));
    // The observer's timeout_ms is 1000; Memory itself answers at once.
    assert!(
        held < std::time::Duration::from_millis(1_500),
        "held {held:?}"
    );
    let last = stack.data.lock().unwrap().hooks.last().unwrap().clone();
    assert!(last["recall_context"].get("extraction_history").is_none());
    assert!(
        stack.data.lock().unwrap().get(SCOPE, "s")[0]["extraction"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stopped_exchange_ending_later_finishes_its_proof_boundary() {
    use crate::types::turn::TurnStatus::{Cancelled, Completed};
    let stack = Stack::with_target("memory::hook::pre-generate").await;
    let mut record = delivered(&stack, "s", "opaque-a", "Wire the run").await;
    acknowledge(
        &stack,
        &record,
        json!({"memory_extraction":true,"memory_bank":"bank-a","memory_derived":true}),
    )
    .await;
    // The turn completed while its session still waited on a wake.
    record.status = Completed;
    record.exchange_finished = false;
    crate::state::put_turn(&stack.deps.iii, &record, 1000)
        .await
        .unwrap();
    finish_extraction(&stack.deps, &record, false).await;
    let parked = history(&stack, "s").await;
    assert_eq!(
        parked["rows"][0]["attestation"]["terminal"],
        json!({"status":"completed","exchange_finished":false})
    );
    // A stop reached it, and its last wake went without a delivery.
    crate::turn_loop::finish_stopped_exchange(&stack.deps, "s", &record.turn_id).await;
    let closed = stack.record("s").await;
    assert_eq!(closed.status, Cancelled);
    assert!(closed.exchange_finished);
    let finished = history(&stack, "s").await;
    let mut expected = parked["rows"][0].clone();
    expected["attestation"]["terminal"] = json!({"status":"cancelled","exchange_finished":true});
    assert_eq!(
        finished["rows"],
        json!([expected]),
        "only the boundary moves"
    );
    // A finished boundary never moves again.
    finish_extraction(&stack.deps, &closed, false).await;
    let mut completed = closed.clone();
    completed.status = Completed;
    finish_extraction(&stack.deps, &completed, true).await;
    assert_eq!(history(&stack, "s").await, finished);
}
