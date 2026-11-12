//! The rules for the global account (agent-accounts D14, §3.6): when its
//! week (or, with `five_hour`, its 5-hour window) runs low, suggest another
//! account or switch to it; after switching by itself, switch back once the
//! old account's window resets when asked to.
//!
//! "Low" grows in steps, each a `reason`: `week_80` (80 % of the week),
//! `week_90`, `five_hour` (the 5-hour window at 100 %, only with
//! `five_hour`) and `limit` (the week at 100 %). The rules act from the step
//! `at` names. A dismissed suggestion stays away until the old account's
//! window resets or a higher step is reached.
//!
//! The target is the first account of `order` that is signed in and not low,
//! then the one with most room this week; an account whose sign-in expired
//! is taken only when no other has room. An account with no reading yet
//! counts as having room, after those with a reading.

use std::collections::HashMap;

use super::registry::{Dismissal, Registry};
use crate::model::{ClaudeAccount, ClaudeUsage, RulesMode, Suggestion, SwitchRules, Threshold};

const FIVE_HOURS_MS: u64 = 5 * 3_600_000;
const WEEK_MS: u64 = 7 * 24 * 3_600_000;

/// How far down a reason is.
pub fn severity(reason: &str) -> u8 {
    match reason {
        "week_80" => 1,
        "week_90" => 2,
        "five_hour" => 3,
        "limit" => 4,
        _ => 0,
    }
}

fn threshold_percent(at: Threshold) -> f64 {
    match at {
        Threshold::Week80 => 80.0,
        Threshold::Week90 => 90.0,
        Threshold::Limit => 100.0,
    }
}

/// Why an account with `usage` counts as low under `rules`, if it does.
pub fn low_reason(usage: Option<&ClaudeUsage>, rules: &SwitchRules) -> Option<&'static str> {
    let usage = usage?;
    let mut reason = None;
    if let Some(week) = usage.week.as_ref().map(|limit| limit.used_percent) {
        if week >= 100.0 {
            reason = Some("limit");
        } else if week >= threshold_percent(rules.at) {
            reason = Some(if week >= 90.0 { "week_90" } else { "week_80" });
        }
    }
    let five_full = usage
        .five_hour
        .as_ref()
        .is_some_and(|limit| limit.used_percent >= 100.0);
    if rules.five_hour && five_full && reason.is_none_or(|r| severity(r) < severity("five_hour")) {
        reason = Some("five_hour");
    }
    reason
}

/// When the window behind `reason` resets for `usage`, or a guess.
pub fn reset_ms(usage: Option<&ClaudeUsage>, reason: &str, now_ms: u64) -> u64 {
    let (limit, fallback) = if reason == "five_hour" {
        (usage.and_then(|u| u.five_hour.as_ref()), FIVE_HOURS_MS)
    } else {
        (usage.and_then(|u| u.week.as_ref()), WEEK_MS)
    };
    limit
        .and_then(|limit| limit.resets_at_ms)
        .filter(|at| *at > now_ms)
        .unwrap_or(now_ms + fallback)
}

/// The account to move the global role to, other than `global`.
pub fn pick_target(
    accounts: &[ClaudeAccount],
    global: &str,
    rules: &SwitchRules,
) -> Option<String> {
    let eligible: Vec<&ClaudeAccount> = accounts
        .iter()
        .filter(|account| account.id != global && account.signed_in)
        .filter(|account| low_reason(account.usage.as_ref(), rules).is_none())
        .collect();
    let fresh: Vec<&ClaudeAccount> = eligible
        .iter()
        .copied()
        .filter(|account| !account.token_expired)
        .collect();
    let pool = if fresh.is_empty() { eligible } else { fresh };
    if let Some(first) = rules
        .order
        .iter()
        .find_map(|id| pool.iter().find(|account| &account.id == id))
    {
        return Some(first.id.clone());
    }
    let room = |account: &ClaudeAccount| {
        account
            .usage
            .as_ref()
            .and_then(|usage| usage.week.as_ref())
            .map(|week| 100.0 - week.used_percent)
    };
    let mut best: Option<(&ClaudeAccount, Option<f64>)> = None;
    for account in pool {
        let this = room(account);
        let better = match (&best, this) {
            (None, _) => true,
            (Some((_, None)), Some(_)) => true,
            (Some((_, Some(old))), Some(new)) => new > *old,
            _ => false,
        };
        if better {
            best = Some((account, this));
        }
    }
    best.map(|(account, _)| account.id.clone())
}

/// A suggestion's id: what `tmux::accounts::dismiss` needs back.
pub fn suggestion_id(from: &str, reason: &str, to: &str) -> String {
    format!("{from}:{reason}:{to}")
}

/// `(from, reason)` of a suggestion id.
pub fn parse_suggestion_id(id: &str) -> Option<(String, String)> {
    let mut parts = id.split(':');
    let (from, reason, _to) = (parts.next()?, parts.next()?, parts.next()?);
    (parts.next().is_none() && severity(reason) > 0).then(|| (from.to_string(), reason.to_string()))
}

fn dismissed(registry: &Registry, from: &str, reason: &str, now_ms: u64) -> bool {
    registry.dismissed.iter().any(|dismissal| {
        dismissal.from == from
            && severity(&dismissal.reason) >= severity(reason)
            && now_ms < dismissal.until_ms
    })
}

/// The suggestion for the feed (mode `suggest`). `first_seen` remembers
/// when each `from:reason` began.
pub fn suggestion(
    registry: &Registry,
    accounts: &[ClaudeAccount],
    following: u32,
    now_ms: u64,
    first_seen: &mut HashMap<String, u64>,
) -> Option<Suggestion> {
    let rules = &registry.rules;
    let global = accounts
        .iter()
        .find(|account| account.id == registry.global);
    let reason = global.and_then(|g| low_reason(g.usage.as_ref(), rules));
    let key = reason.map(|reason| format!("{}:{reason}", registry.global));
    first_seen.retain(|seen, _| Some(seen) == key.as_ref());
    if rules.mode != RulesMode::Suggest {
        return None;
    }
    let reason = reason?;
    let at_ms = *first_seen.entry(key?).or_insert(now_ms);
    if dismissed(registry, &registry.global, reason, now_ms) {
        return None;
    }
    let to = pick_target(accounts, &registry.global, rules)?;
    Some(Suggestion {
        id: suggestion_id(&registry.global, reason, &to),
        from: registry.global.clone(),
        to,
        reason: reason.to_string(),
        following,
        at_ms,
    })
}

/// The dismissal "Not now" leaves for suggestion `id`: until the old
/// account's window resets.
pub fn dismissal(id: &str, accounts: &[ClaudeAccount], now_ms: u64) -> Result<Dismissal, String> {
    let (from, reason) =
        parse_suggestion_id(id).ok_or_else(|| format!("\"{id}\" is not a suggestion."))?;
    let usage = accounts
        .iter()
        .find(|account| account.id == from)
        .and_then(|account| account.usage.as_ref());
    Ok(Dismissal {
        until_ms: reset_ms(usage, &reason, now_ms),
        from,
        reason,
    })
}

/// A switch the rules call for now: `(to, reason, back_at_ms)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub to: String,
    pub reason: String,
    pub back_at_ms: Option<u64>,
}

/// What mode `switch` (and `back_after_reset`) does now, if anything.
pub fn decide(registry: &Registry, accounts: &[ClaudeAccount], now_ms: u64) -> Option<Decision> {
    let rules = &registry.rules;
    // Only mode `switch` switches, back after a reset included.
    if rules.mode != RulesMode::Switch {
        return None;
    }
    // Back to the account it switched away from, once that one has reset;
    // a switch someone made since cleared `back_at_ms` (and the global
    // account must still be the one the rules switched to).
    if let Some(last) = registry.last_switch.as_ref() {
        // Turned off since: the switch back it planned is off too.
        let back_due = rules.back_after_reset
            && last.auto
            && last.reason.as_deref() != Some("back")
            && registry.global == last.to
            && last.back_at_ms.is_some_and(|at| now_ms >= at);
        if back_due {
            let ready = accounts.iter().any(|account| {
                account.id == last.from
                    && account.signed_in
                    && !account.token_expired
                    && low_reason(account.usage.as_ref(), rules).is_none()
            });
            if ready {
                return Some(Decision {
                    to: last.from.clone(),
                    reason: "back".to_string(),
                    back_at_ms: None,
                });
            }
        }
    }
    let global = accounts
        .iter()
        .find(|account| account.id == registry.global)?;
    let reason = low_reason(global.usage.as_ref(), rules)?;
    let to = pick_target(accounts, &registry.global, rules)?;
    Some(Decision {
        to,
        reason: reason.to_string(),
        back_at_ms: rules
            .back_after_reset
            .then(|| reset_ms(global.usage.as_ref(), reason, now_ms)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountSwitch, Limit};

    fn limit(used: f64, resets: u64) -> Option<Limit> {
        Some(Limit {
            used_percent: used,
            resets_at_ms: Some(resets),
            reset_since_read: false,
        })
    }

    fn account(id: &str, week: Option<f64>, five: Option<f64>) -> ClaudeAccount {
        ClaudeAccount {
            id: id.into(),
            name: id.into(),
            color: "blue".into(),
            dir: format!("/a/{id}"),
            dir_display: format!("/a/{id}"),
            is_default: id == "default",
            email: None,
            organization: None,
            plan: None,
            tier: None,
            signed_in: true,
            token_expired: false,
            shared: vec![],
            panes: 0,
            usage: (week.is_some() || five.is_some()).then(|| ClaudeUsage {
                five_hour: five.and_then(|f| limit(f, 5_000)),
                week: week.and_then(|w| limit(w, 9_000)),
                week_opus: None,
                week_sonnet: None,
                week_models: vec![],
                extra: None,
                as_of_ms: 1,
                error: None,
            }),
        }
    }

    fn rules(mode: RulesMode, at: Threshold) -> SwitchRules {
        SwitchRules {
            mode,
            at,
            ..SwitchRules::default()
        }
    }

    #[test]
    fn low_grows_in_steps() {
        let at90 = rules(RulesMode::Suggest, Threshold::Week90);
        let usage = |week, five| account("x", Some(week), Some(five)).usage;
        assert_eq!(low_reason(usage(89.0, 10.0).as_ref(), &at90), None);
        assert_eq!(
            low_reason(usage(91.0, 10.0).as_ref(), &at90),
            Some("week_90")
        );
        assert_eq!(
            low_reason(usage(100.0, 10.0).as_ref(), &at90),
            Some("limit")
        );
        let at80 = rules(RulesMode::Suggest, Threshold::Week80);
        assert_eq!(
            low_reason(usage(85.0, 10.0).as_ref(), &at80),
            Some("week_80")
        );
        assert_eq!(
            low_reason(usage(95.0, 10.0).as_ref(), &at80),
            Some("week_90")
        );
        let at_limit = rules(RulesMode::Suggest, Threshold::Limit);
        assert_eq!(low_reason(usage(99.0, 10.0).as_ref(), &at_limit), None);
        // The 5-hour window only counts when asked, and above week_90.
        assert_eq!(low_reason(usage(50.0, 100.0).as_ref(), &at90), None);
        let five = SwitchRules {
            five_hour: true,
            ..at90.clone()
        };
        assert_eq!(
            low_reason(usage(50.0, 100.0).as_ref(), &five),
            Some("five_hour")
        );
        assert_eq!(
            low_reason(usage(95.0, 100.0).as_ref(), &five),
            Some("five_hour")
        );
        assert_eq!(
            low_reason(usage(100.0, 100.0).as_ref(), &five),
            Some("limit")
        );
        assert_eq!(low_reason(None, &five), None, "no reading, no reason");
    }

    #[test]
    fn targets_follow_the_order_then_the_most_room() {
        let mut at90 = rules(RulesMode::Suggest, Threshold::Week90);
        let accounts = vec![
            account("default", Some(95.0), None),
            account("personal", Some(40.0), None),
            account("side", Some(10.0), None),
            account("full", Some(92.0), None),
        ];
        assert_eq!(
            pick_target(&accounts, "default", &at90).as_deref(),
            Some("side")
        );
        at90.order = vec!["full".into(), "personal".into()];
        assert_eq!(
            pick_target(&accounts, "default", &at90).as_deref(),
            Some("personal"),
            "the first of the order that has room"
        );
        // Signed out never; expired only when nothing else has room.
        let mut accounts = accounts;
        accounts[1].signed_in = false;
        accounts[2].token_expired = true;
        assert_eq!(
            pick_target(&accounts, "default", &at90).as_deref(),
            Some("side")
        );
        accounts.push(account("fresh", None, None));
        assert_eq!(
            pick_target(&accounts, "default", &at90).as_deref(),
            Some("fresh"),
            "unread but signed in beats expired"
        );
        accounts.push(account("roomy", Some(5.0), None));
        assert_eq!(
            pick_target(&accounts, "default", &at90).as_deref(),
            Some("roomy"),
            "a reading with room beats no reading"
        );
        assert_eq!(pick_target(&accounts[..1], "default", &at90), None);
    }

    #[test]
    fn suggestions_come_and_go_and_are_dismissed() {
        let mut registry = Registry {
            rules: rules(RulesMode::Suggest, Threshold::Week90),
            ..Registry::default()
        };
        let accounts = vec![
            account("default", Some(91.0), None),
            account("personal", Some(12.0), None),
        ];
        let mut seen = HashMap::new();
        let s = suggestion(&registry, &accounts, 2, 100, &mut seen).unwrap();
        assert_eq!(
            (s.from.as_str(), s.to.as_str(), s.reason.as_str()),
            ("default", "personal", "week_90")
        );
        assert_eq!((s.following, s.at_ms), (2, 100));
        assert_eq!(s.id, "default:week_90:personal");
        let again = suggestion(&registry, &accounts, 2, 500, &mut seen).unwrap();
        assert_eq!(again.at_ms, 100, "first seen");
        // Not now: away until the week resets, back at a higher step.
        let d = dismissal(&s.id, &accounts, 600).unwrap();
        assert_eq!(
            (d.from.as_str(), d.reason.as_str(), d.until_ms),
            ("default", "week_90", 9_000)
        );
        registry.dismissed.push(d);
        assert!(suggestion(&registry, &accounts, 2, 700, &mut seen).is_none());
        let full = vec![
            account("default", Some(100.0), None),
            account("personal", Some(12.0), None),
        ];
        assert_eq!(
            suggestion(&registry, &full, 2, 700, &mut seen)
                .unwrap()
                .reason,
            "limit"
        );
        assert!(
            suggestion(&registry, &accounts, 2, 9_000, &mut seen).is_some(),
            "reset"
        );
        // Under the threshold, or off: nothing.
        let fine = vec![
            account("default", Some(50.0), None),
            account("personal", Some(12.0), None),
        ];
        assert!(suggestion(&registry, &fine, 2, 700, &mut seen).is_none());
        registry.rules.mode = RulesMode::Off;
        assert!(suggestion(&registry, &full, 2, 700, &mut seen).is_none());
        assert!(dismissal("nonsense", &accounts, 1).is_err());
        assert_eq!(
            parse_suggestion_id("default:five_hour:side"),
            Some(("default".into(), "five_hour".into()))
        );
    }

    #[test]
    fn switch_mode_switches_and_comes_back() {
        let mut registry = Registry {
            rules: SwitchRules {
                back_after_reset: true,
                ..rules(RulesMode::Switch, Threshold::Week90)
            },
            ..Registry::default()
        };
        let low = vec![
            account("default", Some(93.0), None),
            account("personal", Some(12.0), None),
        ];
        let decision = decide(&registry, &low, 1_000).unwrap();
        assert_eq!(
            decision,
            Decision {
                to: "personal".into(),
                reason: "week_90".into(),
                back_at_ms: Some(9_000)
            }
        );
        // Suggest mode never switches.
        registry.rules.mode = RulesMode::Suggest;
        assert!(decide(&registry, &low, 1_000).is_none());
        registry.rules.mode = RulesMode::Switch;
        // After the switch: back once default's week reset.
        registry.global = "personal".into();
        registry.last_switch = Some(AccountSwitch {
            from: "default".into(),
            to: "personal".into(),
            reason: Some("week_90".into()),
            auto: true,
            moved: vec![],
            after_turn: vec![],
            at_ms: 1_000,
            back_at_ms: Some(9_000),
        });
        assert!(decide(&registry, &low, 5_000).is_none(), "not yet");
        assert!(
            decide(&registry, &low, 9_500).is_none(),
            "default still low"
        );
        let reset = vec![
            account("default", Some(0.0), None),
            account("personal", Some(12.0), None),
        ];
        assert_eq!(decide(&registry, &reset, 9_500).unwrap().to, "default");
        // Turned off since the switch: no going back.
        registry.rules.back_after_reset = false;
        assert!(decide(&registry, &reset, 9_500).is_none());
        registry.rules.back_after_reset = true;
        // Suggest mode never switches back either.
        registry.rules.mode = RulesMode::Suggest;
        assert!(decide(&registry, &reset, 9_500).is_none());
        registry.rules.mode = RulesMode::Switch;
        // A switch by hand clears the way back, even once the global one is
        // the rules' target again (personal → side → personal by hand).
        let mut by_hand = registry.clone();
        by_hand.last_switch.as_mut().unwrap().back_at_ms = None;
        assert!(decide(&by_hand, &reset, 9_500).is_none());
        // Someone switched by hand since: no going back.
        registry.global = "default".into();
        assert!(decide(&registry, &reset, 9_500).is_none());
    }
}
