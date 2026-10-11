//! Every model-facing string of the CLM strategy. Written for this worker
//! from the behaviour list in the design spec (§6); none of it is taken from
//! the reference harness. Unit tests pin the structural parts other modules
//! and the model rely on (readout pattern, header examples, receipt tag).

use crate::controller::py_round;

/// Prefix of every edit receipt appended to a tool result.
pub const RECEIPT_TAG: &str = "[context file:";

/// Rendering markers used by the mirror (see `projection`).
pub const EMPTY_TURN: &str = "(no content)";
pub const THINKING_TAG: &str = "[reasoning]";
pub const REDACTED_TAG: &str = "[reasoning, redacted]";
pub const IMAGE_TAG: &str = "[image]";
/// Replaces an assistant turn whose only content was stripped reasoning.
pub const THOUGHT_PLACEHOLDER: &str = "(reasoning removed)";
/// Body of a command that printed nothing.
pub const NO_OUTPUT: &str = "(the command printed nothing)";

/// The text the harness shows when the strategy ends a run
/// (`applied.strategy_stop`). Read by the user, not by the model.
pub const STRATEGY_STOP_REASON: &str = "context-clm ended the run: the context was still over its budget even after the last allowed turn. Raise `context_budget_tokens` or remove `context_strategy` from the session metadata to continue.";

const CLM_SECTION: &str = r#"

## Your context file

Your context is limited to {limit} tokens. Every command result ends with a readout such as `[context: ~1200/{limit} tokens]`.

Before each step, everything after the system prompt and the task is written to `{mirror}`. Whatever that file holds when the next step starts becomes your context from then on. The system prompt and the task are fixed and are not part of the file.

Layout: every turn starts with a header line such as `[[CTX_TURN 3 role=tool]]` (turn number, then role: user, assistant or tool), followed by its text. A turn whose text you delete (header left in place) disappears from your context. Never delete or alter the header line of a turn you want to keep. Changes apply on the next step, and the next result tells you whether your edit was applied.

Change the file with a short script instead of typing its text back out. There is no need to print the file either: you are already reading its contents as your context. `shell::exec` does not start a shell: send a script as {"command":"bash","args":["-c","<script>"]}, which also makes heredocs work. For example, to replace the text of turn 4 with a note:

    python3 - <<'EOF'
    import re
    path = "{mirror}"
    text = open(path).read()
    note = "[turn 4 summary: tests/api.rs fails on a missing header; the fix belongs in src/http.rs]"
    head = r"(\[\[CTX_TURN 4 role=\w+\]\]\n).*?(?=\n+\[\[CTX_TURN |\Z)"
    text = re.sub(head, lambda m: m.group(1) + note, text, flags=re.S)
    open(path, "w").write(text)
    EOF

Editing has a cost: every turn from the first one you change down to the end has to be processed again. So:
- group your changes into one larger edit instead of many small ones;
- favour condensing recent turns; touching an early turn means re-reading everything after it;
- make summaries thorough; they are cheap compared with the text they stand in for.

Never empty the whole file. A summary keeps the facts you found, what you already tried or ruled out, and your next step.
"#;

/// The section appended to the system prompt. Starts with a blank line so it
/// can be appended as-is; byte-stable for a given mirror path and limit.
pub fn clm_section(mirror_rel: &str, limit: u64) -> String {
    CLM_SECTION
        .replace("{mirror}", mirror_rel)
        .replace("{limit}", &limit.to_string())
}

/// How to condense; appended to the nudges, the ledger and the readout.
pub fn compaction_hint(mirror_rel: &str) -> String {
    format!(
        "To condense {mirror_rel}: locate stale turns from a script (their header line, e.g. `[[CTX_TURN 5 role=tool]]`, or a short line unique to where they begin and end) and swap them for summaries, without typing out the text being removed. Condense completed work; keep anything you still rely on."
    )
}

const LAST_TURN: &str = "Crossing the limit leaves exactly one more turn before the run stops.";

const CARRY_OVER: &str = "A summary that replaces old turns must carry over what they established: options you eliminated and why (so you do not try them again), commands and queries you already ran, important values and whether you confirmed them, and what you will do next.";

/// The one-shot nudge for a crossed tier (`frac` of the budget).
pub fn tier_text(frac: f64, budget: u64, tokens: u64, hint: &str) -> String {
    let pct = py_round(frac * 100.0);
    let at = format!("Context use: {tokens} tokens, about {pct}% of the {budget}-token budget.");
    if frac <= 0.25 {
        format!("[context note] {at} No action needed; keep track, in your notes, of which searches and files you have already checked.")
    } else if frac <= 0.5 {
        format!("[context note] {at} Wrap up the current piece of work, then do a single condensing pass. {CARRY_OVER} {hint}")
    } else {
        format!("[context warning] {at} The limit is near. Condense work that is finished, yet keep its substance: a replacement that throws away nearly everything tends to force the same work again. {CARRY_OVER} {LAST_TURN} {hint}")
    }
}

/// The repeating nudge once the room left is too small.
pub fn persistent_text(tokens: u64, limit: u64, ratio: f64, hint: &str) -> String {
    let pct = py_round(ratio * 100.0);
    format!("[context warning: act now] {tokens}/{limit} tokens used, beyond {pct}% of the hard limit. Do nothing this turn except condense your context: cut or summarise stale turns. {LAST_TURN} {hint}")
}

/// The last turn granted after the limit is crossed.
pub fn final_text(tokens: u64, budget: u64) -> String {
    format!("[context limit reached: {tokens}/{budget} tokens] One turn remains, then the run stops. Use it to finish what you can, save results where the task expects them, and say briefly what is done and what is still open.")
}

/// Inputs of the rollback ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerArgs {
    pub retry: u32,
    pub max_retry: u32,
    pub budget: u64,
    pub n_dropped: usize,
    pub n_rolled: usize,
    pub tokens_after: u64,
    pub limit: u64,
    pub hint: String,
    pub cmds: Vec<String>,
}

/// The note that replaces rolled-back turns.
pub fn ledger_text(a: &LedgerArgs) -> String {
    let mut s = format!(
        "[context limit exceeded, rollback {}/{}] Context grew past the {}-token limit. Your newest {} turn(s) were taken out and are gone for good ({} in total during this run). Current size: ~{}/{} tokens. Spend this turn condensing your context and nothing else: swap stale turns for brief summaries. {}",
        a.retry, a.max_retry, a.limit, a.n_dropped, a.n_rolled, a.tokens_after, a.limit, a.hint
    );
    if !a.cmds.is_empty() {
        s.push_str("\nThese commands produced the output that overflowed, which is why their turns were removed. Repeating them as-is will overflow again; if one is still required, rerun it with less output (head, grep -c, wc -l):");
        for c in &a.cmds {
            s.push_str("\n  - ");
            s.push_str(c);
        }
    }
    s
}

/// The size readout appended to the newest tool result.
pub fn readout_text(tokens_now: u64, limit: u64, over_hint: Option<&str>) -> String {
    let over = match (tokens_now > limit, over_hint) {
        (false, _) => String::new(),
        (true, Some(h)) => format!(", over the limit: condense {h} now"),
        (true, None) => ", over the limit".to_string(),
    };
    format!("\n[context: ~{tokens_now}/{limit} tokens{over}]")
}

pub fn receipt_grew(before: u64, after: u64) -> String {
    format!("\n{RECEIPT_TAG} edit kept, but it made the context larger (~{before} -> ~{after} tokens); it is still under the limit. To shrink it, replace old text rather than adding new text.]")
}

pub fn receipt_still_over(before: u64, after: u64, limit: u64) -> String {
    format!("\n{RECEIPT_TAG} edit kept (~{before} -> ~{after} tokens), yet the context is above the limit of ~{limit} tokens. Cut more right away; otherwise only one more turn is left before the run stops.]")
}

pub fn receipt_applied(before: u64, after: u64, n_turns: usize) -> String {
    format!("\n{RECEIPT_TAG} edit kept; ~{before} -> ~{after} tokens across {n_turns} turns]")
}

pub fn receipt_rejected(baseline: u64, after: u64, before: u64, fit: bool, limit: u64) -> String {
    let rule = if fit && limit > 0 {
        format!("Edits may grow the context only while it remains within {limit} tokens")
    } else {
        "Edits must reduce the context".to_string()
    };
    format!("\n{RECEIPT_TAG} edit discarded because it enlarged the context (~{baseline} -> ~{after} tokens); the context stays at ~{before}. {rule}. Swap old text for a more compact summary rather than appending.]")
}

pub fn receipt_no_change_exit(rc: i64, before: u64) -> String {
    format!("\n{RECEIPT_TAG} no edit; the command failed with exit code {rc} (error shown above) and the context is unchanged at ~{before} tokens.]")
}

pub fn receipt_no_change_matched(before: u64) -> String {
    format!("\n{RECEIPT_TAG} no edit; nothing in the file matched, so the context is unchanged at ~{before} tokens. Target text you can actually see, or a real header line such as `[[CTX_TURN 8 role=tool]]` (number, then role).]")
}

/// Leads a tool output that was cut for length.
pub fn too_long_hint() -> &'static str {
    "That command printed more than fits here. Next time, narrow it down first: pipe through head or tail, print a line range with sed -n, tighten the grep or find pattern, or send the output to a file and grep it."
}

/// A long tool output cut to its head and tail.
pub fn observation_cut(head: &str, elided: usize, tail: &str) -> String {
    format!(
        "{}\n\n[first {} chars]\n{head}\n[{elided} chars omitted]\n[last {} chars]\n{tail}",
        too_long_hint(),
        head.chars().count(),
        tail.chars().count()
    )
}

/// Marks the middle cut out of the newest output to fit the room left.
pub fn cap_note(elided: usize, budget: u64) -> String {
    format!("[... {elided} characters removed here so the output fits within {budget} tokens of context; use a more selective command, or free space by condensing your context file ...]")
}

/// One function call as shown in the mirror.
pub fn call_line(function_id: &str, arguments: &str) -> String {
    format!("[call {function_id} {arguments}]")
}

pub fn file_tag(name: &str) -> String {
    format!("[file {name}]")
}

/// A turn the model rewrote under a role it may not author (tool, or
/// assistant where no assistant turn was) becomes a user note.
pub fn lowered_note(role: &str, body: &str) -> String {
    let article = if role.starts_with('a') { "an" } else { "a" };
    format!("[your note, written as {article} {role} turn]\n{body}")
}

/// A trailing assistant message, delivered on the user side because the
/// provider cannot continue an assistant turn.
pub fn user_side_assistant(text: &str) -> String {
    format!("[Repeated here as a user message because the API cannot resume an assistant message. What follows is your own last reply, unchanged:]\n{text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = ".clm/s1.ctx";

    #[test]
    fn readout_has_the_pinned_pattern() {
        assert_eq!(readout_text(100, 200, None), "\n[context: ~100/200 tokens]");
        assert_eq!(
            readout_text(200, 200, Some(M)),
            "\n[context: ~200/200 tokens]"
        );
        let over = readout_text(250, 200, Some(M));
        assert!(over.starts_with("\n[context: ~250/200 tokens") && over.ends_with(']'));
        assert!(over.contains(M));
        assert_eq!(
            readout_text(250, 200, None),
            "\n[context: ~250/200 tokens, over the limit]"
        );
    }

    #[test]
    fn system_section_names_the_mirror_and_limit_and_is_stable() {
        let s = clm_section(M, 63488);
        assert!(s.starts_with("\n\n"));
        assert!(s.contains(&format!("`{M}`")) && s.contains(&format!("path = \"{M}\"")));
        assert!(s.contains("63488 tokens") && s.contains("[context: ~1200/63488 tokens]"));
        assert!(s.contains(r#"{"command":"bash","args":["-c","<script>"]}"#));
        assert!(!s.contains("{mirror}") && !s.contains("{limit}"));
        assert_eq!(s, clm_section(M, 63488));
    }

    #[test]
    fn tier_texts_carry_numbers_and_hint_from_the_middle_tier() {
        let low = tier_text(0.25, 32768, 9000, "HINT");
        assert!(low.contains("9000 tokens, about 25% of the 32768-token budget"));
        assert!(!low.contains("HINT"));
        assert!(tier_text(0.5, 32768, 17000, "HINT").ends_with("HINT"));
        let high = tier_text(0.75, 100, 80, "HINT");
        assert!(high.contains("75%") && high.ends_with("HINT"));
    }

    #[test]
    fn persistent_final_and_ledger_carry_their_numbers() {
        let p = persistent_text(25000, 30720, 0.725, "HINT");
        assert!(p.contains("25000/30720 tokens") && p.contains("72%") && p.ends_with("HINT"));
        assert!(final_text(31000, 32768).contains("31000/32768 tokens"));
        let mut a = LedgerArgs {
            retry: 2,
            max_retry: 50,
            budget: 32768,
            n_dropped: 3,
            n_rolled: 7,
            tokens_after: 28000,
            limit: 30720,
            hint: "HINT".into(),
            cmds: vec![],
        };
        let plain = ledger_text(&a);
        assert!(plain.contains("rollback 2/50") && plain.contains("30720-token"));
        assert!(
            !plain.contains("32768"),
            "ledger must name the enforced limit, not the raw budget"
        );
        assert!(
            plain.contains("newest 3 turn(s)") && plain.contains("(7 in total during this run)")
        );
        assert!(plain.contains("~28000/30720 tokens") && plain.ends_with("HINT"));
        a.cmds = vec!["cat big.log".into(), "ls -R".into()];
        assert!(ledger_text(&a).ends_with(":\n  - cat big.log\n  - ls -R"));
        assert!(ledger_text(&a).starts_with(&plain));
    }

    #[test]
    fn every_receipt_is_tagged_and_carries_its_numbers() {
        let all = [
            (receipt_grew(10, 20), vec!["~10", "~20"]),
            (
                receipt_still_over(300, 250, 200),
                vec!["~300", "~250", "~200 tokens"],
            ),
            (
                receipt_applied(300, 100, 4),
                vec!["~300", "~100", "4 turns"],
            ),
            (
                receipt_rejected(100, 300, 100, true, 200),
                vec!["~100", "~300", "within 200 tokens"],
            ),
            (receipt_rejected(100, 150, 100, false, 200), vec!["reduce"]),
            (receipt_no_change_exit(2, 100), vec!["code 2", "~100"]),
            (
                receipt_no_change_matched(100),
                vec!["~100", "[[CTX_TURN 8 role=tool]]"],
            ),
        ];
        for (text, needles) in all {
            assert!(text.starts_with(&format!("\n{RECEIPT_TAG} ")), "{text}");
            assert!(text.ends_with(']'), "{text}");
            for n in needles {
                assert!(text.contains(n), "{text} lacks {n}");
            }
        }
    }

    #[test]
    fn small_texts() {
        assert!(compaction_hint(M).contains(M));
        assert!(observation_cut("ab", 7, "é").starts_with(too_long_hint()));
        assert!(observation_cut("ab", 7, "é")
            .contains("[first 2 chars]\nab\n[7 chars omitted]\n[last 1 chars]\né"));
        assert!(
            cap_note(9392, 32768).contains("9392 characters")
                && cap_note(9392, 32768).contains("32768 tokens")
        );
        assert_eq!(call_line("shell::exec", "{}"), "[call shell::exec {}]");
        assert!(lowered_note("tool", "x").ends_with("tool turn]\nx"));
        assert!(lowered_note("assistant", "x").contains("as an assistant turn"));
        assert!(user_side_assistant("done").ends_with(":]\ndone"));
        assert!(STRATEGY_STOP_REASON.contains("context_strategy"));
    }
}
