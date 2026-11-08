//! Conservative personal dated commitments. Calendar weeks start Monday in UTC;
//! next week is [next Monday 00:00, following Monday 00:00), never rolling 7d.
//! Date-only assertions retain their stated calendar date. RFC3339 instants are
//! converted to UTC before week membership; unknown named zones fail closed.
use crate::types::{EvaluationClock, Memory};
use chrono::{Datelike, Duration, NaiveDate};
use regex::Regex;
use std::sync::LazyLock;

/// Synthetic benchmark input correction, not the machine's current date:
/// Monday 12 October 2026 00:00 UTC => next week [19,26) October.
pub fn fixture_clock() -> EvaluationClock {
    EvaluationClock::synthetic(
        NaiveDate::from_ymd_opt(2026, 10, 12)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis() as u64,
    )
}

pub fn scope(query: &str) -> bool {
    static QUERY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"^(?:tenho (?:algum compromisso marcado|compromissos|algum compromisso) na próxima semana|quais (?:são )?(?:os )?meus compromissos (?:na|da) próxima semana|what (?:appointments|events|commitments) do i have next week|do i have any (?:appointments|commitments) next week|what is on my calendar next week)\?$"
    ).unwrap()
    });
    QUERY.is_match(&query.trim().to_lowercase())
}

/// Denial-only boundary: mentioning this interval with an agenda noun needs
/// currentness proof even when owner/quote/negation syntax cannot be certified.
/// This never grants a witness; `scope` remains the affirmative owner gate.
pub fn requires_currentness(query: &str) -> bool {
    let q = query.to_lowercase();
    (q.contains("next week") || q.contains("próxima semana"))
        && [
            "appointment",
            "commitment",
            "event",
            "calendar",
            "compromisso",
            "agenda",
        ]
        .iter()
        .any(|noun| q.contains(noun))
}

#[derive(Debug, PartialEq, Eq)]
struct Event {
    date: NaiveDate,
    kind: String,
    time: Option<String>,
    // Only complete bilingual appointment frames may dedup. A talk's title,
    // language, venue and details remain distinct, not bags of event words.
    simple_appointment: bool,
}

fn month(text: &str) -> Option<u32> {
    Some(match text {
        "january" | "janeiro" => 1,
        "february" | "fevereiro" => 2,
        "march" | "março" => 3,
        "april" | "abril" => 4,
        "may" | "maio" => 5,
        "june" | "junho" => 6,
        "july" | "julho" => 7,
        "august" | "agosto" => 8,
        "september" | "setembro" => 9,
        "october" | "outubro" => 10,
        "november" | "novembro" => 11,
        "december" | "dezembro" => 12,
        _ => return None,
    })
}

fn calendar(text: &str) -> Option<(NaiveDate, Option<String>)> {
    static ISO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b\d{4}-\d{2}-\d{2}(?:[tT]\d{2}:\d{2}:\d{2}(?:[zZ]|[+-]\d{2}:\d{2}))?\b")
            .unwrap()
    });
    static SLASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(\d{1,2})/(\d{1,2})/(\d{4})\b").unwrap());
    static NAMED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(\d{1,2})(?: de)? ([\p{L}]+)(?: de)? (\d{4})\b").unwrap());
    // Parse a timestamp as ONE frame before looking for a standalone clock.
    // Otherwise `:30:00` inside RFC3339 can be mistaken for hour 30, and an
    // offset can be mistaken for the appointment time. UTC controls both fields.
    let date = if let Some(m) = ISO.find_iter(text).last() {
        let s = m.as_str();
        if s.len() > 10 {
            let utc = chrono::DateTime::parse_from_rfc3339(&s.to_uppercase())
                .ok()?
                .with_timezone(&chrono::Utc);
            return Some((
                utc.date_naive(),
                Some(utc.format("%H:%M:%S%.f").to_string()),
            ));
        }
        NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?
    } else if let Some(c) = SLASH.captures_iter(text).last() {
        NaiveDate::from_ymd_opt(c[3].parse().ok()?, c[2].parse().ok()?, c[1].parse().ok()?)?
    } else {
        let c = NAMED.captures_iter(text).last()?;
        NaiveDate::from_ymd_opt(c[3].parse().ok()?, month(&c[2])?, c[1].parse().ok()?)?
    };
    static TIME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|[^\p{L}\d:])(\d{1,2}):([0-5]\d)(?:$|[^\d:])").unwrap());
    let time = match TIME.captures(text) {
        Some(c) => {
            let hour = c[1].parse::<u8>().ok()?;
            if hour >= 24 {
                return None;
            }
            Some(format!("{hour:02}:{}", &c[2]))
        }
        None => None,
    };
    Some((date, time))
}

fn event(text: &str) -> Option<Event> {
    static FRAME: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"^(?:user (?:has (?:a|an) (?:[\p{L}-]+ )?(?:appointment|consultation|meeting|workshop|talk|conference) |moved her |rescheduled (?:her |the )?|will (?:attend |run |present )|travels to )|a usuária (?:tem (?:consulta|reunião|palestra) |remarcou |vai apresentar|vai participar|precisa enviar))"
    ).unwrap()
    });
    static UNSAFE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"\b(?:if|unless|might|may|would|could|not|never|cancelled|canceled|said|quoted|mother|father|visitor|se|talvez|não|nunca|cancelou|disse|mãe|pai|visitante|timezone|local time|america|europe)\b"
    ).unwrap()
    });
    static SIMPLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"^(?:user (?:has a dentist appointment at|moved her dentist appointment from \d{4}-\d{2}-\d{2} to) \d{1,2}:\d{2} on \d{4}-\d{2}-\d{2}|a usuária (?:remarcou (?:a |sua )?consulta (?:no|com o) dentista de \d{1,2}/\d{1,2}/\d{4} para|tem consulta (?:no|com o) dentista (?:às|as)) (?:\d{1,2}:\d{2} (?:em|no dia) \d{1,2}/\d{1,2}/\d{4}|\d{1,2}/\d{1,2}/\d{4} (?:às|as) \d{1,2}:\d{2}))\.$"
    ).unwrap()
    });
    let text = text.trim().to_lowercase();
    if !FRAME.is_match(&text) || UNSAFE.is_match(&text) {
        return None;
    }
    static COMMITMENT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"\b(?:appointment|consultation|meeting|workshop|talk|conference|consulta|palestra|reunião|artigo)\b"
    ).unwrap()
    });
    if !COMMITMENT.is_match(&text) {
        return None;
    }
    let kind = if text.contains("dentist") || text.contains("dentista") {
        "dentist"
    } else if text.contains("palestra") || text.contains("talk") {
        "talk"
    } else {
        "commitment"
    };
    let (date, time) = calendar(&text)?;
    Some(Event {
        date,
        kind: kind.into(),
        time,
        simple_appointment: SIMPLE.is_match(&text),
    })
}

/// A reschedule is a complete old-date -> new-date assertion by the same
/// producer. Its destination need not fall in this query's interval: a stale
/// booking is not revived when the new appointment moves outside next week.
pub fn superseded(old: &Memory, new: &Memory) -> bool {
    if old.pinned
        || old.id == new.id
        || !old.is_live()
        || !new.is_live()
        || !old.has_attested_statement()
        || !new.has_attested_statement()
        || !same_source(old, new)
        || new.last_stated() < old.last_stated()
    {
        return false;
    }
    let (Some(before), Some(after)) = (event(&old.text), event(&new.text)) else {
        return false;
    };
    if before.kind != after.kind || before.kind != "dentist" || before.date == after.date {
        return false;
    }
    let text = new.text.to_lowercase();
    let old_iso = before.date.format("%Y-%m-%d").to_string();
    let old_pt = before.date.format("%d/%m/%Y").to_string();
    (text.starts_with("user moved her dentist appointment from ")
        && text.contains(&format!("from {old_iso} to ")))
        || (text.starts_with("a usuária remarcou a consulta no dentista de ")
            && text.contains(&format!("de {old_pt} para ")))
}

pub fn evidence(query: &str, memory: &Memory, clock: EvaluationClock) -> Option<serde_json::Value> {
    if !scope(query) || !memory.is_live() || !memory.has_attested_statement() {
        return None;
    }
    let event = event(&memory.text)?;
    let today =
        chrono::DateTime::from_timestamp_millis(i64::try_from(clock.now_ms()).ok()?)?.date_naive();
    let start = today.checked_add_signed(Duration::days(
        7 - i64::from(today.weekday().num_days_from_monday()),
    ))?;
    let end = start.checked_add_signed(Duration::days(7))?;
    (event.date >= start && event.date < end).then(|| serde_json::json!({
        "relation":"owned_dated_commitment", "owner":"user", "event_date":event.date.to_string(),
        "clock_date":today.to_string(), "week_start":start.to_string(), "week_end_exclusive":end.to_string(),
        "timezone":"UTC", "event_time_utc":event.time,
        "kind":event.kind, "source_text":memory.text
    }))
}

fn same_source(a: &Memory, b: &Memory) -> bool {
    match (&a.source, &b.source) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a == b || (a.session_id.is_some() && a.session_id == b.session_id && a.agent == b.agent)
        }
        _ => false,
    }
}

/// Typed non-equivalence vetoes fuzzy dedup when dates/details differ. Source
/// identity and pinned records are protected; no label or vector is a witness.
pub fn equivalent(a: &Memory, b: &Memory) -> Option<bool> {
    let (left, right) = (event(&a.text), event(&b.text));
    if left.is_none() && right.is_none() {
        return None;
    }
    Some(match (left, right) {
        (Some(l), Some(r)) => {
            !a.pinned
                && !b.pinned
                && same_source(a, b)
                && ((l.simple_appointment && r.simple_appointment && l == r) || a.text == b.text)
        }
        _ => false,
    })
}
