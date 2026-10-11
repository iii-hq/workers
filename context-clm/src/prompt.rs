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

You have a limit of {limit} tokens of context. Each command result ends with a readout such as `[context: ~1200/{limit} tokens]`: the first number is your current size, the second is the limit. Every mention of "the limit" below means this number.

Your context after the system prompt and the task is mirrored in the file `{mirror}` (a path relative to your working directory). The file is written before every step, and whatever it holds when the next step begins becomes your context from then on. The system prompt and the task are not in the file and cannot be edited.

The file is a series of turns. Each turn starts with a header line such as `[[CTX_TURN 3 role=tool]]` (turn number, then the role: user, assistant or tool), followed by the turn's text. If you delete a turn's text and leave its header, the turn drops out of your context. Never remove or change the header of a turn you want to keep. Edits take effect on the next step, and the next command result tells you whether yours was applied.

Edit the file with a script. Do not retype its text, and do not print it: you are already reading it as your context. `shell::exec` does not start a shell, so pass the script as {"command":"bash","args":["-c","<script>"]}; heredocs work that way. Example, replacing the text of turn 4 with a note:

    python3 - <<'EOF'
    import re
    path = "{mirror}"
    text = open(path).read()
    note = "[turn 4 summary: tests/api.rs fails on a missing header; the fix belongs in src/http.rs]"
    head = r"(\[\[CTX_TURN 4 role=\w+\]\]\n).*?(?=\n+\[\[CTX_TURN |\Z)"
    text = re.sub(head, lambda m: m.group(1) + note, text, flags=re.S)
    open(path, "w").write(text)
    EOF

An edit makes every turn from the first one you changed to the end of the file get processed again, so a change early in the file costs more than a change late in it. Still, condense old or stale turns first: they are the ones you need least. Do it in one larger pass rather than several small ones, unless you are close to the limit.

Write compact summaries that keep your findings. A summary costs little next to the text it replaces. It should record what you found, what you tried or ruled out, and what you will do next. Never empty the whole file.
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
        "Condense {mirror_rel} from a script, without retyping the text you remove. Find old or stale turns by their header line (for example `[[CTX_TURN 5 role=tool]]`) or by a short line that is unique to where they start and end. Replace them with compact summaries that keep your findings. Leave alone any turn you still rely on."
    )
}

/// What happens when the context goes over the limit. Shared by the nudges
/// and the receipts so the model hears one description.
const OVER_THE_LIMIT: &str = "Going over the limit rolls back your newest turns; if that keeps happening, the run ends after one final turn.";

/// What a replacement summary has to contain.
const CARRY_OVER: &str = "Write each summary so you could carry on from it alone. Cover, in this order: what you will do next; the values you found and whether you verified them; the commands and queries you have already run; the options you ruled out, and why.";

/// The one-shot nudge for a crossed tier (`frac` of the budget).
pub fn tier_text(frac: f64, budget: u64, tokens: u64, hint: &str) -> String {
    let pct = py_round(frac * 100.0);
    let at = format!("Context use: {tokens} tokens, about {pct}% of the {budget}-token budget.");
    if frac <= 0.25 {
        format!("[context note] {at} Nothing to do yet. Just avoid repeating searches and file reads you have already done.")
    } else if frac <= 0.5 {
        format!("[context note] {at} Finish the piece of work you are on, then condense once: replace old or stale turns with compact summaries that keep your findings. {CARRY_OVER} {hint}")
    } else {
        format!("[context warning] {at} The limit is close. Condense finished work now, using compact summaries that keep your findings: a summary that drops almost everything makes you redo the work. {CARRY_OVER} {OVER_THE_LIMIT} {hint}")
    }
}

/// The repeating nudge once the room left is too small.
pub fn persistent_text(tokens: u64, limit: u64, ratio: f64, hint: &str) -> String {
    let pct = py_round(ratio * 100.0);
    format!("[context warning: act now] Context is at {tokens}/{limit} tokens, {pct}% of the limit. Use this turn only to condense: replace old or stale turns with compact summaries that keep your findings, and start no other work first. {OVER_THE_LIMIT} {hint}")
}

/// The last turn granted before the run ends. `limit` is the enforced limit.
pub fn final_text(tokens: u64, limit: u64) -> String {
    format!("[context limit reached: {tokens}/{limit} tokens] This is your last turn; the run stops after it. Spend it on finishing: save your results where the task expects them, and state what is done and what is still open.")
}

/// Inputs of the rollback ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerArgs {
    pub retry: u32,
    pub max_retry: u32,
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
        "[context limit exceeded, rollback {}/{}] The context passed the {}-token limit, so your newest {} turn(s) were removed ({} in total during this run). Their content is gone. The context now stands at ~{}/{} tokens. Use this turn only to condense: replace old or stale turns with compact summaries that keep your findings. {}",
        a.retry, a.max_retry, a.limit, a.n_dropped, a.n_rolled, a.tokens_after, a.limit, a.hint
    );
    if !a.cmds.is_empty() {
        s.push_str("\nThe output of these commands caused the overflow, and their turns are among those removed. Do not rerun them unchanged. If you still need one, first cut down what it prints (head, grep -c, wc -l):");
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
    format!("\n{RECEIPT_TAG} edit kept. The context grew from ~{before} to ~{after} tokens but is still under the limit. To shrink it, replace old text with shorter text instead of adding more.]")
}

pub fn receipt_still_over(before: u64, after: u64, limit: u64) -> String {
    format!("\n{RECEIPT_TAG} edit kept. The context went from ~{before} to ~{after} tokens, which is still above the limit of ~{limit} tokens. Cut more now. {OVER_THE_LIMIT}]")
}

pub fn receipt_applied(before: u64, after: u64, n_turns: usize) -> String {
    format!("\n{RECEIPT_TAG} edit kept. The file now holds {n_turns} turns and ~{after} tokens (it was ~{before}).]")
}

pub fn receipt_rejected(baseline: u64, after: u64, before: u64, fit: bool, limit: u64) -> String {
    let rule = if fit && limit > 0 {
        format!(
            "An edit may make the context larger only if it still ends up within {limit} tokens"
        )
    } else {
        "An edit has to reduce the context".to_string()
    };
    format!("\n{RECEIPT_TAG} edit discarded. It would have grown the context from ~{baseline} to ~{after} tokens, so it stays at ~{before}. {rule}. Use a more compact summary instead of appending text.]")
}

pub fn receipt_no_change_exit(rc: i64, before: u64) -> String {
    format!("\n{RECEIPT_TAG} no edit made. The command exited with code {rc} (its error is shown above), so the context is still ~{before} tokens.]")
}

pub fn receipt_no_change_matched(before: u64) -> String {
    format!("\n{RECEIPT_TAG} no edit made. Nothing in the file matched your script, so the context is still ~{before} tokens. Match text that is really there, or a real header line such as `[[CTX_TURN 8 role=tool]]` (turn number first, then role).]")
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
/// `limit` is the enforced limit (budget minus reserve).
pub fn cap_note(elided: usize, limit: u64) -> String {
    format!("[... {elided} characters removed here so the output fits within the limit of {limit} tokens; use a more selective command, or free space by condensing your context file ...]")
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
    let article = if matches!(role.chars().next(), Some('a' | 'e' | 'i' | 'o' | 'u')) {
        "an"
    } else {
        "a"
    };
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
        assert!(final_text(31000, 32768).contains("last turn"));
        let mut a = LedgerArgs {
            retry: 2,
            max_retry: 50,
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
            plain.contains("newest 3 turn(s)") && plain.contains("(7 in total during this run)")
        );
        assert!(plain.contains("~28000/30720 tokens") && plain.ends_with("HINT"));
        a.cmds = vec!["cat big.log".into(), "ls -R".into()];
        assert!(ledger_text(&a).ends_with(":\n  - cat big.log\n  - ls -R"));
        assert!(ledger_text(&a).starts_with(&plain));
    }

    #[test]
    fn crossing_the_limit_is_described_as_a_rollback_first() {
        let texts = [
            tier_text(0.75, 100, 80, "HINT"),
            persistent_text(25000, 30720, 0.725, "HINT"),
            receipt_still_over(300, 250, 200),
        ];
        for t in texts {
            assert!(t.contains("rolls back your newest turns"), "{t}");
            assert!(t.contains("one final turn"), "{t}");
            assert!(!t.contains("one more turn"), "{t}");
        }
    }

    #[test]
    fn one_instruction_for_what_a_summary_is() {
        let hint = compaction_hint(M);
        let ledger = ledger_text(&LedgerArgs {
            retry: 1,
            max_retry: 50,
            n_dropped: 1,
            n_rolled: 1,
            tokens_after: 10,
            limit: 20,
            hint: String::new(),
            cmds: vec![],
        });
        let texts = [
            hint,
            ledger,
            tier_text(0.5, 100, 60, ""),
            tier_text(0.75, 100, 80, ""),
            persistent_text(80, 100, 0.8, ""),
            clm_section(M, 100),
        ];
        for t in texts {
            assert!(
                t.contains("compact summaries that keep your findings"),
                "{t}"
            );
            assert!(
                !t.contains("brief summar") && !t.contains("thorough"),
                "{t}"
            );
        }
    }

    #[test]
    fn section_defines_its_terms() {
        let s = clm_section(M, 100);
        assert!(s.contains("relative to your working directory"));
        assert!(s.contains("unless you are close to the limit"));
        assert!(s.contains("old or stale turns first"));
        assert!(!s.contains("in your notes") && !s.contains("favour"));
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
        assert!(lowered_note("editor", "x").contains("as an editor turn"));
        assert!(lowered_note("tool", "x").contains("as a tool turn"));
        assert!(user_side_assistant("done").ends_with(":]\ndone"));
        assert!(STRATEGY_STOP_REASON.contains("context_strategy"));
    }
}
