use super::*;

#[test]
fn cardinal_requested_values_bind_directly_without_topic_or_owner_shortcuts() {
    let frames = [
        (
            "How many sensors does the Aurora array have?",
            "The Aurora array has ",
            " sensors.",
        ),
        (
            "How many relays does the Cobalt rack contain?",
            "The Cobalt rack contains ",
            " relays.",
        ),
        (
            "How many stewards stand at the meadow gate?",
            "",
            " stewards stand at the meadow gate.",
        ),
    ];
    let valid = [
        "0",
        "23",
        "zero",
        "two",
        "seven",
        "eleven",
        "seventeen",
        "twenty",
        "twenty-three",
        "forty two",
        "ninety-nine",
        "one hundred",
        "one hundred and seven",
        "two hundred thirty-five",
        "three thousand",
        "three thousand and twelve",
        "four thousand five hundred and six",
        "two million three thousand",
        "one billion two million three thousand and four",
    ];
    let invalid = [
        "several",
        "many",
        "some",
        "seven hours",
        "seven hundred hours",
        "seven seven",
        "twenty thirteen",
        "one hundred hundred",
        "zero hundred",
        "hundred",
        "two thousand thousand",
        "seven and",
        "seven or eight",
        "seven and eight",
        "not seven",
        "never seven",
        "would be seven",
    ];
    let mut executed = 0;
    for (query, prefix, suffix) in frames {
        for (values, expected) in [(&valid[..], true), (&invalid[..], false)] {
            for value in values {
                let text = format!("{prefix}{value}{suffix}");
                let proof =
                    crate::store::query_projection::requested_attribute(query, &text).unwrap();
                assert_eq!(proof["matched"], expected, "{query} / {text}: {proof}");
                if expected {
                    assert_eq!(proof["value"], *value);
                    let span = proof["value_bytes"].as_array().unwrap();
                    let normalized = text.to_lowercase();
                    assert_eq!(
                        &normalized[span[0].as_u64().unwrap() as usize
                            ..span[1].as_u64().unwrap() as usize],
                        *value
                    );
                    assert!(proof["source_subject_bytes"].is_array());
                    assert!(proof["source_governor_bytes"].is_array());
                    assert!(proof["source_property_bytes"].is_array());
                } else {
                    assert!(proof["value"].is_null());
                    assert!(proof["value_bytes"].is_null());
                }
                executed += 1;
            }
        }
    }
    assert_eq!(executed, 108);
}

#[tokio::test]
async fn spelled_counts_require_requested_property_owner_and_root_assertion_in_hook() {
    let cases = [
        (
            "How many people sit on the astronomy team?",
            "Seven people sit on the astronomy team.",
            "seven",
            "Seven hours sit on the astronomy team.",
            "The astronomy team sits on seven people.",
            "The astronomy team studies seven comets.",
            "Seven people sit on the visitor's astronomy team.",
        ),
        (
            "How many relays does the Cobalt rack contain?",
            "The Cobalt rack contains twenty-three relays.",
            "twenty-three",
            "The Cobalt rack contains twenty-three switches.",
            "Twenty-three relays contain the Cobalt rack.",
            "The Cobalt rack supplies twenty-three volts.",
            "The visitor's Cobalt rack contains twenty-three relays.",
        ),
        (
            "How many sensors does the Aurora array have?",
            "The Aurora array has one hundred and seven sensors.",
            "one hundred and seven",
            "The Aurora array has one hundred and seven channels.",
            "One hundred and seven sensors have the Aurora array.",
            "The Aurora array operates for one hundred and seven hours.",
            "The visitor's Aurora array has one hundred and seven sensors.",
        ),
        (
            "How many stewards stand at the meadow gate?",
            "Forty two stewards stand at the meadow gate.",
            "forty two",
            "Forty two signs stand at the meadow gate.",
            "The meadow gate stands at forty two stewards.",
            "The meadow gate opens for forty two minutes.",
            "Forty two stewards stand at the visitor's meadow gate.",
        ),
    ];
    let mut executed = 0;
    for (query, positive, value, wrong_property, inverse, missing, foreign) in cases {
        let negated = positive
            .replacen(" sit ", " never sit ", 1)
            .replacen(" stand ", " never stand ", 1)
            .replacen(" contains ", " never contains ", 1)
            .replacen(" has ", " never has ", 1);
        for (text, expected) in [
            (positive.to_owned(), true),
            (wrong_property.to_owned(), false),
            (inverse.to_owned(), false),
            (missing.to_owned(), false),
            (foreign.to_owned(), false),
            (negated, false),
            (format!("User reports: {positive}"), false),
            (format!("User quotes: \"{positive}\""), false),
            (format!("If {positive} ask the coordinator."), false),
            (positive.to_lowercase().replacen(value, "several", 1), false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let original = record("count", &text);
            bank.commit(original.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            // Direct evidence and the actual store proof must agree: topic,
            // another property or an incidental count cannot certify a value.
            let direct = crate::store::query_projection::requested_attribute(query, &text).unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let ev = evidence.get("count").expect("neutral candidate must exist");
            let proof = ev.requested_attribute.as_ref().unwrap();
            assert_eq!(*proof, direct);
            assert_eq!(proof["matched"], expected, "{query} / {text}: {proof}");
            assert_eq!(
                ev.target_scope_matched,
                ev.target_scope_required && expected,
                "store target binding: {query} / {text}"
            );
            if expected {
                assert_eq!(proof["value"], value);
                let span = proof["value_bytes"].as_array().unwrap();
                let normalized = text.to_lowercase();
                assert_eq!(
                    &normalized
                        [span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize],
                    value
                );
            } else {
                assert!(proof["value_bytes"].is_null());
            }
            let reasons = admission_reasons(&ranked, &evidence);
            assert_eq!(
                reasons.get("count").is_some_and(|r| r.supported()),
                expected,
                "before select: {query} / {text}: {reasons:?}"
            );
            println!(
                "{}",
                json!({"diagnostic":"neutral-spelled-cardinal","stage":"before_select","query":query,"text":text,"proof":proof,"expected":expected,"reasons":reasons})
            );
            let selected = select(ranked, &evidence, &HashSet::new(), 6, query);
            assert_eq!(selected.len(), usize::from(expected), "{query} / {text}");
            let response = hook(
                &deps,
                "neutral-cardinal",
                json!([{"role":"user","content":query}]),
                context("neutral-cardinal", "direct", query),
            )
            .await;
            assert_eq!(
                ids(&response),
                if expected {
                    json!(["count"])
                } else {
                    json!([])
                },
                "{query} / {text}: {response}"
            );
            rules_preserved(&response);
            assert_eq!(bank.get("count").await.unwrap(), original);
            executed += 1;
        }
    }
    assert_eq!(executed, 40);
}

#[tokio::test]
async fn requested_property_neutral_roles_values_and_missing_attributes() {
    let cases = [
        (
            "How many partitions do the Zephyr topics have?",
            "The Zephyr topics have 23 partitions.",
            "23",
            "The Zephyr topics have 23 publishers.",
            "23 partitions have the Zephyr topics.",
            "The Zephyr topics publish events.",
            "The visitor's Zephyr topics have 23 partitions.",
        ),
        (
            "How many people sit on the astronomy team?",
            "9 people sit on the astronomy team.",
            "9",
            "9 hours sit on the astronomy team.",
            "The astronomy team sits on 9 people.",
            "The astronomy team studies comets.",
            "9 people sit on the visitor's astronomy team.",
        ),
        (
            "Which hotel is booked for the Lima conference?",
            "The hotel Cypress Lodge is booked for the Lima conference.",
            "cypress lodge",
            "The flight Cypress Lodge is booked for the Lima conference.",
            "The Lima conference is booked for the hotel Cypress Lodge.",
            "The Lima conference starts on 2028-04-19.",
            "The hotel Cypress Lodge is booked for the visitor's Lima conference.",
        ),
        (
            "Which room is the acoustics workshop in?",
            "The acoustics workshop is in the room Willow.",
            "willow",
            "The acoustics workshop is in the building Willow.",
            "The room Willow is in the acoustics workshop.",
            "The acoustics workshop lasts 25 minutes.",
            "The visitor's acoustics workshop is in the room Willow.",
        ),
        (
            "How many shards do the Nimbus queues have?",
            "The Nimbus queues have 13 shards.",
            "13",
            "The Nimbus queues have 13 consumers.",
            "13 shards have the Nimbus queues.",
            "The Nimbus queues publish events.",
            "The visitor's Nimbus queues have 13 shards.",
        ),
        (
            "How many technicians work on the observatory team?",
            "17 technicians work on the observatory team.",
            "17",
            "17 hours work on the observatory team.",
            "The observatory team works on 17 technicians.",
            "The observatory team maintains antennas.",
            "17 technicians work on the visitor's observatory team.",
        ),
        (
            "Which hostel is reserved for the Quito symposium?",
            "The hostel Cedar House is reserved for the Quito symposium.",
            "cedar house",
            "The flight Cedar House is reserved for the Quito symposium.",
            "The Quito symposium is reserved for the hostel Cedar House.",
            "The Quito symposium starts on 2028-06-14.",
            "The hostel Cedar House is reserved for the visitor's Quito symposium.",
        ),
        (
            "Which hall is the optics seminar in?",
            "The optics seminar is in the hall Juniper.",
            "juniper",
            "The optics seminar is in the building Juniper.",
            "The hall Juniper is in the optics seminar.",
            "The optics seminar lasts 35 minutes.",
            "The visitor's optics seminar is in the hall Juniper.",
        ),
    ];
    let mut executed = 0;
    for (query, positive, value, wrong_property, inverse, missing, foreign) in cases {
        let negated = positive
            .replacen(" have ", " never have ", 1)
            .replacen(" sit ", " never sit ", 1)
            .replacen(" work ", " never work ", 1)
            .replacen(" is ", " is not ", 1);
        for (text, expected) in [
            (positive.to_owned(), true),
            (wrong_property.to_owned(), false),
            (inverse.to_owned(), false),
            (missing.to_owned(), false),
            (foreign.to_owned(), false),
            (negated, false),
            (format!("User reports: {positive}"), false),
            (format!("User quotes: \"{positive}\""), false),
            (format!("If {positive} ask the coordinator."), false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let deps = offline(dir.path());
            let store = deps.store().await;
            let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
            let original = record("property", &text);
            bank.commit(original.clone()).await.unwrap();
            bank.set_rule("standing", "Preserve the permanent safety rule.")
                .unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(query, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let proof = evidence
                .get("property")
                .and_then(|e| e.requested_attribute.as_ref());
            assert!(
                proof.is_some(),
                "candidate and obligation required: {query} / {text}"
            );
            let proof = proof.unwrap();
            assert_eq!(proof["matched"], expected, "{query} / {text}: {proof}");
            if expected {
                assert_eq!(proof["value"], value);
                let span = proof["value_bytes"].as_array().unwrap();
                let normalized = text.trim().to_lowercase();
                assert_eq!(
                    &normalized
                        [span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize],
                    value
                );
                assert!(proof["source_subject_bytes"].is_array());
                assert!(proof["source_governor_bytes"].is_array());
                assert!(proof["source_property_bytes"].is_array());
            } else {
                assert!(proof["value_bytes"].is_null());
            }
            let reasons = admission_reasons(&ranked, &evidence);
            assert_eq!(
                reasons.get("property").is_some_and(|r| r.supported()),
                expected,
                "before selection: {query} / {text}: {reasons:?}"
            );
            println!(
                "{}",
                json!({"diagnostic":"neutral-requested-property","stage":"before_select","query":query,"text":text,"proof":proof,"expected":expected,"reasons":reasons})
            );
            let selected = select(ranked, &evidence, &HashSet::new(), 6, query);
            assert_eq!(selected.len(), usize::from(expected), "{query} / {text}");
            let response = hook(
                &deps,
                "neutral-property",
                json!([{"role":"user","content":query}]),
                context("neutral-property", "direct", query),
            )
            .await;
            assert_eq!(
                ids(&response),
                if expected {
                    json!(["property"])
                } else {
                    json!([])
                },
                "{query} / {text}"
            );
            rules_preserved(&response);
            assert_eq!(bank.get("property").await.unwrap(), original);
            executed += 1;
        }
    }
    assert_eq!(executed, 72);
}

#[tokio::test]
async fn communication_coordinated_domain_exact_argument_and_hook() {
    let query = "Which protocol do I use for private service communication?";
    for (text, expected_argument) in [
        ("User prefers HTTP with JSON between internal services and AMQP for public endpoints.", Some("http with json")),
        ("User prefers HTTP with JSON between public endpoints and AMQP for internal services.", Some("amqp")),
        ("User uses NATS and Protobuf for private microservices.", Some("nats and protobuf")),
        ("User uses CoAP for public endpoints and NATS for private microservices.", Some("nats")),
        ("User uses NATS for private microservices and CoAP for public endpoints.", Some("nats")),
        ("User prefers HTTP with JSON between public endpoints.", None),
        ("User prefers HTTP with JSON between public endpoints and AMQP.", None),
        ("User prefers HTTP with JSON between public endpoints and not AMQP for internal services.", None),
        ("User never uses NATS for private microservices.", None),
        ("User's colleague uses NATS for private microservices.", None),
        ("NATS uses user for private microservices.", None),
        ("User reports that user uses NATS for private microservices.", None),
        ("User quotes: \"User uses NATS for private microservices.\"", None),
        ("If user uses NATS for private microservices, ask the visitor.", None),
        ("User prefers HTTP between public endpoints and NATS for private microservices and CoAP for internal services.", None),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline(dir.path());
        let store = deps.store().await;
        let (bank, _) = store.ensure_bank("hook-relevance", None).await.unwrap();
        bank.commit(attested("wire", text)).await.unwrap();
        bank.set_rule("standing", "Preserve the permanent safety rule.").unwrap();
        let (ranked, evidence) = bank.recall_evidence(query, None, "", usize::MAX, 30, false, 0.0).await;
        let proof = evidence.get("wire").and_then(|e| e.technical_practice.as_ref());
        assert_eq!(proof.is_some(), expected_argument.is_some(), "{text}: {proof:?}");
        if let Some(argument) = expected_argument {
            let proof = proof.unwrap();
            assert_eq!(proof["argument"], argument, "{text}");
            let span = proof["argument_bytes"].as_array().unwrap();
            let normalized = text.trim().to_lowercase();
            assert_eq!(&normalized[span[0].as_u64().unwrap() as usize..span[1].as_u64().unwrap() as usize], argument);
            assert!(!argument.contains("public"));
            assert!(proof["domain_bytes"].is_array());
        }
        let reasons = admission_reasons(&ranked, &evidence);
        assert_eq!(reasons.get("wire").is_some_and(|r| r.supported()), expected_argument.is_some(), "before select: {text}");
        println!("{}", json!({"diagnostic":"coordinated-communication","stage":"before_select","text":text,"proof":proof,"reasons":reasons}));
        let selected = select(ranked, &evidence, &HashSet::new(), 6, query);
        assert_eq!(selected.len(), usize::from(expected_argument.is_some()), "{text}");
        let response = hook(&deps, "wire-domain", json!([{"role":"user","content":query}]), context("wire-domain", "direct", query)).await;
        assert_eq!(ids(&response), if expected_argument.is_some() { json!(["wire"]) } else { json!([]) }, "{text}");
        rules_preserved(&response);
    }
}
