//! Offline certificates: no model, service, golden update or test-specific source path.
use std::collections::HashSet;
use std::sync::Mutex;

use memory_consolidate::consolidate::{
    self, BankReport, ConsolidationTransport, MemoryRow, TransportFuture,
};
use memory_consolidate::{llm, semantic};
use serde_json::{json, Value};

fn row(id: &str, text: &str, at: u64) -> MemoryRow {
    serde_json::from_value(
        json!({"id":id,"text":text,"revision":0,"created_at":at,"updated_at":at}),
    )
    .unwrap()
}

#[test]
fn proof_positive_same_fact_and_bilingual_sources_are_auditable() {
    for (keep, retire) in [
        (
            "User prefers concise answers.",
            "user prefers concise answers!",
        ),
        ("The API port is 3000", "the api port is 3000."),
        (
            "Usuário prefere respostas curtas.",
            "usuário prefere respostas curtas!",
        ),
        ("limit < 5", "limit < 5."),
        ("User prefers tea.", "Usuário prefere chá."),
        (
            "Appointment on 2026-10-05 at 9:30.",
            "Consulta em 5/10/2026 às 09:30.",
        ),
    ] {
        let proof = consolidate::detail_proof(keep, retire).expect("positive proof required");
        assert_eq!(proof.survivor_source, keep);
        assert_eq!(proof.retired_source, retire);
        assert!(!proof.survivor_atoms.is_empty());
        assert!(consolidate::verifies_detail_proof(&proof, keep, retire));
        let wire = serde_json::to_value(&proof).unwrap();
        assert_eq!(wire["version"], 1);
        assert!(wire["retired_atoms"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["rule"].is_string() && a["source"].is_string()));
    }
}

#[test]
fn proof_negative_details_operators_case_numbers_and_punctuation() {
    for (a, b) in [
        (
            "The configured expression is n",
            "The configured expression is n!",
        ),
        (
            "The exact filename is /tmp/report",
            "The exact filename is /tmp/report.",
        ),
        (
            "The exact identifier is PATH",
            "The exact identifier is Path",
        ),
        ("value n", "value n!"),
        ("limit < 5", "limit <= 5"),
        ("dose 0.05 mg", "dose 0.5 mg"),
        ("temperature -5 C", "temperature 5 C"),
        ("Use C++", "Use C#"),
        ("User prefers tea", "Usuário prefere chá à noite"),
        (
            "User prefers tea at night",
            "User prefers tea in the morning",
        ),
        ("Alice manages Bob", "Bob manages Alice"),
        (
            "Appointment on 2026-10-05 at 9:30.",
            "Consulta em 5/10/2026 às 09:31.",
        ),
        (
            "Appointment on 2026-02-30 at 9:30.",
            "Consulta em 30/2/2026 às 09:30.",
        ),
    ] {
        assert!(
            consolidate::detail_proof(a, b).is_none(),
            "must retain both: {a:?} vs {b:?}"
        );
        assert!(
            consolidate::detail_proof(b, a).is_none(),
            "must retain both directions"
        );
    }
}

#[test]
fn proof_tampering_and_stale_binding_are_rejected() {
    let a = "User prefers tea.";
    let b = "Usuário prefere chá.";
    let proof = consolidate::detail_proof(a, b).unwrap();
    assert!(!consolidate::verifies_detail_proof(
        &proof,
        a,
        "Usuário prefere café."
    ));
    let mut forged = proof.clone();
    forged.retired_atoms[0].canonical = "untrusted".into();
    assert!(!consolidate::verifies_detail_proof(&forged, a, b));
    let batch = semantic::build_batch(&[(row("new", b, 2), vec![row("old", a, 1)])]);
    let raw = semantic::parse_actions(&json!({"actions":[{"type":"merge","survivor":"c1","retired":"n1","reason":"claimed equivalence","proof":forged}]}).to_string()).unwrap();
    let verdict = semantic::validate(&raw, &batch, &HashSet::new(), 10);
    assert!(verdict.accepted.is_empty());
    assert_eq!(verdict.rejected.len(), 1);
}

#[test]
fn semantic_bilingual_proposal_needs_independent_detail_equality() {
    for (new, count) in [
        ("Usuário prefere chá.", 1),
        ("Usuário prefere chá à noite.", 0),
    ] {
        let batch = semantic::build_batch(&[(
            row("new", new, 2),
            vec![row("old", "User prefers tea.", 1)],
        )]);
        let raw = semantic::parse_actions(r#"{"actions":[{"type":"merge","survivor":"c1","retired":"n1","reason":"same source and topic; very confident"}]}"#).unwrap();
        let verdict = semantic::validate(&raw, &batch, &HashSet::new(), 10);
        assert_eq!(verdict.accepted.len(), count);
        if count == 1 {
            assert!(verdict.accepted[0].proof.is_some());
        } else {
            assert_eq!(verdict.rejected.len(), 1);
        }
    }
}

struct Boundary {
    rows: Vec<MemoryRow>,
    calls: Mutex<Vec<(String, Value)>>,
}
impl ConsolidationTransport for Boundary {
    fn invoke<'a>(&'a self, function: &'a str, payload: Value) -> TransportFuture<'a, Value> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((function.into(), payload));
            match function {
                "memory::list" => Ok(
                    json!({"memories": self.rows.iter().map(MemoryRowWire::wire).collect::<Vec<_>>(), "total":self.rows.len()}),
                ),
                "memory::supersede" => Ok(json!({"reinforced":true})),
                _ => Err(format!("unexpected call: {function}")),
            }
        })
    }
    fn judge<'a>(
        &'a self,
        _model: &'a str,
        _prompt: String,
    ) -> TransportFuture<'a, llm::JudgeReply> {
        Box::pin(async { Ok(llm::parse_reply(r#"{"merge_groups":[0],"promotions":[]}"#)) })
    }
}
trait MemoryRowWire {
    fn wire(&self) -> Value;
}
impl MemoryRowWire for MemoryRow {
    fn wire(&self) -> Value {
        json!({"id":self.id,"text":self.text,"created_at":self.created_at,"updated_at":self.updated_at,"revision":0})
    }
}

#[tokio::test]
async fn deterministic_handler_emits_exact_source_bound_certificate() {
    let boundary = Boundary {
        rows: vec![
            row("a", "User prefers concise answers.", 1),
            row("b", "user prefers concise answers!", 2),
        ],
        calls: Mutex::new(vec![]),
    };
    let (report, _) = consolidate::run_bank(&boundary, "offline", false, &mut 1).await;
    assert_eq!(report.superseded, 1);
    assert_eq!(report.detail_proofs.len(), 1);
    let calls = boundary.calls.lock().unwrap();
    let (_, payload) = calls
        .iter()
        .find(|(f, _)| f == "memory::supersede")
        .unwrap();
    assert_eq!(
        payload["expected_memory"]["text"],
        report.detail_proofs[0].retired_source
    );
    assert_eq!(
        payload["expected_superseded_by"]["text"],
        report.detail_proofs[0].survivor_source
    );
}

#[tokio::test]
async fn reorder_proposal_cannot_swap_roles_but_proves_intersective_style() {
    for (a, b, count) in [
        ("Alice manages Bob", "Bob manages Alice", 0),
        ("concise direct answers", "direct concise answers", 1),
    ] {
        let boundary = Boundary {
            rows: vec![row("a", a, 1), row("b", b, 2)],
            calls: Mutex::new(vec![]),
        };
        let mut report = BankReport {
            groups: consolidate::plan(&boundary.rows),
            ..Default::default()
        };
        let mut budget = 1;
        consolidate::run_llm_tier(
            &boundary,
            "offline",
            &boundary.rows,
            &mut report,
            &mut budget,
            &consolidate::LlmOptions {
                model: String::new(),
                promote_threshold: 0,
            },
        )
        .await;
        assert_eq!(report.superseded, count);
        assert_eq!(report.detail_proofs.len(), count);
        assert_eq!(
            boundary
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(f, _)| f == "memory::supersede")
                .count(),
            count
        );
    }
}
