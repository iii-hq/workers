//! Delivery-tracking handlers: list_pending, list_undelivered,
//! ack_delivered, consume_undelivered, flush_delivered, sweep_session.

mod common;

use approval_gate::*;
use common::{empty_policy_rules, sample_call, FailingStateBus, FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};
use std::sync::Mutex;



    #[tokio::test]
    async fn handle_list_undelivered_caps_at_default_limit_and_reports_omitted() {
        let bus = InMemoryStateBus::new();
        for i in 0..75 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 50);
        assert_eq!(resp["omitted"].as_u64(), Some(25));
    }


    #[tokio::test]
    async fn handle_list_undelivered_honors_explicit_limit() {
        let bus = InMemoryStateBus::new();
        for i in 0..10 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_list_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "limit": 3}),
            100_000,
        )
        .await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 3);
        assert_eq!(resp["omitted"].as_u64(), Some(7));
    }


    #[tokio::test]
    async fn handle_list_undelivered_returns_oldest_first_by_resolved_at() {
        let bus = InMemoryStateBus::new();
        for (i, ts) in [(0_u32, 5_000_u64), (1, 1_000), (2, 3_000)] {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                ts,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_list_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "limit": 10}),
            100_000,
        )
        .await;
        let entries = resp["entries"].as_array().unwrap();
        let ids: Vec<&str> = entries
            .iter()
            .map(|e| e["function_call_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["c1", "c2", "c0"]);
    }


    #[tokio::test]
    async fn handle_list_undelivered_omitted_is_zero_when_under_limit() {
        let bus = InMemoryStateBus::new();
        let mut rec = transition_record_with_now(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            1_500,
        );
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), rec)
            .await
            .unwrap();
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 1);
        assert_eq!(resp["omitted"].as_u64(), Some(0));
    }


    #[tokio::test]
    async fn handle_consume_undelivered_stamps_returned_entries() {
        let bus = InMemoryStateBus::new();
        for i in 0..3 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_consume_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "turn-7", "limit": 10}),
            100_000,
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["entries"].as_array().unwrap().len(), 3);
        assert_eq!(resp["omitted"].as_u64(), Some(0));
        let next =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(next["entries"].as_array().unwrap().len(), 0);
    }


    #[tokio::test]
    async fn handle_consume_undelivered_respects_limit_and_leaves_remainder() {
        let bus = InMemoryStateBus::new();
        for i in 0..5 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_consume_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "turn-7", "limit": 2}),
            100_000,
        )
        .await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 2);
        assert_eq!(resp["omitted"].as_u64(), Some(3));
        let next =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(next["entries"].as_array().unwrap().len(), 3);
    }


    #[tokio::test]
    async fn handle_consume_undelivered_missing_turn_id_returns_error() {
        let bus = InMemoryStateBus::new();
        let resp = handle_consume_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1"}),
            100_000,
        )
        .await;
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["error"], json!("missing_turn_id"));
    }


    #[tokio::test]
    async fn handle_flush_delivered_stamps_all_unacked_terminals() {
        let bus = InMemoryStateBus::new();
        for i in 0..5 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_flush_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "manual-flush"}),
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["stamped"].as_u64(), Some(5));
        let next =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(next["entries"].as_array().unwrap().len(), 0);
    }


    #[tokio::test]
    async fn handle_flush_delivered_skips_pending_records() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();
        let resp = handle_flush_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "manual-flush"}),
        )
        .await;
        assert_eq!(resp["stamped"].as_u64(), Some(0));
        let still = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(still["status"].as_str(), Some("pending"));
        assert!(still.get("delivered_in_turn_id").is_none());
    }


    #[tokio::test]
    async fn handle_flush_delivered_idempotent_on_already_stamped() {
        let bus = InMemoryStateBus::new();
        let mut rec = transition_record_with_now(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            1_500,
        );
        {
            let obj = rec.as_object_mut().unwrap();
            obj.insert(
                "delivered_in_turn_id".into(),
                Value::String("turn-prev".into()),
            );
            obj.insert("session_id".into(), Value::String("s1".into()));
        }
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), rec)
            .await
            .unwrap();
        let resp = handle_flush_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "manual-flush"}),
        )
        .await;
        assert_eq!(resp["stamped"].as_u64(), Some(0));
        let still = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(still["delivered_in_turn_id"].as_str(), Some("turn-prev"));
    }


    #[tokio::test]
    async fn handle_list_undelivered_returns_terminal_records_with_no_delivered_stamp() {
        let bus = InMemoryStateBus::new();
        let mut r1 = transition_record(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        r1.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), r1)
            .await
            .unwrap();
        let mut r2 = transition_record(
            &build_pending_record("c2", "shell::fs::write", &json!({}), 1_000, 60_000),
            "denied",
            None,
            None,
            Some(Denial::UserCorrected {
                feedback: "nope".into(),
            }),
        );
        r2.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c2"), r2)
            .await
            .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        let entries = resp["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(resp["omitted"].as_u64(), Some(0));
    }


    #[tokio::test]
    async fn handle_list_undelivered_excludes_pending_records() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 1_500).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 0);
    }


    #[tokio::test]
    async fn handle_list_undelivered_empty_session_returns_empty() {
        let bus = InMemoryStateBus::new();
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 1_500).await;
        assert_eq!(resp["entries"], json!([]));
    }


    #[tokio::test]
    async fn handle_list_undelivered_excludes_records_stamped_with_delivered_turn_id() {
        let bus = InMemoryStateBus::new();
        let mut rec = transition_record(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        {
            let obj = rec.as_object_mut().unwrap();
            obj.insert(
                "delivered_in_turn_id".into(),
                Value::String("turn-prev".into()),
            );
            obj.insert("session_id".into(), Value::String("s1".into()));
        }
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), rec)
            .await
            .unwrap();

        let mut r2 = transition_record(
            &build_pending_record("c2", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        r2.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c2"), r2)
            .await
            .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        let entries = resp["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["function_call_id"], "c2");
    }


    #[tokio::test]
    async fn handle_list_undelivered_returns_empty_when_session_id_missing() {
        let bus = InMemoryStateBus::new();
        let resp = handle_list_undelivered(&bus, STATE_SCOPE, json!({}), 1_500).await;
        assert_eq!(resp["entries"], json!([]));
    }


    #[tokio::test]
    async fn handle_ack_delivered_stamps_records_with_turn_id() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1",
                "call_ids": ["c1"],
                "turn_id": "turn-1",
            }),
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["stamped"], json!(1));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["delivered_in_turn_id"], "turn-1");
    }


    #[tokio::test]
    async fn handle_ack_delivered_is_idempotent_keeps_first_turn_id() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let _ = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["c1"], "turn_id": "turn-first",
            }),
        )
        .await;
        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["c1"], "turn_id": "turn-second",
            }),
        )
        .await;
        assert_eq!(resp["stamped"], json!(0), "second ack must not re-stamp");

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["delivered_in_turn_id"], "turn-first");
    }


    #[tokio::test]
    async fn handle_ack_delivered_skips_unknown_call_ids_silently() {
        let bus = InMemoryStateBus::new();
        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["ghost"], "turn_id": "turn-1",
            }),
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["stamped"], json!(0));
    }


    #[tokio::test]
    async fn list_pending_returns_only_pending_for_session() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();
        let mut resolved = build_pending_record("tc-2", "write", &json!({}), 0, 60_000);
        resolved["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-2"), resolved)
            .await
            .unwrap();
        bus.set(
            STATE_SCOPE,
            &pending_key("other", "tc-3"),
            build_pending_record("tc-3", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let out = handle_list_pending(&bus, STATE_SCOPE, json!({ "session_id": "s1" })).await;
        let items = out["pending"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["function_call_id"], "tc-1");
    }


    #[tokio::test]
    async fn handle_sweep_session_flips_pending_records_to_timed_out() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({"session_id": "s1"})).await;
        assert_eq!(resp["swept"], json!(1));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "timed_out");
        // sweep_session no longer stamps a reason string — timed_out is
        // self-describing per the Denial refactor.
        assert!(rec.get("denial").is_none());
        assert!(rec.get("decision_reason").is_none());
    }


    #[tokio::test]
    async fn handle_sweep_session_ignores_legacy_reason_payload_field() {
        // Old callers may still pass `reason` — approval-gate accepts the
        // payload but does not persist it. Behavior is identical to a
        // bare {session_id} payload.
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();
        let resp = handle_sweep_session(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "reason": "run_stopped"}),
        )
        .await;
        assert_eq!(resp["swept"], json!(1));
        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "timed_out");
        assert!(rec.get("denial").is_none());
    }


    #[tokio::test]
    async fn handle_sweep_session_skips_non_pending_records() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({"session_id": "s1"})).await;
        assert_eq!(resp["swept"], json!(0));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "executed");
    }


    #[tokio::test]
    async fn handle_sweep_session_returns_error_when_session_id_missing() {
        let bus = InMemoryStateBus::new();
        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({})).await;
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["error"], "missing_session_id");
        assert_eq!(resp["swept"], json!(0));
    }


    #[tokio::test]
    async fn handle_ack_delivered_returns_zero_when_only_one_field_is_empty() {
        // mutant L677: two `||` operators in the empty-field guard.
        let bus = InMemoryStateBus::new();
        // empty turn_id
        let r1 = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "turn_id": "", "call_ids": ["c"]}),
        )
        .await;
        assert_eq!(r1["stamped"], json!(0));
        // empty call_ids
        let r2 = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "turn_id": "t", "call_ids": []}),
        )
        .await;
        assert_eq!(r2["stamped"], json!(0));
        // empty session_id
        let r3 = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "", "turn_id": "t", "call_ids": ["c"]}),
        )
        .await;
        assert_eq!(r3["stamped"], json!(0));
    }


    #[tokio::test]
    async fn handle_ack_delivered_short_circuits_before_stamping_on_one_empty_field() {
        // mutant L677 — two `||` operators. If either flips to `&&`, the
        // function falls through and stamps a record even when a required
        // field is empty. Seed a record so the stamping path can be
        // observed.
        let bus = InMemoryStateBus::new();
        let terminal = transition_record(
            &build_pending_record("c", "shell::fs::write", &json!({}), 0, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        bus.set(STATE_SCOPE, &pending_key("s", "c"), terminal)
            .await
            .unwrap();

        // empty turn_id — must NOT stamp the seeded record.
        let r = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "turn_id": "", "call_ids": ["c"]}),
        )
        .await;
        assert_eq!(r["stamped"], json!(0));
        let stored = bus.get(STATE_SCOPE, &pending_key("s", "c")).await.unwrap();
        assert!(
            stored.get("delivered_in_turn_id").is_none(),
            "must not stamp when turn_id is empty; mutant would stamp"
        );

        // empty call_ids — same property.
        let r = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "turn_id": "t", "call_ids": []}),
        )
        .await;
        assert_eq!(r["stamped"], json!(0));
        let stored = bus.get(STATE_SCOPE, &pending_key("s", "c")).await.unwrap();
        assert!(
            stored.get("delivered_in_turn_id").is_none(),
            "must not stamp when call_ids is empty"
        );
    }
