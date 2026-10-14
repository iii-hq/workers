//! Live judge eval for the semantic consolidation tier (MOT-5384): the real
//! `SEMANTIC_SYSTEM` prompt, batch builder, parser and validator over cases
//! built from the memory worker's recall golden set
//! (`../memory/tests/recall_quality/golden.json`):
//!
//! - supersede: the golden `upd-*` old/new pairs and the rescheduled
//!   dentist — the outdated record must retire, the current one survive;
//! - merge: the golden `dup_of` paraphrase pairs (EN, PT, cross-language);
//! - keep: near-misses on one topic but distinct facets — no action.
//!
//! **Harmful** actions — retiring the current record, touching a keep case,
//! retiring a distractor — must be 0 (Hindsight's "trap", applied to
//! consolidation). Ignored: it needs a live engine with a router.
//!
//! ```text
//! III_NAMESPACE=<ns> cargo test --test judge_golden -- --ignored --nocapture
//! UPDATE_CONSOLIDATION=1 ...   # also record memory/tests/recall_quality/consolidation.json
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use iii_sdk::protocol::TriggerRequest;
use memory_consolidate::consolidate::MemoryRow;
use memory_consolidate::semantic::{
    build_batch, classify, parse_actions, validate, ActionKind, Outcome, SEMANTIC_SYSTEM,
};
use serde::Deserialize;
use serde_json::{json, Value};

const DAY_MS: u64 = 86_400_000;
const DEFAULT_MODEL: &str = "claude-sonnet-5-5";

#[derive(Deserialize)]
struct Golden {
    memories: Vec<GoldenMemory>,
}

#[derive(Deserialize)]
struct GoldenMemory {
    key: String,
    text: String,
    #[serde(default)]
    pinned: bool,
    age_days: u64,
    #[serde(default)]
    dup_of: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Expect {
    /// `pending` replaces `other`: `other` must retire.
    Supersede,
    /// Same fact restated: either may retire, the other survives.
    Merge,
    /// Distinct facets: nothing may retire.
    Keep,
}

struct Case {
    expect: Expect,
    /// The "new" memory being judged.
    pending: String,
    /// Its counterpart among the candidates.
    other: String,
}

/// Unrelated candidates every case also sees (the judge must leave them).
const DISTRACTORS: [&str; 2] = ["id-gata", "pt-life-tabuleiro-wingspan"];

fn cases(golden: &Golden) -> Vec<Case> {
    let mut out: Vec<Case> = ["city", "tz", "ci", "run", "diet", "manager", "editor"]
        .iter()
        .map(|t| Case {
            expect: Expect::Supersede,
            pending: format!("upd-{t}-new"),
            other: format!("upd-{t}-old"),
        })
        .collect();
    out.push(Case {
        expect: Expect::Supersede,
        pending: "evt-dentist-moved".into(),
        other: "evt-dentist-oct15".into(),
    });
    for m in &golden.memories {
        if let Some(of) = &m.dup_of {
            out.push(Case {
                expect: Expect::Merge,
                pending: m.key.clone(),
                other: of.clone(),
            });
        }
    }
    for (pending, other) in [
        ("pt-life-alergia-dipirona", "pt-life-alergia-amendoim"),
        ("pt-life-julia-videochamada", "pt-life-julia-bh"),
        ("en-work-rotafy-mongodb", "en-work-ledgerly-postgres"),
        ("pt-life-cha-camomila", "pt-life-cafe-v60"),
        ("en-work-go-tools", "en-work-kotlin-spring"),
        ("en-work-small-prs", "en-work-review-two-approvals"),
    ] {
        out.push(Case {
            expect: Expect::Keep,
            pending: pending.into(),
            other: other.into(),
        });
    }
    out
}

fn row(m: &GoldenMemory, now: u64) -> MemoryRow {
    let at = now - m.age_days * DAY_MS;
    serde_json::from_value(json!({
        "id": m.key, "text": m.text, "pinned": m.pinned,
        "corroboration": 0, "created_at": at, "updated_at": at,
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine with a router; judges golden consolidation cases"]
async fn judge_golden_cases() {
    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../memory/tests/recall_quality/golden.json");
    let golden: Golden =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).unwrap()).unwrap();
    let by_key: HashMap<&str, &GoldenMemory> = golden
        .memories
        .iter()
        .map(|m| (m.key.as_str(), m))
        .collect();
    let model = std::env::var("JUDGE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
    let now = 1_792_000_000_000; // fixed "today" so ages are stable across runs

    let mut tally: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut harmful: Vec<String> = Vec::new();
    let mut decisions: Vec<Value> = Vec::new();
    let mut report = String::new();
    for case in cases(&golden) {
        let pending = row(by_key[case.pending.as_str()], now);
        let mut candidates = vec![row(by_key[case.other.as_str()], now)];
        candidates.extend(DISTRACTORS.iter().map(|k| row(by_key[k], now)));
        let batch = build_batch(&[(pending, candidates)]);
        let reply = iii
            .trigger(TriggerRequest {
                function_id: "router::complete".into(),
                payload: json!({
                    "model": model,
                    "system_prompt": SEMANTIC_SYSTEM,
                    "messages": [{"role": "user", "content": [{"type": "text", "text": batch.prompt}], "timestamp": 0}],
                    "max_output_tokens": 2048,
                }),
                action: None,
                timeout_ms: Some(120_000),
            })
            .await
            .map_err(|e| e.to_string());
        let actions = match classify(&reply) {
            Outcome::Reply(text) => {
                parse_actions(&text).unwrap_or_else(|e| panic!("{}: {e}\n{text}", case.pending))
            }
            other => panic!("{}: {other:?}", case.pending),
        };
        let verdict = validate(&actions, &batch, &HashSet::new(), usize::MAX);
        let retired: Vec<(&str, &str, ActionKind)> = verdict
            .accepted
            .iter()
            .map(|a| (a.retire.id.as_str(), a.keep.id.as_str(), a.kind))
            .collect();
        let retires = |k: &str| retired.iter().any(|(r, _, _)| *r == k);
        let ok = match case.expect {
            Expect::Supersede => retires(&case.other) && !retires(&case.pending),
            Expect::Merge => retires(&case.other) ^ retires(&case.pending),
            Expect::Keep => retired.is_empty(),
        };
        for (r, k, kind) in &retired {
            let bad = DISTRACTORS.contains(r)
                || case.expect == Expect::Keep
                || (case.expect == Expect::Supersede && *r == case.pending.as_str());
            if bad {
                harmful.push(format!("{} retired {r} for {k} ({kind:?})", case.pending));
            }
            decisions.push(json!({
                "kind": if *kind == ActionKind::Merge { "merge" } else { "supersede" },
                "keep": k, "retire": r,
                "reason": verdict.accepted.iter().find(|a| a.retire.id == *r).map(|a| a.reason.clone()),
            }));
        }
        let label = match case.expect {
            Expect::Supersede => "supersede",
            Expect::Merge => "merge",
            Expect::Keep => "keep",
        };
        let t = tally.entry(label).or_default();
        t.0 += 1;
        t.1 += usize::from(ok);
        report.push_str(&format!(
            "{} {:<9} {:<28} -> {:?} rejected={:?}\n",
            if ok { "ok  " } else { "MISS" },
            label,
            case.pending,
            retired,
            verdict
                .rejected
                .iter()
                .map(|(_, why)| *why)
                .collect::<Vec<_>>()
        ));
    }
    for (label, (n, ok)) in &tally {
        report.push_str(&format!("{label:<9} {ok}/{n}\n"));
    }
    report.push_str(&format!("harmful: {}\n", harmful.len()));
    println!("model {model}\n{report}");
    for h in &harmful {
        println!("HARMFUL {h}");
    }

    assert!(
        harmful.is_empty(),
        "harmful consolidation actions: {harmful:?}"
    );
    // Recorded only after the harmful check: a bad run never becomes the
    // recall eval's consolidation fixture.
    if std::env::var("UPDATE_CONSOLIDATION").is_ok() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../memory/tests/recall_quality/consolidation.json");
        let doc = json!({ "model": model, "decisions": decisions });
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
        println!("wrote {}", path.display());
    }
}
