use serde_json::json;

use super::common;
use super::{CriterionSpec, ExecutionPolicy, ScenarioSpec};

pub const ID: &str = "design_tradeoff";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    ScenarioSpec {
        id: ID,
        version: 2,
        prompt: "You are advising a five-engineer team that operates a payments ledger \
service on a single PostgreSQL instance. Facts: storage sits at 70% of the current \
instance's capacity, and one larger instance size (double the capacity) is still \
available from the cloud provider; data volume doubles roughly every eight months; \
about 60% of stored rows are reconciliation records older than 18 months that are read \
only during audits; regulators require strict transactional consistency for ledger \
writes; nobody on the team has operated a sharded or distributed database; leadership \
expects entry into two new markets within twelve months, which may double merchant \
count.\n\n\
The team must commit now to one of two plans: (A) shard PostgreSQL by merchant id over \
the next two quarters, or (B) stay single-node, move audit-only records to cheaper \
archival storage, take the instance upgrade when needed, and re-evaluate in twelve \
months.\n\n\
Recommend exactly one option. Justify it against the stated facts, state the real \
costs and risks of the option you recommend, and give concrete conditions under which \
the other option would become the right choice. Answer in a single reply and do not \
perform any external action."
            .into(),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 2,
            max_output_tokens: Some(4_096),
            max_total_tokens: 49_152,
            stuck_timeout_seconds: 120,
        },
        denied_functions: &[],
        criteria: vec![
            CriterionSpec {
                id: "commitment",
                weight: 30,
                description: "Full credit: exactly one option clearly recommended, stated early \
and never walked back. Half: a pick exists but is hedged, buried, or drowned in \
both-sides framing. Zero: refuses to choose, recommends both, or answers 'it depends'.",
            },
            CriterionSpec {
                id: "constraint_reasoning",
                weight: 30,
                description: "Full credit: argues from at least three stated facts with correct \
implications, including the storage-runway arithmetic (archiving plus the remaining 2x \
upgrade versus the eight-month doubling). Half: uses some facts but generically, or \
skips the arithmetic. Zero: boilerplate pros and cons untethered from the stated facts.",
            },
            CriterionSpec {
                id: "honest_costs",
                weight: 20,
                description: "Full credit: at least two concrete costs or risks of the \
recommended option, stated plainly. Half: a single cost or only vague acknowledgement. \
Zero: presents the recommended option as cost-free or lists costs only for the \
rejected option.",
            },
            CriterionSpec {
                id: "reversal_conditions",
                weight: 20,
                description: "Full credit: specific checkable triggers (metrics, thresholds, \
dates, or events) under which the other option becomes right. Half: only vague 'if \
circumstances change' conditions. Zero: no reversal conditions.",
            },
        ],
        judge_reference: Some(json!({
            "better_supported_option": "B: archive audit-only rows, use the remaining vertical upgrade, re-evaluate in twelve months",
            "key_factors": {
                "storage_runway": "archiving ~60% of rows drops usage to roughly 28% of current capacity, and one 2x upgrade remains; at an eight-month doubling rate that is roughly two years of runway, comfortably past the twelve-month re-evaluation point",
                "team_capability": "five engineers with no distributed-database experience make a two-quarter sharding migration of a regulated ledger high risk",
                "consistency": "strict transactional consistency is native on a single node; cross-shard ledger writes require two-phase commit or a redesign",
                "growth_uncertainty": "market entry may double merchant count, but doubling data volume is already priced into the doubling-rate runway"
            },
            "reversal_examples": "storage growth outpacing archiving, single-writer throughput or vacuum saturation, per-market data-residency regulation forcing physical partitioning",
            "grading_note": "an answer recommending A can still earn constraint_reasoning, honest_costs, and reversal_conditions credit if it honestly prices in team risk and consistency costs; commitment scores the clarity of the pick, not which pick"
        })),
        setup: None,
        evaluate: common::evaluate_text_response,
        cleanup: None,
    }
}
