//! End to end: fetch the checkpoint from the Hub, then answer a judge
//! `EvaluateRequest` (JSON on stdin, or a built-in ticket) in-process.
use anyhow::Result;
use judge_laya::{download, LayaClient};
use serde_json::json;
use std::io::Read;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    let model = std::env::var("LAYA_MODEL").unwrap_or_else(|_| "laya".into());
    let t0 = Instant::now();
    let gguf = std::env::var_os("III_LAYA_ENCODER_GGUF").map(std::path::PathBuf::from);
    let checkpoint = download::fetch(&model, None, gguf.as_deref())?;
    eprintln!(
        "checkpoint {} @ {} fetched in {:.1}s",
        checkpoint.model,
        checkpoint.revision,
        t0.elapsed().as_secs_f32()
    );
    let client = LayaClient::load(
        std::slice::from_ref(&checkpoint),
        judge_laya::engine::Options::default(),
    )?;
    eprintln!("device {}", client.device());
    eprintln!("loaded in {:.1}s", t0.elapsed().as_secs_f32());
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let request = if input.trim().is_empty() {
        json!({
            "timeout_ms": 60000,
            "evaluations": [{
                "id": "ticket-4411",
                "state": {"from": "user@acme.com", "subject": "Duplicate charge on invoice #4411",
                          "body": "Hi, we were billed twice for March. Please refund the duplicate today or we will cancel our plan."},
                "questions": {
                    "department": {"type": "choice", "instructions": "Which department should handle this request?",
                                   "criteria": {"billing": "invoices, payments, refunds", "technical": "bugs, outages, system errors",
                                                "sales": "pricing, new contracts", "other": null}},
                    "urgency": {"type": "score", "instructions": "How urgent is this request?",
                                "criteria": ["not urgent", "soon", {"impact": "critical deadline or blocking issue"}]},
                    "churn_risk": {"type": "noul", "instructions": "Does the user threaten to cancel or leave?"},
                    "refund_requested": {"type": "noul", "instructions": "Does the user explicitly request a refund?",
                                         "criteria": {"true": "an explicit refund request"}}
                }
            }]
        })
    } else {
        serde_json::from_str(&input)?
    };
    let request = serde_json::from_value(request)?;
    let t1 = Instant::now();
    let response = client
        .with_caller_id(Some("example"))
        .evaluate(request)
        .await;
    eprintln!("evaluated in {:.2}s", t1.elapsed().as_secs_f32());
    println!("{}", serde_json::to_string_pretty(&response)?);
    let models = client.list_models(Default::default()).await;
    println!("{}", serde_json::to_string_pretty(&models)?);
    Ok(())
}
