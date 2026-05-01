//! UTC-anchored period math for budgets. Direct port of
//! roster/workers/llm-budget/src/periods.ts.

use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    Day,
    Week,
    Month,
}

const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

fn from_ms(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().expect("valid ms")
}

pub fn utc_day_start(ms: i64) -> i64 {
    let d = from_ms(ms);
    Utc.with_ymd_and_hms(d.year(), d.month(), d.day(), 0, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis()
}

pub fn utc_week_start(ms: i64) -> i64 {
    let day_ms = utc_day_start(ms);
    let dow = from_ms(day_ms).weekday().num_days_from_monday() as i64;
    day_ms - dow * MS_PER_DAY
}

pub fn utc_month_start(ms: i64) -> i64 {
    let d = from_ms(ms);
    Utc.with_ymd_and_hms(d.year(), d.month(), 1, 0, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis()
}

pub fn period_start(p: Period, ms: i64) -> i64 {
    match p {
        Period::Day => utc_day_start(ms),
        Period::Week => utc_week_start(ms),
        Period::Month => utc_month_start(ms),
    }
}

pub fn next_period_start(p: Period, start: i64) -> i64 {
    match p {
        Period::Day => start + MS_PER_DAY,
        Period::Week => start + 7 * MS_PER_DAY,
        Period::Month => {
            let d = from_ms(start);
            let (y, m) = if d.month() == 12 {
                (d.year() + 1, 1)
            } else {
                (d.year(), d.month() + 1)
            };
            Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0)
                .single()
                .unwrap()
                .timestamp_millis()
        }
    }
}

pub fn period_key(p: Period, start: i64) -> String {
    let d = from_ms(start);
    match p {
        Period::Day => format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()),
        Period::Week => {
            let iso = d.iso_week();
            format!("{:04}-W{:02}", iso.year(), iso.week())
        }
        Period::Month => format!("{:04}-{:02}", d.year(), d.month()),
    }
}

pub fn days_elapsed(start: i64, now: i64) -> f64 {
    let v = (now - start) as f64 / MS_PER_DAY as f64;
    if v < 1.0 {
        1.0
    } else {
        v
    }
}

pub fn days_remaining(now: i64, resets_at: i64) -> f64 {
    let v = (resets_at - now) as f64 / MS_PER_DAY as f64;
    if v < 0.0 {
        0.0
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn ms(year: i32, m: u32, d: u32, h: u32) -> i64 {
        Utc.with_ymd_and_hms(year, m, d, h, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn day_start_zeros_time() {
        let t = ms(2026, 4, 30, 17);
        let s = utc_day_start(t);
        assert_eq!(from_ms(s).hour(), 0);
        assert_eq!(from_ms(s).day(), 30);
    }

    #[test]
    fn week_start_anchors_on_monday() {
        // 2026-04-30 is a Thursday; week start is 2026-04-27 Monday.
        let t = ms(2026, 4, 30, 17);
        let ws = utc_week_start(t);
        assert_eq!(from_ms(ws).day(), 27);
        assert_eq!(from_ms(ws).weekday().num_days_from_monday(), 0);
    }

    #[test]
    fn month_start_first_of_month_zero_time() {
        let t = ms(2026, 4, 30, 17);
        let s = utc_month_start(t);
        assert_eq!(from_ms(s).day(), 1);
        assert_eq!(from_ms(s).month(), 4);
    }

    #[test]
    fn next_month_handles_year_wrap() {
        let dec_start = ms(2026, 12, 1, 0);
        let jan_start = next_period_start(Period::Month, dec_start);
        assert_eq!(from_ms(jan_start).year(), 2027);
        assert_eq!(from_ms(jan_start).month(), 1);
    }

    #[test]
    fn period_key_day_format() {
        let s = utc_day_start(ms(2026, 4, 30, 17));
        assert_eq!(period_key(Period::Day, s), "2026-04-30");
    }

    #[test]
    fn period_key_week_uses_iso() {
        let s = utc_week_start(ms(2026, 4, 30, 17));
        let k = period_key(Period::Week, s);
        assert!(k.starts_with("2026-W"), "got {k}");
    }

    #[test]
    fn period_key_month_format() {
        let s = utc_month_start(ms(2026, 4, 30, 17));
        assert_eq!(period_key(Period::Month, s), "2026-04");
    }

    #[test]
    fn days_elapsed_floors_at_one() {
        let now = ms(2026, 4, 30, 1);
        assert!((days_elapsed(now, now) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn days_remaining_clamps_at_zero() {
        let now = ms(2026, 4, 30, 1);
        let earlier = now - MS_PER_DAY;
        assert!(days_remaining(now, earlier).abs() < f64::EPSILON);
    }
}
