//! The source transcript of one memory, read back for the verify tier: the
//! lines its extraction read (the run between `first_entry_id` and
//! `entry_id`), or the turn an agent saved it from. Each line starts as what
//! the extraction model reads of it ([`super::line`]): redacted and cut, so
//! the transcript never leaves memory raw. A user line is then cut again to
//! its head and tail past [`USER_LINE_CHARS`], so the judge can read less of
//! it than extraction did.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

use super::{CALL_CHARS, PAGE_SIZE, cut_middle, line, parse_entry, session_code, session_page};
use crate::error::MemoryError;
use crate::types::Provenance;

/// Pages a span read may take. Text-less steps cost no chars, so extraction
/// closes their run on ids alone, at `RUN_KEY_BYTES / key_bytes(id)`
/// entries: under `10 * PAGE_SIZE` for ids of 23 chars or more (harness ids
/// are 34 to 50).
const SPAN_PAGES: usize = 10;
/// Pages a turn walk from the root may take.
const TURN_PAGES: usize = 30;
/// One session-manager read. A read starts only with all of it left before
/// [`READ_TOTAL`] and is never cut short, so a timeout is a hung sibling (an
/// error the caller retries) while a long walk is a skip.
const PAGE_MS: u64 = 5_000;
/// The whole read; a read that could not finish by then is not started and
/// the note is skipped as `too_long`.
const READ_TOTAL: Duration = Duration::from_secs(10);

/// One transcript line as the verify judge reads it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Line {
    /// `user`, `assistant`, or `notice` (written by the harness or a bot).
    pub role: String,
    /// The day the line was sent, `YYYY-MM-DD Weekday` in local time.
    pub date: String,
    pub text: String,
}

/// Why a note has no transcript to check against (the note is skipped).
pub(crate) type Missing = &'static str;

/// A read that ends early: a skip, or a sibling failure the caller retries.
enum Stop {
    Missing(Missing),
    Failed(MemoryError),
}

const UNREADABLE: Stop = Stop::Missing("unreadable");

/// The lines `source` came from. `Ok(Err(missing))`: the note or its span
/// cannot be checked; `Err`: session-manager is down, timed out, or
/// answered with an error outside its read contract.
pub(crate) async fn read(
    iii: &IIIClient,
    source: &Provenance,
) -> Result<Result<Vec<Line>, Missing>, MemoryError> {
    let Some(session_id) = source.session_id.as_deref() else {
        return Ok(Err("no_span"));
    };
    let deadline = Instant::now() + READ_TOTAL;
    let read = match (
        source.first_entry_id.as_deref(),
        source.entry_id.as_deref(),
        source.turn_id.as_deref(),
    ) {
        (Some(first), Some(last), _) => span(iii, deadline, session_id, first, last).await,
        (_, _, Some(turn_id)) => turn(iii, deadline, session_id, turn_id).await,
        _ => Err(Stop::Missing("no_span")),
    };
    match read {
        Ok(lines) => Ok(checked(lines)),
        Err(Stop::Missing(missing)) => Ok(Err(missing)),
        Err(Stop::Failed(error)) => Err(error),
    }
}

/// The read lines, or why there is nothing a judge can read.
fn checked(lines: Vec<Line>) -> Result<Vec<Line>, Missing> {
    if lines.is_empty() {
        Err("empty")
    } else if lines.iter().map(|l| l.text.len()).sum::<usize>() > CALL_CHARS {
        Err("too_long")
    } else {
        Ok(lines)
    }
}

/// An extracted run: the entries after `first` through `last`, read as the
/// leaf of its own path (any branch), then `first` itself.
async fn span(
    iii: &IIIClient,
    deadline: Instant,
    session_id: &str,
    first: &str,
    last: &str,
) -> Result<Vec<Line>, Stop> {
    let mut lines = Vec::new();
    pages(
        iii,
        deadline,
        json!({
            "session_id": session_id, "from_entry_id": last, "after_entry_id": first,
            "roles": ["user", "assistant"], "include_image_data": false, "limit": PAGE_SIZE,
        }),
        SPAN_PAGES,
        |items| {
            lines.extend(items.iter().filter_map(to_line));
            true
        },
    )
    .await?;
    let reply = call(
        iii,
        deadline,
        "session::get-message",
        json!({"session_id": session_id, "entry_id": first, "include_image_data": false}),
    )
    .await?;
    if reply.is_null() {
        return Err(Stop::Missing("entry_not_found"));
    }
    let entry = &reply["entry"];
    let item =
        json!({"entry_id": entry["id"], "message": entry["message"], "origin": entry["origin"]});
    crate::extraction_proof::source_page(&json!({ "messages": [&item] }))
        .map_err(|_| UNREADABLE)?;
    if let Some(line) = to_line(&item) {
        lines.insert(0, line);
    }
    Ok(lines)
}

/// An agent's save: the turn's lines off a walk from the root.
async fn turn(
    iii: &IIIClient,
    deadline: Instant,
    session_id: &str,
    turn_id: &str,
) -> Result<Vec<Line>, Stop> {
    let mut walk = TurnWalk::new(turn_id);
    pages(
        iii,
        deadline,
        json!({
            "session_id": session_id, "roles": ["user", "assistant"],
            "include_image_data": false, "limit": PAGE_SIZE,
        }),
        TURN_PAGES,
        |items| walk.feed(items),
    )
    .await?;
    walk.finish().map_err(Stop::Missing)
}

/// Pages `session::messages` from `payload`, at most `max` pages, handing
/// each page's items to `feed` until it returns false or the path ends.
/// More pages than `max` is `too_long`.
async fn pages(
    iii: &IIIClient,
    deadline: Instant,
    mut payload: Value,
    max: usize,
    mut feed: impl FnMut(&[Value]) -> bool,
) -> Result<(), Stop> {
    let mut cursors = HashSet::new();
    for _ in 0..max {
        let reply = call(iii, deadline, "session::messages", payload.clone()).await?;
        crate::extraction_proof::source_page(&reply).map_err(|_| UNREADABLE)?;
        let items = reply["messages"].as_array().ok_or(UNREADABLE)?;
        let next = match reply.get("next_cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(next)) if !next.is_empty() => Some(next),
            _ => return Err(UNREADABLE),
        };
        if !feed(items) {
            return Ok(());
        }
        let Some(next) = next else {
            return Ok(());
        };
        if items.is_empty() || !cursors.insert(next.clone()) {
            return Err(UNREADABLE);
        }
        if let Some(fields) = payload.as_object_mut() {
            fields.remove("after_entry_id");
        }
        payload["cursor"] = json!(next);
    }
    Err(Stop::Missing("too_long"))
}

/// One session-manager read; its documented "not there" codes are skips.
/// Without a full [`PAGE_MS`] left before `deadline` it is not started.
async fn call(
    iii: &IIIClient,
    deadline: Instant,
    function_id: &str,
    payload: Value,
) -> Result<Value, Stop> {
    if deadline.saturating_duration_since(Instant::now()) < Duration::from_millis(PAGE_MS) {
        return Err(Stop::Missing("too_long"));
    }
    const MISSING: [(&str, Missing); 3] = [
        ("session/not_found", "session_not_found"),
        ("session/entry_not_found", "entry_not_found"),
        ("session/invalid_cursor", "off_path"),
    ];
    session_page(iii, function_id, payload, PAGE_MS)
        .await
        .map_err(
            |e| match MISSING.iter().find(|(code, _)| session_code(&e, code)) {
                Some((_, missing)) => Stop::Missing(missing),
                None => Stop::Failed(MemoryError::SiblingUnavailable(format!(
                    "{function_id}: {e}"
                ))),
            },
        )
}

/// One page item as a judge line; `None` for an entry with no message (a
/// custom entry), no text (a tool step), another role (a function result),
/// or our own injected `<memory …>` block.
fn to_line(item: &Value) -> Option<Line> {
    let (_, role, text, ts) = parse_entry(item)?;
    if !matches!(role.as_str(), "user" | "assistant" | "notice") {
        return None;
    }
    let (date, mut text) = line(&role, &text, ts)?;
    // shortcut: a note from the middle of a long message is judged on
    // silence; show the judge the piece it came from if verify must cover it.
    if role == "user" {
        cut_middle(&mut text, USER_LINE_CHARS);
    }
    Some(Line { role, date, text })
}

/// How much of a user line the judge reads whole.
const USER_LINE_CHARS: usize = 12_000;

/// The lines of one turn off a root → leaf walk, fed a page at a time: the
/// user line right before the turn's first entry, then every line through
/// its last entry (a user message sent mid-turn included). An entry is the
/// turn's when its `origin.turn_id` names it.
pub(crate) struct TurnWalk<'a> {
    turn_id: &'a str,
    /// The latest line before the first hit, when it is a user line.
    before: Option<Line>,
    lines: Vec<Line>,
    /// How many of `lines` run through the last hit; `None` before a hit.
    through: Option<usize>,
}

impl<'a> TurnWalk<'a> {
    pub(crate) fn new(turn_id: &'a str) -> Self {
        Self {
            turn_id,
            before: None,
            lines: Vec::new(),
            through: None,
        }
    }

    /// Reads one page; `false` once the turn is over: a page with no hit
    /// after one.
    pub(crate) fn feed(&mut self, items: &[Value]) -> bool {
        let mut hit = false;
        for item in items {
            let line = to_line(item);
            let ours =
                item.pointer("/origin/turn_id").and_then(Value::as_str) == Some(self.turn_id);
            if self.through.is_none() && !ours {
                // An entry with no line (an injected `<memory …>` block) keeps
                // the request it follows; a reply, even a text-less one,
                // ends it.
                let reply =
                    item.pointer("/message/role").and_then(Value::as_str) == Some("assistant");
                if line.is_some() || reply {
                    self.before = line.filter(|l| l.role == "user");
                }
                continue;
            }
            self.lines.extend(line);
            if ours {
                hit = true;
                self.through = Some(self.lines.len());
            }
        }
        hit || self.through.is_none()
    }

    pub(crate) fn finish(self) -> Result<Vec<Line>, Missing> {
        let through = self.through.ok_or("turn_not_found")?;
        Ok(self
            .before
            .into_iter()
            .chain(self.lines.into_iter().take(through))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, role: &str, text: &str, turn: Option<&str>) -> Value {
        json!({
            "entry_id": id,
            "message": {"role": role, "content": [{"type": "text", "text": text}],
                "timestamp": 1_760_443_200_000_u64},
            "origin": turn.map(|t| json!({"turn_id": t})),
        })
    }

    fn texts(lines: &[Line]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn lines_keep_conversation_text_only_and_redacted() {
        let key = format!("AKIA{}", "Q".repeat(16));
        let custom = json!({"entry_id": "c", "custom": {"custom_type": "compaction", "data": {}}});
        let items = [
            item("u", "user", &format!("my key is {key}"), None),
            item("f", "function_result", "tool output", None),
            custom,
            item(
                "m",
                "assistant",
                "<memory bank=\"b\">injected</memory>",
                None,
            ),
            item("e", "assistant", "  ", None),
            item("a", "assistant", "noted", None),
        ];
        let lines: Vec<Line> = items.iter().filter_map(to_line).collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0].role, "user");
        assert!(lines[0].text.starts_with("my key is "));
        assert!(!lines[0].text.contains(&key), "{}", lines[0].text);
        assert_eq!(lines[1].text, "noted");
        assert!(lines[1].date.len() > "YYYY-MM-DD".len());
    }

    #[test]
    fn a_long_user_line_is_shown_as_its_head_and_tail() {
        let text = format!("head {} tail", "x".repeat(14_990));
        let line = to_line(&item("u", "user", &text, None)).unwrap();
        assert!(line.text.starts_with("head ") && line.text.ends_with(" tail"));
        assert!(line.text.contains(" … ") && line.text.len() <= 12_005);
    }

    #[test]
    fn a_turn_walk_reads_the_request_and_the_turn_through_its_last_entry() {
        let mut walk = TurnWalk::new("t1");
        assert!(walk.feed(&[
            item("u0", "user", "an earlier request", None),
            item("a0", "assistant", "an earlier reply", Some("t0")),
            item("u1", "user", "please use port 8080", None),
            item("a1", "assistant", "on it", Some("t1")),
        ]));
        assert!(walk.feed(&[
            item("u2", "user", "and keep it local", None),
            item("a2", "assistant", "saved", Some("t1")),
            item("u3", "user", "a later request", None),
        ]));
        assert!(!walk.feed(&[item("a3", "assistant", "a later reply", Some("t2"))]));
        assert_eq!(
            texts(&walk.finish().unwrap()),
            [
                "please use port 8080",
                "on it",
                "and keep it local",
                "saved"
            ]
        );

        // A turn woken without a user line starts at its first entry.
        let mut walk = TurnWalk::new("t1");
        walk.feed(&[
            item("a0", "assistant", "an earlier reply", Some("t0")),
            item("a1", "assistant", "woken", Some("t1")),
        ]);
        assert_eq!(texts(&walk.finish().unwrap()), ["woken"]);

        // An injected memory block between the request and the turn keeps
        // the request; a text-less earlier reply still ends it.
        let mut walk = TurnWalk::new("t1");
        walk.feed(&[
            item("u1", "user", "please use port 8080", None),
            item(
                "m1",
                "user",
                "<memory bank=\"main\">port 3000</memory>",
                None,
            ),
            item("a1", "assistant", "on it", Some("t1")),
        ]);
        assert_eq!(
            texts(&walk.finish().unwrap()),
            ["please use port 8080", "on it"]
        );
        let mut walk = TurnWalk::new("t1");
        walk.feed(&[
            item("u0", "user", "an earlier request", None),
            item("a0", "assistant", "", Some("t0")),
            item("a1", "assistant", "woken", Some("t1")),
        ]);
        assert_eq!(texts(&walk.finish().unwrap()), ["woken"]);

        let mut walk = TurnWalk::new("unknown");
        assert!(walk.feed(&[item("u0", "user", "hello", None)]));
        assert_eq!(walk.finish(), Err("turn_not_found"));
    }

    #[tokio::test]
    async fn a_call_without_its_full_timeout_left_is_never_started() {
        // Not connected: a started call would fail, so `too_long` proves none was.
        let iii = IIIClient::new("ws://127.0.0.1:1");
        let late = Instant::now() + Duration::from_millis(PAGE_MS - 1);
        assert!(matches!(
            call(&iii, late, "session::messages", json!({})).await,
            Err(Stop::Missing("too_long"))
        ));
    }

    #[test]
    fn text_past_one_call_is_too_long_and_none_is_empty() {
        let line = |text: String| Line {
            role: "user".into(),
            date: "2025-10-14 Tuesday".into(),
            text,
        };
        assert_eq!(checked(Vec::new()), Err("empty"));
        let fits = vec![
            line("x".repeat(CALL_CHARS / 2)),
            line("y".repeat(CALL_CHARS / 2)),
        ];
        assert_eq!(checked(fits.clone()), Ok(fits));
        let over = vec![
            line("x".repeat(CALL_CHARS / 2)),
            line("y".repeat(CALL_CHARS / 2 + 1)),
        ];
        assert_eq!(checked(over), Err("too_long"));
    }
}
