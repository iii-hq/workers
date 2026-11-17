//! The verify tier (opt-in): a judge checks each note against the
//! transcript it came from, and a note the transcript contradicts is
//! replaced through `memory::correct` (the old one stays as history).
//!
//! Memory reads and redacts the transcript (`memory::transcript`); this
//! worker never calls `session::*`. Corrections, agent saves and inferred
//! notes go first, then plain extracted notes, each lane behind its own
//! cursor. A correction is written only when its quote is in a user or
//! assistant line and every number of the new text is in those lines'
//! message text; the stored reason is built here from the matched quote,
//! so no free model text reaches the record.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::consolidate::{call, normalize, BankReport, ConsolidationTransport, MemoryRow};
use crate::semantic::{classify, json_object, pending, Cursor, Outcome};

/// Static system prompt (identical for every note, so providers can cache it).
pub const VERIFY_SYSTEM: &str = "You check one NOTE from a user's long-term memory against \
the conversation it was taken from. Reply ONLY with a JSON object (no prose, no code fences), \
one of:\n\
{\"verdict\": \"supported\"}\n\
{\"verdict\": \"contradicted\", \"quote\": \"<text copied from one line>\", \"corrected\": \
\"<the note rewritten>\"}\n\
\n\
Rules:\n\
1. Answer contradicted ONLY when a user or assistant line states something that conflicts with \
the note. Silence is not a contradiction: a note the transcript does not mention, or covers only \
in part, is supported.\n\
2. Each transcript line is `role [date]: text`. The bracket is the date the line was sent; it is \
not part of the line.\n\
3. quote: copy it verbatim from the text of ONE user or assistant line, after the `role [date]:` \
prefix: the words that conflict with the note. A notice line is context, never a quote.\n\
4. corrected: the note rewritten to agree with the transcript. Change only what the transcript \
contradicts and keep the rest as written. Use only numbers that appear in the message text of \
the lines, never one from a date bracket.";

const MAX_OUTPUT_TOKENS: u32 = 1_024;
/// A correction changes only what the transcript contradicts; extraction
/// writes notes of at most 200 chars.
const MAX_GROWTH_CHARS: usize = 200;
/// An empty quote is in every line; a short one matches by accident.
const MIN_QUOTE_CHARS: usize = 8;
const MAX_QUOTE_CHARS: usize = 200;
/// What memory puts between the head and the tail of a cut line.
const CUT: &str = " … ";
/// A memory worker older than `memory::transcript`. A plain substring would
/// also match memory reporting that session-manager is the missing worker.
const OLD_MEMORY: &str = "memory::transcript: remote error (function_not_found)";

pub struct VerifyOptions {
    pub model: String,
    /// Notes whose transcript is requested, per bank per pass.
    pub max_per_run: usize,
    /// Model calls this bank may still make: the run's cap less what
    /// earlier banks used.
    pub max_llm_calls: usize,
}

/// Bank progress, one position per lane (`first` = corrections, agent saves
/// and inferred notes; `rest` = everything else).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyCursor {
    pub first: Cursor,
    pub rest: Cursor,
}

impl VerifyCursor {
    fn lane(&mut self, first: bool) -> &mut Cursor {
        if first {
            &mut self.first
        } else {
            &mut self.rest
        }
    }
}

/// One redacted transcript line as `memory::transcript` returns it.
#[derive(Debug, Clone, Deserialize)]
pub struct Line {
    pub role: String,
    #[serde(default)]
    pub date: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "verdict", rename_all = "lowercase")]
pub enum Verdict {
    Supported,
    Contradicted { quote: String, corrected: String },
}

/// Strict: garbage or an unknown verdict is an error, never "supported".
pub fn parse_verdict(text: &str) -> Result<Verdict, String> {
    serde_json::from_str(json_object(text)?).map_err(|e| format!("unparseable verify reply: {e}"))
}

pub fn prompt(note: &str, lines: &[Line]) -> String {
    let mut out = format!("<note>{note}</note>\n<transcript>\n");
    for l in lines {
        out.push_str(&format!("{} [{}]: {}\n", l.role, l.date, l.text));
    }
    out.push_str("</transcript>");
    out
}

/// The guards a correction passes before anything is written, over the
/// lines the judge saw. Returns the corrected text and the reason to store.
pub fn check(
    quote: &str,
    corrected: &str,
    note: &str,
    lines: &[Line],
) -> Result<(String, String), &'static str> {
    let corrected = corrected.trim();
    if normalize(corrected).is_empty() {
        return Err("empty corrected text");
    }
    if normalize(corrected) == normalize(note) {
        return Err("corrected text unchanged");
    }
    if corrected.chars().count() > note.chars().count() + MAX_GROWTH_CHARS {
        return Err("corrected text too long");
    }
    // `numbers` sees ASCII digits only, so another script's numeral
    // would slip past the number guard.
    if corrected
        .chars()
        .any(|c| c.is_numeric() && !c.is_ascii_digit())
    {
        return Err("corrected text has a non-ASCII numeral");
    }
    // A notice is written by the system or a bot, not by the conversation.
    // A long line comes as `head … tail` cut at any byte: no quote may span
    // the cut, and the partial word or number on each side of it is dropped.
    let evidence: Vec<&str> = lines
        .iter()
        .filter(|l| l.role == "user" || l.role == "assistant")
        .flat_map(|l| {
            let segments: Vec<&str> = l.text.split(CUT).collect();
            let last = segments.len() - 1;
            segments.into_iter().enumerate().map(move |(i, s)| {
                let s = if i > 0 {
                    s.split_once(char::is_whitespace).map_or("", |p| p.1)
                } else {
                    s
                };
                if i < last {
                    s.rsplit_once(char::is_whitespace).map_or("", |p| p.0)
                } else {
                    s
                }
            })
        })
        .collect();
    let q = normalize(quote);
    if q.chars().count() < MIN_QUOTE_CHARS {
        return Err("quote too short");
    }
    let padded = format!(" {q} ");
    if !evidence
        .iter()
        .any(|t| format!(" {} ", normalize(t)).contains(&padded))
    {
        return Err("quote not in a user or assistant line");
    }
    // The note's own numbers count too: extraction writes resolved dates
    // ("2026-10-15" for "tomorrow") that no message text holds, and a
    // correction may keep them. The judge still cannot add a new one.
    let known: HashSet<String> = evidence
        .iter()
        .copied()
        .chain([note])
        .flat_map(numbers)
        .collect();
    // shortcut: number words ("three") are not numbers, so one the
    // transcript never states passes; an English number-word list would be
    // the upgrade if judges start spelling numbers out.
    if !numbers(corrected).is_subset(&known) {
        return Err("a number is not in the transcript or the note");
    }
    let q: String = q.chars().take(MAX_QUOTE_CHARS).collect();
    Ok((
        corrected.to_string(),
        format!("the source conversation says: \"{q}\""),
    ))
}

/// Digit-bearing whole tokens of [`normalize`], which keeps the sign and the
/// `. , : -` joins: "1,500", "1.5", "10:30" and "-5" are each one number, so
/// none of them vouches for a fragment ("500", "10") or a flipped sign.
/// shortcut: no folding, so "9:30" against "09:30" or "1500" against "1,500"
/// is refused; fold leading zeros and thousands commas on both sides if
/// such refusals show up in `verify_rejected`.
fn numbers(text: &str) -> HashSet<String> {
    normalize(text)
        .split(' ')
        .filter(|w| w.bytes().any(|b| b.is_ascii_digit()))
        .map(str::to_owned)
        .collect()
}

fn source<'a>(row: &'a MemoryRow, key: &str) -> Option<&'a str> {
    row.snapshot.get("source")?.get(key)?.as_str()
}

/// Corrections, agent saves and inferred notes: checked before plain
/// extracted ones.
fn priority(row: &MemoryRow) -> bool {
    source(row, "correction_reason").is_some()
        || match row.snapshot.get("confidence").and_then(Value::as_str) {
            Some("inferred") => true,
            Some("stated") => source(row, "turn_id").is_some(),
            _ => false,
        }
}

/// Why a note cannot be checked, decided without any call.
fn unchecked(row: &MemoryRow) -> Option<&'static str> {
    // A reinforced note has later evidence its stored span does not cover.
    if row.corroboration > 0 {
        return Some("reinforced");
    }
    let span = source(row, "first_entry_id").is_some() && source(row, "entry_id").is_some();
    if source(row, "session_id").is_none() || !(span || source(row, "turn_id").is_some()) {
        return Some("no recorded span");
    }
    None
}

fn cursor_key(bank: &str) -> String {
    format!("verify_cursor:{bank}")
}

enum Step {
    /// Judged, refused or skipped: the cursor may pass the note. `parsed` =
    /// a judge reply parsed.
    Done { parsed: bool },
    /// No verdict: the judge reply did not parse, was truncated, or the
    /// transcript overflowed the model's context.
    Unparsed,
    /// Stop the tier with this note; the cursor stays before the note.
    Stop(String),
}

/// The tier for one bank. Runs last, on its own fresh fetch, so it never
/// sees records the tiers before it retired.
pub async fn run_verify_tier<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    bank: &str,
    opts: &VerifyOptions,
    budget: &mut usize,
    report: &mut BankReport,
) {
    let rows = match crate::consolidate::fetch_live(iii, bank, &mut report.errors).await {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(e);
            return;
        }
    };
    // A failed read must never pass for "no cursor": that would re-judge the
    // bank from its oldest note.
    let mut cursor: VerifyCursor =
        match crate::functions::state_try_get(iii, &cursor_key(bank)).await {
            Ok(None) => VerifyCursor::default(),
            Ok(Some(v)) => match serde_json::from_value(v) {
                Ok(c) => c,
                Err(e) => {
                    report
                        .notes
                        .push(format!("verify tier skipped: cursor undecodable: {e}"));
                    return;
                }
            },
            Err(e) => {
                report
                    .notes
                    .push(format!("verify tier skipped: cursor unreadable: {e}"));
                return;
            }
        };

    let mut skips: BTreeMap<String, usize> = BTreeMap::new();
    let mut checked = 0usize;
    // While no reply has parsed, the cursor stays before the first note
    // whose reply did not; a second such note stops the tier (the judge
    // itself is failing). Once one parses, an unparsed note is passed.
    let mut parsed = false;
    let mut unparsed: Option<String> = None;
    // The lane held before `unparsed`, and the last note of it visited
    // since: a parse in either lane moves that lane's cursor there.
    let mut held: Option<(bool, Cursor)> = None;
    // Local skips moved a cursor since the last save.
    let mut dirty = false;
    'tier: for first in [true, false] {
        let lane: Vec<MemoryRow> = rows
            .iter()
            .filter(|r| !r.pinned && priority(r) == first)
            .cloned()
            .collect();
        let start = cursor.lane(first).clone();
        for row in pending(&lane, &start, usize::MAX) {
            if let Some(why) = unchecked(row) {
                *skips.entry(why.into()).or_default() += 1;
                match &mut held {
                    Some((which, last)) if *which == first => *last = Cursor::of(row),
                    _ => {
                        *cursor.lane(first) = Cursor::of(row);
                        dirty = true;
                    }
                }
                continue;
            }
            if checked == opts.max_per_run {
                break 'tier;
            }
            if *budget == 0 {
                report
                    .notes
                    .push("verify tier deferred: per-run supersede cap reached".into());
                break 'tier;
            }
            if report.llm_calls >= opts.max_llm_calls {
                report
                    .notes
                    .push("verify tier deferred: per-run LLM call cap reached".into());
                break 'tier;
            }
            checked += 1;
            match verify_note(iii, bank, opts, row, budget, report, &mut skips).await {
                Step::Stop(note) => {
                    report.notes.push(note);
                    break 'tier;
                }
                Step::Unparsed if parsed => report.verify_rejected += 1,
                Step::Unparsed => {
                    if unparsed.is_some() {
                        break 'tier;
                    }
                    unparsed = Some(row.id.clone());
                    held = Some((first, Cursor::of(row)));
                    continue;
                }
                Step::Done { parsed: now } => {
                    if now && !parsed {
                        parsed = true;
                        if let Some((which, last)) = held.take() {
                            *cursor.lane(which) = last;
                            report.verify_rejected += 1;
                        }
                    }
                }
            }
            if let Some((which, last)) = &mut held {
                if *which == first {
                    *last = Cursor::of(row);
                    continue;
                }
            }
            *cursor.lane(first) = Cursor::of(row);
            dirty = false;
            // Saved per judged note: a crash must not re-spend its calls.
            if let Err(e) = crate::functions::state_set(iii, &cursor_key(bank), json!(cursor)).await
            {
                report.notes.push(format!("verify cursor not saved: {e}"));
                break 'tier;
            }
        }
    }
    if dirty {
        if let Err(e) = crate::functions::state_set(iii, &cursor_key(bank), json!(cursor)).await {
            report.notes.push(format!("verify cursor not saved: {e}"));
        }
    }
    if let (false, Some(id)) = (parsed, &unparsed) {
        report.notes.push(format!(
            "verify tier stopped: no judge reply parsed; cursor held before `{id}`"
        ));
    }
    let total: usize = skips.values().sum();
    if total > 0 {
        report.verify_skipped += total;
        let why: Vec<String> = skips.iter().map(|(k, n)| format!("{k} {n}")).collect();
        report
            .notes
            .push(format!("verify: skipped {total} ({})", why.join(", ")));
    }
}

/// Read one note's transcript, judge it, and write a correction that passes
/// [`check`]. Counts its own outcome on `report` and `skips`.
async fn verify_note<T: ConsolidationTransport + ?Sized>(
    iii: &T,
    bank: &str,
    opts: &VerifyOptions,
    row: &MemoryRow,
    budget: &mut usize,
    report: &mut BankReport,
    skips: &mut BTreeMap<String, usize>,
) -> Step {
    let reply = match call(
        iii,
        "memory::transcript",
        json!({ "bank": bank, "id": row.id }),
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) if e.starts_with(OLD_MEMORY) => {
            return Step::Stop(
                "verify tier skipped: memory worker lacks memory::transcript — upgrade memory"
                    .into(),
            )
        }
        Err(e) => return Step::Stop(format!("verify tier stopped: {e}")),
    };
    if let Some(missing) = reply.get("missing").and_then(Value::as_str) {
        *skips.entry(missing.into()).or_default() += 1;
        return Step::Done { parsed: false };
    }
    let lines: Vec<Line> = match serde_json::from_value(reply["lines"].clone()) {
        Ok(lines) => lines,
        Err(e) => return Step::Stop(format!("verify tier stopped: memory::transcript: {e}")),
    };

    report.llm_calls += 1;
    let result = iii
        .complete(
            &opts.model,
            VERIFY_SYSTEM,
            prompt(&row.text, &lines),
            MAX_OUTPUT_TOKENS,
        )
        .await;
    if let Ok(reply) = &result {
        crate::consolidate::tally_usage(report, &reply["usage"]);
    }
    let text = match classify(&result) {
        Outcome::Reply(text) => text,
        Outcome::Transport(e) => return Step::Stop(format!("verify tier stopped: {e}")),
        // Truncated or over the context: no verdict, the same as a reply that
        // does not parse, so a judge that always truncates holds the cursor.
        Outcome::InputShaped(e) => {
            tracing::debug!(bank, id = %row.id, error = %e, "verify reply refused");
            return Step::Unparsed;
        }
    };
    let (quote, corrected) = match parse_verdict(&text) {
        Ok(Verdict::Supported) => {
            report.verified += 1;
            return Step::Done { parsed: true };
        }
        Ok(Verdict::Contradicted { quote, corrected }) => (quote, corrected),
        Err(e) => {
            tracing::debug!(bank, id = %row.id, error = %e, "verify reply unparsed");
            return Step::Unparsed;
        }
    };
    let (text, reason) = match check(&quote, &corrected, &row.text, &lines) {
        Ok(fix) => fix,
        Err(why) => {
            tracing::debug!(bank, id = %row.id, why, "verify correction refused");
            report.verify_rejected += 1;
            return Step::Done { parsed: true };
        }
    };
    let reply = match call(
        iii,
        "memory::correct",
        json!({
            "bank": bank,
            "id": row.id,
            "text": text,
            "reason": reason,
            "expected_memory": row.expected_snapshot(),
        }),
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) => return Step::Stop(format!("verify tier stopped: {e}")),
    };
    if reply.get("corrected") == Some(&Value::Bool(true)) {
        report.corrected += 1;
        *budget = budget.saturating_sub(1);
    } else {
        let why = reply
            .get("skipped")
            .and_then(Value::as_str)
            .unwrap_or("not corrected");
        *skips.entry(why.into()).or_default() += 1;
    }
    Step::Done { parsed: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "User's team standup is at 9am.";
    const QUOTE: &str = "our team standup moved to 10am";
    const FIXED: &str = "User's team standup is at 10am.";

    fn line(role: &str, text: &str) -> Line {
        Line {
            role: role.into(),
            date: "2026-10-15 Thursday".into(),
            text: text.into(),
        }
    }

    fn transcript() -> Vec<Line> {
        vec![
            line(
                "user",
                "Heads up: our team standup moved to 10am, starting Monday.",
            ),
            line("assistant", "Noted, the standup is at 10am from now on."),
            line("notice", "The calendar bot booked an 11am sync for Friday."),
        ]
    }

    fn fix(quote: &str, corrected: &str) -> Result<(String, String), &'static str> {
        check(quote, corrected, NOTE, &transcript())
    }

    #[test]
    fn a_quoted_correction_with_known_numbers_passes() {
        assert_eq!(
            fix(QUOTE, &format!("  {FIXED} ")),
            Ok((
                FIXED.to_string(),
                format!("the source conversation says: \"{QUOTE}\"")
            ))
        );
    }

    #[test]
    fn number_outside_the_transcript_is_refused() {
        let missing = Err("a number is not in the transcript or the note");
        assert_eq!(fix(QUOTE, "User's team standup is at 8am."), missing);
        // Only in a line's date bracket.
        assert_eq!(
            fix(QUOTE, "User's team standup is at 10am since 2026-10-15."),
            missing
        );
        // Only in a notice line.
        assert_eq!(fix(QUOTE, "User's team standup is at 11am."), missing);
        assert_eq!(
            fix(QUOTE, "User's team standup is at ١٠am."),
            Err("corrected text has a non-ASCII numeral")
        );
        // A fragment of a number, or its sign flipped, is not that number.
        for (said, corrected) in [
            (
                "My rent went up to $1,500 a month",
                "User's rent is $500 a month.",
            ),
            ("I run 1.5 km each morning", "User runs 5 km each morning."),
            (
                "Our standup moved to 10:30",
                "User's team standup is at 10.",
            ),
            ("It is 5 degrees outside", "It is -5 degrees outside."),
        ] {
            assert_eq!(check(said, corrected, NOTE, &[line("user", said)]), missing);
        }
        assert!(check(
            "My rent went up to $1,500",
            "User's rent is $1,500 a month.",
            NOTE,
            &[line("user", "My rent went up to $1,500 a month")]
        )
        .is_ok());
    }

    #[test]
    fn a_number_the_note_already_carries_may_stay() {
        let dated = "User's team standup moved to 9am on 2026-10-01.";
        let fixed = "User's team standup moved to 10am on 2026-10-01.";
        assert!(check(QUOTE, fixed, dated, &transcript()).is_ok());
        // A number in neither the note nor the transcript is still refused.
        assert_eq!(
            check(
                QUOTE,
                "User's team standup moved to 10am on 2026-10-02.",
                dated,
                &transcript()
            ),
            Err("a number is not in the transcript or the note")
        );
    }

    #[test]
    fn nothing_is_taken_from_across_the_cut_of_a_long_line() {
        // Memory keeps a long line's head and tail, cut at any byte.
        let cut = [line("assistant", "Your rent is 15 … 00 a month")];
        let fixed = "User's rent is 15 a month.";
        assert_eq!(
            check("your rent is 15", fixed, NOTE, &cut),
            Err("quote not in a user or assistant line")
        );
        assert_eq!(
            check("your rent is", fixed, NOTE, &cut),
            Err("a number is not in the transcript or the note")
        );
        assert_eq!(
            check("is 15 00 a month", fixed, NOTE, &cut),
            Err("quote not in a user or assistant line")
        );
        // Whole words away from the cut still count.
        assert!(check("your rent is", "User's rent is a month.", NOTE, &cut).is_ok());
    }

    #[test]
    fn a_quote_must_be_whole_words_of_a_user_or_assistant_line() {
        let absent = Err("quote not in a user or assistant line");
        assert_eq!(fix("our team standup moved to 11am", FIXED), absent);
        assert_eq!(fix("andup is at 10am", FIXED), absent);
        assert_eq!(fix("booked an 11am sync", FIXED), absent);
        assert_eq!(fix("", FIXED), Err("quote too short"));
        assert_eq!(fix("at 10am", FIXED), Err("quote too short"));
        // Case and punctuation do not matter.
        assert!(fix("Standup is at 10AM!", FIXED).is_ok());
    }

    #[test]
    fn an_unchanged_empty_or_overlong_correction_is_refused() {
        assert_eq!(
            fix(QUOTE, "user's team standup is at 9am"),
            Err("corrected text unchanged")
        );
        assert_eq!(fix(QUOTE, " ... "), Err("empty corrected text"));
        assert_eq!(
            fix(QUOTE, &format!("{FIXED} {}", "x".repeat(MAX_GROWTH_CHARS))),
            Err("corrected text too long")
        );
    }

    #[test]
    fn garbage_and_unknown_verdicts_fail_to_parse() {
        assert!(parse_verdict("The note looks fine.").is_err());
        assert!(parse_verdict(r#"{"verdict":"unsure"}"#).is_err());
        assert!(parse_verdict(r#"{"verdict":"contradicted","quote":"x"}"#).is_err());
        assert_eq!(
            parse_verdict("```json\n{\"verdict\":\"supported\"}\n```"),
            Ok(Verdict::Supported)
        );
        assert_eq!(
            parse_verdict(r#"{"verdict":"contradicted","quote":"q","corrected":"c"}"#),
            Ok(Verdict::Contradicted {
                quote: "q".into(),
                corrected: "c".into()
            })
        );
    }

    #[test]
    fn the_prompt_puts_the_date_in_a_bracket_before_each_line() {
        let p = prompt(NOTE, &transcript()[..1]);
        assert_eq!(
            p,
            "<note>User's team standup is at 9am.</note>\n<transcript>\n\
             user [2026-10-15 Thursday]: Heads up: our team standup moved to 10am, starting \
             Monday.\n</transcript>"
        );
    }
}
