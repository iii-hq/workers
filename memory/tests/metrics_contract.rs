use memory::functions::metrics_support;

use metrics_support::{
    aggregate_usage, byte_budget, canonical_usage, estimate_tokens_from_bytes, CostTotal,
    MetricsError, ReportedUsage, SizeBreakdown, StageCounts, TokenTotal, UsageStatus,
};
use serde_json::{json, Value};

fn assert_keys(value: &Value, expected: &[&str]) {
    let mut actual: Vec<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

fn usage(input: Option<u64>, cost_usd: Option<f64>) -> ReportedUsage {
    ReportedUsage::new(input, None, None, None, None, cost_usd).unwrap()
}

#[test]
fn byte_budget_preserves_default_and_saturates_instead_of_wrapping() {
    assert_eq!(byte_budget(0), 0);
    assert_eq!(byte_budget(1), 4);
    assert_eq!(byte_budget(1200), 4800);
    assert_eq!(byte_budget(u64::MAX / 4), u64::MAX - 3);
    assert_eq!(byte_budget(u64::MAX / 4 + 1), u64::MAX);
    assert_eq!(byte_budget(u64::MAX), u64::MAX);
}

#[test]
fn ascii_bytes_fit_exactly_then_exceed_the_unchanged_default_budget() {
    let exact = "a".repeat(4800);
    let too_long = "a".repeat(4801);
    assert_eq!(exact.len() as u64, byte_budget(1200));
    assert!(too_long.len() as u64 > byte_budget(1200));
    assert_eq!(estimate_tokens_from_bytes(exact.len() as u64), 1200);
    assert_eq!(estimate_tokens_from_bytes(too_long.len() as u64), 1201);
    let sizes = SizeBreakdown {
        memory_text_bytes: exact.len() as u64,
        ..SizeBreakdown::default()
    };
    assert_eq!(sizes.validate(4800), Ok(()));
}

#[test]
fn estimate_is_ceiling_of_total_bytes_even_at_u64_max() {
    for (bytes, expected) in [
        (0, 0),
        (1, 1),
        (3, 1),
        (4, 1),
        (5, 2),
        (7, 2),
        (8, 2),
        (u64::MAX - 3, 4_611_686_018_427_387_903),
        (u64::MAX - 2, 4_611_686_018_427_387_904),
        (u64::MAX - 1, 4_611_686_018_427_387_904),
        (u64::MAX, 4_611_686_018_427_387_904),
    ] {
        assert_eq!(estimate_tokens_from_bytes(bytes), expected);
    }
}

#[test]
fn utf8_partition_matches_observed_bytes_and_excludes_discarded_content() {
    // Synthetic bytes only: 2 + 4 + 3, not three bytes or provider tokens.
    let text = "é🦀漢";
    assert_eq!(text.chars().count(), 3);
    assert_eq!(text.len(), 9);
    let observed = [
        "<memory>\n- ",
        text,
        "\n- a\n</memory>",
        "\n## ",
        "regra",
        "Hoje\n",
        "\nRegras\n",
        "<update></update>",
    ]
    .concat();
    assert_eq!(observed.len(), 73);
    let sizes = SizeBreakdown {
        memory_text_bytes: 10,
        memory_line_decoration_bytes: 4,
        line_separator_bytes: 1,
        recall_wrapper_header_bytes: 19,
        rule_body_kept_bytes: 5,
        rule_formatting_marker_bytes: 4,
        ambient_header_bytes: 5,
        reminder_bytes: 8,
        rules_update_wrapper_bytes: 17,
        excluded_memory_line_bytes: 13,
        excluded_rule_body_bytes: 3,
    };
    assert_eq!(sizes.injected_bytes(), Ok(73));
    assert_eq!(sizes.excluded_bytes(), Ok(16));
    assert_eq!(sizes.validate(observed.len() as u64), Ok(()));
    assert_eq!(
        estimate_tokens_from_bytes(sizes.injected_bytes().unwrap()),
        19
    );
    assert_eq!(sizes.validate(89), Err(MetricsError::SizeMismatch));
}

#[test]
fn round_only_once_and_count_overhead_even_without_memories() {
    let sizes = SizeBreakdown {
        recall_wrapper_header_bytes: 1,
        ambient_header_bytes: 1,
        reminder_bytes: 1,
        rules_update_wrapper_bytes: 1,
        ..SizeBreakdown::default()
    };
    assert_eq!(sizes.validate(4), Ok(()));
    assert_eq!(
        estimate_tokens_from_bytes(sizes.injected_bytes().unwrap()),
        1
    );
    assert_eq!(StageCounts::default().validate(), Ok(()));
    assert_eq!(sizes.excluded_bytes(), Ok(0));
}

#[test]
fn size_partition_rejects_overflow_or_double_count_against_observation() {
    assert_eq!(SizeBreakdown::default().validate(0), Ok(()));
    let mut sizes = SizeBreakdown {
        memory_text_bytes: u64::MAX,
        ..SizeBreakdown::default()
    };
    assert_eq!(sizes.validate(u64::MAX), Ok(()));
    sizes.line_separator_bytes = 1;
    assert_eq!(
        sizes.injected_bytes(),
        Err(MetricsError::ArithmeticOverflow)
    );
    assert_eq!(
        sizes.validate(u64::MAX),
        Err(MetricsError::ArithmeticOverflow)
    );
    let excluded = SizeBreakdown {
        excluded_memory_line_bytes: u64::MAX,
        excluded_rule_body_bytes: 1,
        ..SizeBreakdown::default()
    };
    assert_eq!(
        excluded.excluded_bytes(),
        Err(MetricsError::ArithmeticOverflow)
    );
    assert_eq!(excluded.validate(0), Err(MetricsError::ArithmeticOverflow));
    let duplicated = SizeBreakdown {
        memory_text_bytes: 4,
        memory_line_decoration_bytes: 4,
        ..SizeBreakdown::default()
    };
    assert_eq!(duplicated.validate(4), Err(MetricsError::SizeMismatch));
}

#[test]
fn stages_account_for_explicit_additions_and_drops() {
    let updates = StageCounts {
        input: 2,
        added: 3,
        output: 4,
        dropped: 1,
    };
    assert_eq!(updates.validate(), Ok(()));
    let selection = StageCounts {
        input: 8,
        added: 0,
        output: 5,
        dropped: 3,
    };
    assert_eq!(selection.validate(), Ok(()));
    assert_eq!(
        StageCounts {
            input: u64::MAX,
            added: 0,
            output: u64::MAX,
            dropped: 0
        }
        .validate(),
        Ok(())
    );
}

#[test]
fn stages_reject_missing_additions_bad_drops_and_overflow() {
    for counts in [
        StageCounts {
            input: 2,
            added: 0,
            output: 4,
            dropped: 0,
        },
        StageCounts {
            input: 8,
            added: 0,
            output: 5,
            dropped: 2,
        },
        StageCounts {
            input: 2,
            added: 3,
            output: 4,
            dropped: 0,
        },
    ] {
        assert_eq!(counts.validate(), Err(MetricsError::StageMismatch));
    }
    for counts in [
        StageCounts {
            input: u64::MAX,
            added: 1,
            output: u64::MAX,
            dropped: 1,
        },
        StageCounts {
            input: u64::MAX,
            added: 0,
            output: u64::MAX,
            dropped: 1,
        },
    ] {
        assert_eq!(counts.validate(), Err(MetricsError::ArithmeticOverflow));
    }
}

#[test]
fn reported_usage_keeps_all_unknown_keys_null_not_omitted_or_zero() {
    let value = serde_json::to_value(ReportedUsage::default()).unwrap();
    assert_keys(
        &value,
        &[
            "input",
            "output",
            "cache_read",
            "cache_write",
            "reasoning",
            "cost_usd",
        ],
    );
    assert_eq!(
        value,
        json!({
            "input": null, "output": null, "cache_read": null,
            "cache_write": null, "reasoning": null, "cost_usd": null,
        })
    );
    let measured_zero =
        ReportedUsage::new(Some(0), Some(0), Some(0), Some(0), Some(0), Some(0.0)).unwrap();
    assert_eq!(
        serde_json::to_value(measured_zero).unwrap(),
        json!({
            "input": 0, "output": 0, "cache_read": 0,
            "cache_write": 0, "reasoning": 0, "cost_usd": 0.0,
        })
    );
    assert_ne!(measured_zero, ReportedUsage::default());
}

#[test]
fn usage_rejects_negative_and_nonfinite_cost_before_debug_or_serialization() {
    // Token inputs are Option<u64>: negative token counts are not representable.
    for cost in [
        -1.0,
        -f64::MIN_POSITIVE,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert_eq!(
            ReportedUsage::new(None, None, None, None, None, Some(cost)),
            Err(MetricsError::InvalidCost)
        );
    }
    assert!(ReportedUsage::new(None, None, None, None, None, Some(-0.0)).is_ok());
    assert!(ReportedUsage::new(Some(u64::MAX), None, None, None, None, Some(f64::MAX)).is_ok());
}

#[test]
fn canonical_usage_handles_absence_single_sources_and_duplicates() {
    let report = usage(Some(7), Some(0.25));
    let absent = canonical_usage(None, None);
    assert_eq!(absent.status(), UsageStatus::Absent);
    assert_eq!(absent.usage(), ReportedUsage::default());
    let top = canonical_usage(Some(report), None);
    assert_eq!(top.status(), UsageStatus::TopLevel);
    assert_eq!(top.usage(), report);
    let message = canonical_usage(None, Some(report));
    assert_eq!(message.status(), UsageStatus::Message);
    assert_eq!(message.usage(), report);
    let duplicate = canonical_usage(Some(report), Some(report));
    assert_eq!(duplicate.status(), UsageStatus::Duplicate);
    assert_eq!(duplicate.usage(), report);
    let aggregate = aggregate_usage(&[duplicate]).unwrap();
    assert_eq!(aggregate.input.total, Some(7));
    assert_eq!(aggregate.input.known_count, 1);
    assert_eq!(aggregate.cost_usd.total, Some(0.25));
    assert_eq!(aggregate.conflict_count, 0);
}

#[test]
fn conflicts_in_any_usage_field_become_unknown_instead_of_choosing_a_source() {
    let base = ReportedUsage::new(Some(1), Some(2), Some(3), Some(4), Some(5), Some(0.5)).unwrap();
    let alternatives = [
        ReportedUsage::new(Some(9), Some(2), Some(3), Some(4), Some(5), Some(0.5)),
        ReportedUsage::new(Some(1), Some(9), Some(3), Some(4), Some(5), Some(0.5)),
        ReportedUsage::new(Some(1), Some(2), Some(9), Some(4), Some(5), Some(0.5)),
        ReportedUsage::new(Some(1), Some(2), Some(3), Some(9), Some(5), Some(0.5)),
        ReportedUsage::new(Some(1), Some(2), Some(3), Some(4), Some(9), Some(0.5)),
        ReportedUsage::new(Some(1), Some(2), Some(3), Some(4), Some(5), Some(0.75)),
        ReportedUsage::new(Some(1), None, Some(3), Some(4), Some(5), Some(0.5)),
    ];
    for alternative in alternatives {
        let conflict = canonical_usage(Some(base), Some(alternative.unwrap()));
        assert_eq!(conflict.status(), UsageStatus::Conflict);
        assert_eq!(conflict.usage(), ReportedUsage::default());
        let aggregate = aggregate_usage(&[conflict]).unwrap();
        assert_eq!(aggregate.conflict_count, 1);
        assert_eq!(aggregate.input.total, None);
        assert_eq!(aggregate.cost_usd.total, None);
        assert_eq!(aggregate.input.missing_count, 1);
    }
}

#[test]
fn present_but_all_unknown_usage_remains_unknown_not_measured_zero() {
    let unknown = ReportedUsage::default();
    let canonical = canonical_usage(Some(unknown), Some(unknown));
    assert_eq!(canonical.status(), UsageStatus::Duplicate);
    let aggregate = aggregate_usage(&[canonical]).unwrap();
    assert_eq!(aggregate.input.total, None);
    assert_eq!(aggregate.input.known_count, 0);
    assert_eq!(aggregate.input.missing_count, 1);
    assert_eq!(aggregate.cost_usd.total, None);
    assert_eq!(aggregate.cost_usd.known_count, 0);
    assert_eq!(aggregate.cost_usd.missing_count, 1);
}

#[test]
fn complementary_partial_reports_are_not_silently_merged() {
    let input = usage(Some(3), None);
    let output = ReportedUsage::new(None, Some(5), None, None, None, None).unwrap();
    let conflict = canonical_usage(Some(input), Some(output));
    assert_eq!(conflict.status(), UsageStatus::Conflict);
    assert_eq!(conflict.usage(), ReportedUsage::default());
}

#[test]
fn aggregate_retains_observed_subtotals_but_incomplete_totals_are_null() {
    let reports = [
        canonical_usage(Some(usage(Some(9), None)), None),
        canonical_usage(
            None,
            Some(ReportedUsage::new(None, Some(2), None, None, None, Some(0.0)).unwrap()),
        ),
        canonical_usage(None, None),
    ];
    let aggregate = aggregate_usage(&reports).unwrap();
    assert_eq!(
        aggregate.input,
        TokenTotal {
            total: None,
            subtotal_observed: 9,
            known_count: 1,
            missing_count: 2
        }
    );
    assert_eq!(
        aggregate.output,
        TokenTotal {
            total: None,
            subtotal_observed: 2,
            known_count: 1,
            missing_count: 2
        }
    );
    assert_eq!(
        aggregate.cost_usd,
        CostTotal {
            total: None,
            subtotal_observed: 0.0,
            known_count: 1,
            missing_count: 2
        }
    );
    let value = serde_json::to_value(aggregate).unwrap();
    assert!(value["input"]["total"].is_null());
    assert!(value["cost_usd"]["total"].is_null());
    assert_eq!(value["cost_usd"]["subtotal_observed"], json!(0.0));
}

#[test]
fn conflict_makes_aggregate_incomplete_while_preserving_other_observations() {
    let known = usage(Some(7), Some(0.25));
    let conflict = canonical_usage(Some(known), Some(usage(Some(8), Some(0.25))));
    let aggregate = aggregate_usage(&[canonical_usage(Some(known), None), conflict]).unwrap();
    assert_eq!(aggregate.conflict_count, 1);
    assert_eq!(
        aggregate.input,
        TokenTotal {
            total: None,
            subtotal_observed: 7,
            known_count: 1,
            missing_count: 1,
        }
    );
    assert_eq!(
        aggregate.cost_usd,
        CostTotal {
            total: None,
            subtotal_observed: 0.25,
            known_count: 1,
            missing_count: 1,
        }
    );
}

#[test]
fn complete_aggregates_sum_each_field_once_without_cross_field_token_totals() {
    let a = ReportedUsage::new(Some(2), Some(3), Some(5), Some(7), Some(11), Some(0.25)).unwrap();
    let b =
        ReportedUsage::new(Some(13), Some(17), Some(19), Some(23), Some(29), Some(0.5)).unwrap();
    let aggregate = aggregate_usage(&[
        canonical_usage(Some(a), Some(a)),
        canonical_usage(Some(b), None),
    ])
    .unwrap();
    for (metric, expected) in [
        (aggregate.input, 15),
        (aggregate.output, 20),
        (aggregate.cache_read, 24),
        (aggregate.cache_write, 30),
        (aggregate.reasoning, 40),
    ] {
        assert_eq!(
            metric,
            TokenTotal {
                total: Some(expected),
                subtotal_observed: expected,
                known_count: 2,
                missing_count: 0
            }
        );
    }
    assert_eq!(
        aggregate.cost_usd,
        CostTotal {
            total: Some(0.75),
            subtotal_observed: 0.75,
            known_count: 2,
            missing_count: 0
        }
    );
}

#[test]
fn aggregates_distinguish_no_observations_absence_and_measured_zero() {
    let empty = aggregate_usage(&[]).unwrap();
    assert_eq!(
        empty.input,
        TokenTotal {
            total: None,
            subtotal_observed: 0,
            known_count: 0,
            missing_count: 0
        }
    );
    assert_eq!(
        empty.cost_usd,
        CostTotal {
            total: None,
            subtotal_observed: 0.0,
            known_count: 0,
            missing_count: 0
        }
    );
    let absent = aggregate_usage(&[canonical_usage(None, None)]).unwrap();
    assert_eq!(absent.cost_usd.missing_count, 1);
    assert_eq!(absent.cost_usd.total, None);
    let zero = aggregate_usage(&[canonical_usage(Some(usage(Some(0), Some(0.0))), None)]).unwrap();
    assert_eq!(zero.input.total, Some(0));
    assert_eq!(zero.cost_usd.total, Some(0.0));
    assert_eq!(zero.cost_usd.known_count, 1);
    assert_eq!(zero.cost_usd.missing_count, 0);
}

#[test]
fn aggregates_reject_integer_and_floating_point_overflow() {
    let max = canonical_usage(Some(usage(Some(u64::MAX), Some(f64::MAX))), None);
    let single = aggregate_usage(&[max]).unwrap();
    assert_eq!(single.input.total, Some(u64::MAX));
    assert_eq!(single.cost_usd.total, Some(f64::MAX));
    let token = canonical_usage(Some(usage(Some(1), None)), None);
    assert_eq!(
        aggregate_usage(&[max, token]),
        Err(MetricsError::ArithmeticOverflow)
    );
    let cost = canonical_usage(Some(usage(None, Some(f64::MAX))), None);
    assert_eq!(
        aggregate_usage(&[max, cost]),
        Err(MetricsError::ArithmeticOverflow)
    );
}

#[test]
fn serialized_payloads_have_closed_key_allowlists() {
    let sizes = serde_json::to_value(SizeBreakdown::default()).unwrap();
    assert_keys(
        &sizes,
        &[
            "memory_text_bytes",
            "memory_line_decoration_bytes",
            "line_separator_bytes",
            "recall_wrapper_header_bytes",
            "rule_body_kept_bytes",
            "rule_formatting_marker_bytes",
            "ambient_header_bytes",
            "reminder_bytes",
            "rules_update_wrapper_bytes",
            "excluded_memory_line_bytes",
            "excluded_rule_body_bytes",
        ],
    );
    assert_keys(
        &serde_json::to_value(StageCounts::default()).unwrap(),
        &["input", "added", "output", "dropped"],
    );
    for (canonical, status) in [
        (canonical_usage(None, None), "absent"),
        (
            canonical_usage(Some(usage(Some(1), None)), None),
            "top_level",
        ),
        (canonical_usage(None, Some(usage(Some(1), None))), "message"),
        (
            canonical_usage(Some(usage(Some(1), None)), Some(usage(Some(1), None))),
            "duplicate",
        ),
        (
            canonical_usage(Some(usage(Some(1), None)), Some(usage(Some(2), None))),
            "conflict",
        ),
    ] {
        let value = serde_json::to_value(canonical).unwrap();
        assert_keys(&value, &["status", "usage"]);
        assert_eq!(value["status"], json!(status));
        assert_keys(
            &value["usage"],
            &[
                "input",
                "output",
                "cache_read",
                "cache_write",
                "reasoning",
                "cost_usd",
            ],
        );
    }
    let value =
        serde_json::to_value(aggregate_usage(&[canonical_usage(None, None)]).unwrap()).unwrap();
    assert_keys(
        &value,
        &[
            "input",
            "output",
            "cache_read",
            "cache_write",
            "reasoning",
            "cost_usd",
            "conflict_count",
        ],
    );
    for field in [
        "input",
        "output",
        "cache_read",
        "cache_write",
        "reasoning",
        "cost_usd",
    ] {
        assert_keys(
            &value[field],
            &["total", "subtotal_observed", "known_count", "missing_count"],
        );
        assert!(value[field]["total"].is_null());
    }
}

#[test]
fn synthetic_canaries_cannot_enter_debug_or_payload_through_numeric_boundary() {
    let canaries = [
        "CANARIO_MEMORIA_SINTETICA_🦀",
        "CANARIO_REGRA_SINTETICA_é",
        "CANARIO_CONSULTA_SINTETICA",
        "CANARIO_ERRO_SINTETICO",
    ];
    // Only lengths cross the boundary. No real data, redactor or textual input.
    let sizes = SizeBreakdown {
        memory_text_bytes: canaries[0].len() as u64,
        rule_body_kept_bytes: canaries[1].len() as u64,
        ..SizeBreakdown::default()
    };
    let counts = StageCounts {
        input: 2,
        added: 0,
        output: 1,
        dropped: 1,
    };
    let reported = usage(Some(4), None);
    let canonical = canonical_usage(Some(reported), Some(reported));
    let aggregate = aggregate_usage(&[canonical]).unwrap();
    let payload = serde_json::to_string(&(sizes, counts, reported, canonical, &aggregate)).unwrap();
    let debug = format!("{sizes:?}{counts:?}{reported:?}{canonical:?}{aggregate:?}");
    for canary in canaries {
        assert!(!payload.contains(canary));
        assert!(!debug.contains(canary));
    }
    for (error, expected) in [
        (MetricsError::ArithmeticOverflow, "arithmetic_overflow"),
        (MetricsError::SizeMismatch, "size_mismatch"),
        (MetricsError::StageMismatch, "stage_mismatch"),
        (MetricsError::InvalidCost, "invalid_cost"),
    ] {
        assert_eq!(serde_json::to_value(error).unwrap(), json!(expected));
        for canary in canaries {
            assert!(!format!("{error:?}").contains(canary));
        }
    }
}

// Transportes temporários exercitam os handlers reais; nenhum modelo é executado.
mod productive {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use memory::{
        config::WorkerConfig,
        deps::Deps,
        hooks::PreGenerateInput,
        types::{Confidence, Memory},
    };
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex, OnceLock},
    };
    use tracing_subscriber::{layer::SubscriberExt, Layer};

    type Fields = BTreeMap<String, String>;
    static EVENTS: OnceLock<Arc<Mutex<Vec<Fields>>>> = OnceLock::new();
    struct Capture(Arc<Mutex<Vec<Fields>>>);
    struct Visitor(Fields);
    impl tracing::field::Visit for Visitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().into(), format!("{value:?}"));
        }
    }
    impl<S: tracing::Subscriber> Layer<S> for Capture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if event.metadata().target() == "memory_metrics" {
                let mut visitor = Visitor(Fields::new());
                event.record(&mut visitor);
                self.0.lock().unwrap().push(visitor.0);
            }
        }
    }
    fn events(session: &str) -> Vec<Fields> {
        EVENTS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.get("session_id").is_some_and(|id| id.contains(session)))
            .cloned()
            .collect()
    }
    fn capture() {
        EVENTS.get_or_init(|| {
            let events = Arc::new(Mutex::new(Vec::new()));
            tracing::subscriber::set_global_default(
                tracing_subscriber::registry().with(Capture(events.clone())),
            )
            .expect("único subscriber deste binário");
            events
        });
    }

    #[derive(Clone, Copy)]
    enum Mode {
        Usage,
        Absent,
        ProviderError,
        Malformed,
        TransportError,
        Timeout,
        EmptyCatalog,
        Embedded,
    }
    struct Boundary {
        deps: Arc<Deps>,
        calls: Arc<Mutex<Vec<String>>>,
        cursor: Arc<Mutex<Option<String>>>,
        server: tokio::task::JoinHandle<()>,
        _dir: tempfile::TempDir,
    }
    impl Drop for Boundary {
        fn drop(&mut self) {
            self.deps.iii.shutdown();
            self.server.abort();
        }
    }
    impl Boundary {
        async fn new(mode: Mode) -> Self {
            capture();
            let dir = tempfile::tempdir().unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            let calls = Arc::new(Mutex::new(Vec::new()));
            let recorded = calls.clone();
            let cursor = Arc::new(Mutex::new(None));
            let saved = cursor.clone();
            let (ready, wait) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
                // Exercise control frames in every productive fixture. The SDK
                // heartbeat and its Pong are WebSocket protocol, not JSON RPC.
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Ping(
                        b"metrics-control".to_vec().into(),
                    ))
                    .await
                    .unwrap();
                let _ = ready.send(());
                while let Some(Ok(frame)) = socket.next().await {
                    let text = match frame {
                        tokio_tungstenite::tungstenite::Message::Text(text)
                            if !text.trim().is_empty() =>
                        {
                            text
                        }
                        tokio_tungstenite::tungstenite::Message::Ping(payload) => {
                            if socket
                                .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                                .await
                                .is_err()
                            {
                                break;
                            }
                            continue;
                        }
                        tokio_tungstenite::tungstenite::Message::Close(_) => break,
                        _ => continue,
                    };
                    let request: Value = serde_json::from_str(text.as_ref())
                        .expect("nonempty text frame must contain fixture JSON RPC");
                    if request["type"] != "invokefunction" {
                        continue;
                    }
                    let function = request["function_id"].as_str().unwrap();
                    recorded.lock().unwrap().push(function.to_string());
                    let mut reply = json!({"type":"invocationresult", "invocation_id":request["invocation_id"],"function_id":function});
                    let data = &request["data"];
                    match function {
                        "engine::workers::register" => reply["result"] = json!({"success":true}),
                        "session::get" => {
                            reply["result"] = json!({"meta":{"session_id":data["session_id"],"kind":"user","metadata":{"memory_bank":"private"}}})
                        }
                        "state::get" => {
                            reply["result"] = saved
                                .lock()
                                .unwrap()
                                .as_ref()
                                .map_or(Value::Null, |id| json!({"last_entry_id":id}))
                        }
                        "state::set" => {
                            *saved.lock().unwrap() =
                                Some(data["value"]["last_entry_id"].as_str().unwrap().into());
                            reply["result"] = json!({"ok":true});
                        }
                        "session::messages" => {
                            reply["result"] = json!({"messages":[{
                            "entry_id":"metric-user", "message":{"role":"user", "timestamp":1760443200000_u64,
                                "content":[{"type":"text","text":"PRIVATE_TRANSCRIPT Amber instruments require copper calibration."}]}
                        }, {
                            "entry_id":"metric-end",
                            "origin":{"turn_id":format!("turn-{}",data["session_id"].as_str().unwrap()),
                                "memory_bank":"private","memory_extraction":true},
                            "message":{"role":"assistant","timestamp":1760443200000_u64,"content":[]}
                        }],"next_cursor":null})
                        }
                        "session::list" => {
                            reply["result"] = json!({"sessions":[],"next_cursor":null})
                        }
                        "router::embed" => {
                            if matches!(mode, Mode::Embedded) {
                                reply["result"] =
                                    json!({"model":"offline-vector","embeddings":[[1.0,0.0]]});
                            } else {
                                reply["error"] =
                                    json!({"code":"unavailable","message":"PRIVATE_EMBED_ERROR"});
                            }
                        }
                        "router::models::list" => {
                            reply["result"] = if matches!(mode, Mode::EmptyCatalog) {
                                json!({"models":[]})
                            } else {
                                json!({"models":[{"id":"fixture-first"}]})
                            }
                        }
                        "router::complete" => {
                            if matches!(mode, Mode::Timeout) {
                                continue;
                            }
                            if matches!(mode, Mode::TransportError) {
                                reply["error"] = json!({"code":"unavailable","message":"PRIVATE_TRANSPORT_ERROR"});
                            } else {
                                let body = if matches!(mode, Mode::Malformed) {
                                    "[{broken]"
                                } else {
                                    "[]"
                                };
                                let mut result = json!({"model":"effective-fixture","provider":"offline-fixture",
                                    "arbitrary":{"secret":"PRIVATE_ARBITRARY"},
                                    "message":{"role":"assistant", "stop_reason":"end",
                                        "content":[{"type":"text","text":body}]}});
                                if matches!(mode, Mode::Usage) {
                                    result["usage"] = json!({"input":137,"output":9,"cache_read":17,"cost_usd":null,"arbitrary":"PRIVATE_USAGE"});
                                    result["message"]["usage"] = result["usage"].clone();
                                }
                                if matches!(mode, Mode::ProviderError) {
                                    result["message"]["stop_reason"] = json!("error");
                                    result["message"]["error_kind"] = json!("PRIVATE_ERROR_KIND");
                                    result["message"]["error_message"] =
                                        json!("PRIVATE_PROVIDER_ERROR sk-not-a-real-secret");
                                }
                                reply["result"] = result;
                            }
                        }
                        other => panic!("RPC não previsto: {other}"),
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
                .unwrap()
                .unwrap();
            let deps = Arc::new(Deps {
                emitter: Arc::new(memory::events::Emitter::new(iii.clone())),
                iii,
                store: Arc::new(tokio::sync::RwLock::new(Arc::new(
                    memory::store::Store::open(dir.path().to_path_buf()).unwrap(),
                ))),
                config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig {
                    default_bank: "private".into(),
                    embeddings_enabled: true,
                    extraction_model: "fixture-requested".into(),
                    extraction_timeout_ms: 30,
                    ..WorkerConfig::default()
                }))),
            });
            Self {
                deps,
                calls,
                cursor,
                server,
                _dir: dir,
            }
        }
        async fn configure(&self, edit: impl FnOnce(&mut WorkerConfig)) {
            let mut cfg = (*self.deps.config().await).clone();
            edit(&mut cfg);
            *self.deps.config.write().await = Arc::new(cfg);
        }
        async fn hook(&self, session: &str, query: &str) -> Value {
            let input: PreGenerateInput = serde_json::from_value(json!({
                "session_id":session, "turn_id":format!("turn-{session}"), "step":0,
                "metadata":{"memory_bank":"private"},
                "recall_context":{"version":1,"session_id":session,"turn_id":format!("turn-{session}"),
                    "source":{"kind":"direct","entry_id":"entry-query"},"query":{"entry_id":"entry-query","text":query}},
                "generate":{"system_prompt":"base","messages":[{"role":"user","content":query}]}
            })).unwrap();
            serde_json::to_value(
                memory::hooks::pre_generate(&self.deps, input)
                    .await
                    .unwrap(),
            )
            .unwrap()
        }
        async fn extract(&self, session: &str) -> Result<(), String> {
            let job = serde_json::from_value(json!({"session_id":session,"turn_id":format!("turn-{session}"),"bank":"private","until":"metric-end"})).unwrap();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                memory::extract::try_run(&self.deps, &job),
            )
            .await
            .unwrap()
        }
    }
    fn fact(id: &str, text: String) -> Memory {
        Memory {
            id: id.into(),
            text,
            tags: vec![],
            entities: vec![],
            pinned: false,
            confidence: Confidence::Stated,
            corroboration: 0,
            source: None,
            created_at: 1760443200000,
            updated_at: 1760443200000,
            stated_at: Some(1760443200000),
            invalid_at: None,
            superseded_by: None,
            revision: 0,
        }
    }
    fn assert_sizes(reply: &Value) {
        let metrics = &reply["annotations"]["memory_metrics"];
        let sizes: &Value = &metrics["sizes"];
        let sum: u64 = sizes
            .as_object()
            .unwrap()
            .iter()
            .filter(|(k, _)| !k.starts_with("excluded_"))
            .map(|(_, v)| v.as_u64().unwrap())
            .sum();
        let mutations = &reply["mutations"];
        let actual = mutations["system_prompt"]
            .as_str()
            .map_or(0, |s| s.len() - "base".len())
            + mutations["append_messages"].as_array().map_or(0, |ms| {
                ms.iter()
                    .map(|m| memory::extract::content_text(m.get("content")).len())
                    .sum::<usize>()
            });
        assert_eq!(sum, actual as u64);
        assert_eq!(metrics["delivered_bytes"], sum);
        assert_eq!(metrics["estimated_tokens"], sum.div_ceil(4));
    }
    fn assert_private(session: &str) -> String {
        let found = events(session);
        assert!(!found.is_empty(), "métricas produtivas obrigatórias");
        for event in &found {
            assert_keys(
                &serde_json::to_value(event).unwrap(),
                &["session_id", "turn_id", "metrics", "message"],
            );
            assert!(event["turn_id"].contains(&format!("turn-{session}")));
        }
        let text = format!("{found:?}");
        for forbidden in [
            "PRIVATE_",
            "sk-not",
            "Amber instruments",
            "calibration",
            "system_prompt",
            "error_message",
            "arbitrary",
        ] {
            assert!(!text.contains(forbidden), "vazamento: {forbidden}");
        }
        text
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metrics_productive_hook_ascii_utf8_exact_fit_long_then_short_and_overflow() {
        for (case, filler, count) in [("ascii", "x", 4763), ("utf8", "é", 2381)] {
            let stack = Boundary::new(Mode::Absent).await;
            let short = format!(
                "Amber instruments use {}{}",
                filler.repeat(count),
                if case == "utf8" { "x" } else { "" }
            );
            // Prefix 22 bytes + payload + dated decoration 15 bytes = 4800.
            assert_eq!(short.len() + 15, 4800);
            let bank = stack
                .deps
                .store()
                .await
                .ensure_bank("private", None)
                .await
                .unwrap()
                .0;
            let mut long = fact(
                "long",
                format!(
                    "{}{}",
                    "Amber instruments use ".repeat(300),
                    "z".repeat(100)
                ),
            );
            long.pinned = true;
            bank.commit(long).await.unwrap();
            let mut short = fact("short", short);
            short.pinned = true;
            bank.commit(short).await.unwrap();
            stack
                .configure(|c| {
                    c.recall_limit = 10;
                })
                .await;
            let session = format!("metrics-{case}");
            let reply = stack.hook(&session, "What do Amber instruments use?").await;
            let m = &reply["annotations"]["memory_metrics"];
            assert_eq!(m["budget_tokens"], 1200);
            assert_eq!(m["budget_bytes"], 4800);
            assert_eq!(m["mode"], "bm25-entity");
            assert_eq!(m["selected"], 1);
            assert_eq!(m["budget_dropped"], 1);
            assert_eq!(m["candidates"], 2);
            assert_eq!(m["unsupported"], 0);
            assert_eq!(m["selection_policy_dropped"], 0);
            assert_eq!(m["sizes"]["memory_text_bytes"], 4785);
            assert_eq!(m["sizes"]["memory_line_decoration_bytes"], 15);
            assert_eq!(m["sizes"]["line_separator_bytes"], 0);
            assert!(m["duration_ms"].is_u64());
            assert!(m["sizes"]["excluded_memory_line_bytes"].as_u64().unwrap() > 4800);
            assert_eq!(reply["annotations"]["memory_ids"], json!(["short"]));
            assert_sizes(&reply);
            assert_private(&session);
            assert!(stack
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|id| id == "router::embed"));
            assert!(!stack
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|id| id == "router::complete"));
            stack.configure(|c| c.recall_budget_tokens = 1199).await;
            let below = stack
                .hook(
                    &format!("metrics-below-{case}"),
                    "What do Amber instruments use?",
                )
                .await;
            assert_eq!(below["annotations"]["memory_metrics"]["selected"], 0);
            assert_eq!(below["annotations"]["memory_metrics"]["budget_dropped"], 2);
            assert_sizes(&below);
            stack.configure(|c| c.recall_budget_tokens = u64::MAX).await;
            let reply = stack
                .hook(
                    &format!("metrics-overflow-{case}"),
                    "What do Amber instruments use?",
                )
                .await;
            assert_eq!(
                reply["annotations"]["memory_metrics"]["budget_bytes"],
                u64::MAX
            );
            assert_sizes(&reply);
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metrics_productive_hook_semantic_mode_requires_independent_support() {
        let stack = Boundary::new(Mode::Embedded).await;
        let bank = stack
            .deps
            .store()
            .await
            .ensure_bank("private", None)
            .await
            .unwrap()
            .0;
        let text = "The ledger service uses Postgres 16.";
        bank.commit(fact("supported", text.into())).await.unwrap();
        bank.set_vector("supported", "offline-vector", text, vec![1.0, 0.0])
            .await
            .unwrap();
        let unrelated = "Cedar orchards flourish near mountain rivers.";
        bank.commit(fact("unsupported", unrelated.into()))
            .await
            .unwrap();
        bank.set_vector("unsupported", "offline-vector", unrelated, vec![1.0, 0.0])
            .await
            .unwrap();
        let reply = stack
            .hook("metrics-semantic", "Which services use Postgres 16?")
            .await;
        let m = &reply["annotations"]["memory_metrics"];
        assert_eq!(m["mode"], "bm25-entity-semantic");
        assert_eq!(reply["annotations"]["memory_ids"], json!(["supported"]));
        assert!(m["unsupported"].as_u64().unwrap() >= 1);
        assert_sizes(&reply);
        assert_private("metrics-semantic");
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metrics_productive_hook_rules_overhead_exclusions_and_empty_recall() {
        let stack = Boundary::new(Mode::Absent).await;
        let bank = stack
            .deps
            .store()
            .await
            .ensure_bank("private", None)
            .await
            .unwrap()
            .0;
        bank.set_rule("a", &"é".repeat(400)).unwrap();
        bank.set_rule("b", "second omitted rule").unwrap();
        stack.configure(|c| c.max_rule_chars = 301).await;
        let reply = stack
            .hook("metrics-rules", "PRIVATE_QUERY unrelated question?")
            .await;
        assert_sizes(&reply);
        let m = &reply["annotations"]["memory_metrics"];
        assert_eq!(m["selected"], 0);
        assert_eq!(m["sizes"]["rule_body_kept_bytes"], 300);
        assert_eq!(m["sizes"]["excluded_rule_body_bytes"], 519);
        for key in [
            "ambient_header_bytes",
            "rule_formatting_marker_bytes",
            "reminder_bytes",
            "recall_wrapper_header_bytes",
        ] {
            assert!(m["sizes"][key].as_u64().unwrap() > 0);
        }
        assert_private("metrics-rules");
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metrics_productive_try_run_usage_and_absence_do_not_inherit_harness_totals() {
        for (session, mode) in [
            ("metrics-usage", Mode::Usage),
            ("metrics-absent", Mode::Absent),
        ] {
            let stack = Boundary::new(mode).await;
            stack.extract(session).await.unwrap();
            assert_eq!(stack.cursor.lock().unwrap().as_deref(), Some("metric-end"));
            let text = assert_private(session);
            assert!(text.contains("effective-fixture") && text.contains("offline-fixture"));
            assert!(text.contains("cost_usd: None"));
            if matches!(mode, Mode::Usage) {
                assert!(
                    text.contains("Duplicate")
                        && text.contains("input: Some(137)")
                        && text.contains("output: Some(9)")
                );
            } else {
                assert!(text.contains("Absent") && text.contains("input: None"));
            }
            assert!(!stack._dir.path().join("private/memories.jsonl").exists());
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metrics_productive_try_run_errors_timeout_catalog_and_explicit_fallback() {
        for (session, mode, outcome) in [
            (
                "metrics-provider-error",
                Mode::ProviderError,
                "ProviderError",
            ),
            (
                "metrics-transport-error",
                Mode::TransportError,
                "TransportError",
            ),
            ("metrics-malformed", Mode::Malformed, "InvalidReply"),
            ("metrics-timeout", Mode::Timeout, "Timeout"),
            ("metrics-no-model", Mode::EmptyCatalog, "ModelUnavailable"),
        ] {
            let stack = Boundary::new(mode).await;
            if matches!(mode, Mode::EmptyCatalog) {
                stack.configure(|c| c.extraction_model.clear()).await;
            }
            let error = stack.extract(session).await.unwrap_err();
            assert!(!error.contains("PRIVATE_") && !error.contains("sk-not"));
            assert_eq!(*stack.cursor.lock().unwrap(), None);
            assert!(assert_private(session).contains(outcome));
        }
        let stack = Boundary::new(Mode::Absent).await;
        stack.configure(|c| c.extraction_model.clear()).await;
        stack.extract("metrics-catalog-first").await.unwrap();
        let text = assert_private("metrics-catalog-first");
        assert!(text.contains("catalog_first") && text.contains("fixture-first"));
    }
}
