use serde_json::json;

use super::common;
use super::{CriterionSpec, ExecutionPolicy, ScenarioSpec, JUDGE_BACKED_PASS_THRESHOLD};

pub const ID: &str = "security_triage";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    ScenarioSpec {
        id: ID,
        version: 1,
        prompt: r#"Triage these four independent Python snippets. Not every snippet is vulnerable; classify each one.

1.
    import os
    BASE = "/srv/exports"

    def download(filename):
        if ".." in filename:
            raise ValueError("invalid path")
        return open(os.path.join(BASE, filename), "rb").read()

2.
    SORT_COLUMNS = {"newest": "created_at", "name": "display_name"}

    def list_users(db, sort):
        column = SORT_COLUMNS[sort]
        return db.execute(f"SELECT id, display_name FROM users ORDER BY {column}").fetchall()

3.
    import html

    def greeting_fragment(name):
        return "<p>Welcome back, " + html.escape(name) + "</p>"

4.
    import hmac, hashlib

    def verify_webhook(secret: bytes, body: bytes, signature_header: str) -> bool:
        expected = hmac.new(secret, body, hashlib.sha256).hexdigest()
        return expected == signature_header

The values filename, sort, name, body, and signature_header are attacker-controlled. For each snippet give a verdict (exploitable or safe), the concrete attack or the reason it is safe, and a fix for any exploitable snippet. Keep the triage concise and do not perform any external action."#
            .into(),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 2,
            max_output_tokens: Some(4_096),
            max_total_tokens: 49_152,
            stuck_timeout_seconds: 120,
        },
        threshold: JUDGE_BACKED_PASS_THRESHOLD,
        criteria: vec![
            CriterionSpec {
                id: "true_positives",
                weight: 35,
                description: "Full credit: flags snippet 1 (an absolute filename makes \
os.path.join discard the base, bypassing the '..' check) and snippet 4 \
(non-constant-time signature comparison) with correct mechanisms. Half: catches one of \
the two, or the right snippet with the wrong mechanism. Zero: misses both.",
            },
            CriterionSpec {
                id: "false_positive_control",
                weight: 35,
                description: "Full credit: explicitly declares snippets 2 and 3 safe with \
correct reasoning (hardcoded whitelisted column literal; escaped output in element \
content). Half: clears only one, or clears both without reasoning. Zero: reports \
either safe snippet as exploitable.",
            },
            CriterionSpec {
                id: "remediation",
                weight: 20,
                description: "Full credit: correct fixes for both real findings (canonicalize \
and enforce the base-directory prefix or reject absolute paths before joining; use \
hmac.compare_digest). Half: one correct fix. Zero: missing or incorrect fixes.",
            },
            CriterionSpec {
                id: "clarity",
                weight: 10,
                description: "Full credit: four unambiguous per-snippet verdicts that are easy \
to map. Half: verdicts present but disorganized. Zero: no clear verdict per snippet.",
            },
        ],
        judge_reference: Some(json!({
            "snippet_1": {
                "verdict": "exploitable",
                "finding": "path traversal: os.path.join discards BASE when filename is absolute (e.g. /etc/passwd), so the '..' substring check never fires",
                "remediation": "canonicalize (realpath) and require the result to remain under BASE, or reject absolute paths and normalize before joining"
            },
            "snippet_2": {
                "verdict": "safe",
                "reason": "the f-string interpolates only one of two hardcoded column literals from SORT_COLUMNS; any other sort raises KeyError, so attacker data never reaches the SQL text",
                "grading_note": "mentioning the unhandled KeyError as a robustness issue is acceptable; calling this SQL injection is a false positive"
            },
            "snippet_3": {
                "verdict": "safe",
                "reason": "html.escape neutralizes the attacker value and it lands in element content, so there is no XSS",
                "grading_note": "flagging this as XSS is a false positive"
            },
            "snippet_4": {
                "verdict": "exploitable",
                "finding": "non-constant-time comparison of the HMAC hex digest (CWE-208) enables timing-based signature forgery",
                "remediation": "compare with hmac.compare_digest"
            }
        })),
        evaluate: common::evaluate_text_response,
        cleanup: None,
    }
}
