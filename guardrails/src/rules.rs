//! Pattern tables + Luhn check. Port of roster/workers/guardrails/src/rules.ts.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    #[serde(rename = "pii:email")]
    PiiEmail,
    #[serde(rename = "pii:ssn")]
    PiiSsn,
    #[serde(rename = "pii:credit_card")]
    PiiCreditCard,
    #[serde(rename = "pii:phone")]
    PiiPhone,
    #[serde(rename = "keys:openai")]
    KeysOpenai,
    #[serde(rename = "keys:github")]
    KeysGithub,
    #[serde(rename = "keys:aws")]
    KeysAws,
    #[serde(rename = "keys:slack")]
    KeysSlack,
    #[serde(rename = "jailbreak")]
    Jailbreak,
    #[serde(rename = "toxicity")]
    Toxicity,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PiiEmail => "pii:email",
            Self::PiiSsn => "pii:ssn",
            Self::PiiCreditCard => "pii:credit_card",
            Self::PiiPhone => "pii:phone",
            Self::KeysOpenai => "keys:openai",
            Self::KeysGithub => "keys:github",
            Self::KeysAws => "keys:aws",
            Self::KeysSlack => "keys:slack",
            Self::Jailbreak => "jailbreak",
            Self::Toxicity => "toxicity",
        }
    }
}

pub struct Pattern {
    pub category: Category,
    pub regex: Regex,
}

pub static PII_PATTERNS: Lazy<Vec<Pattern>> = Lazy::new(|| {
    vec![
        Pattern {
            category: Category::PiiEmail,
            regex: Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").unwrap(),
        },
        Pattern {
            category: Category::PiiSsn,
            regex: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
        },
        Pattern {
            category: Category::PiiCreditCard,
            regex: Regex::new(r"\b(?:\d[ -]?){13,19}\b").unwrap(),
        },
        Pattern {
            category: Category::PiiPhone,
            regex: Regex::new(r"\b(?:\+?1[\s.-]?)?\(?\d{3}\)?[\s.-]?\d{3}[\s.-]?\d{4}\b").unwrap(),
        },
    ]
});

pub static KEY_PATTERNS: Lazy<Vec<Pattern>> = Lazy::new(|| {
    vec![
        Pattern {
            category: Category::KeysOpenai,
            regex: Regex::new(r"\bsk-[A-Za-z0-9_-]{20,}\b").unwrap(),
        },
        Pattern {
            category: Category::KeysGithub,
            regex: Regex::new(r"\bghp_[A-Za-z0-9]{36}\b").unwrap(),
        },
        Pattern {
            category: Category::KeysAws,
            regex: Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
        },
        Pattern {
            category: Category::KeysSlack,
            regex: Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b").unwrap(),
        },
    ]
});

pub static JAILBREAK_KEYWORDS: &[&str] = &[
    "ignore previous instructions",
    "ignore all previous instructions",
    "ignore the above",
    "disregard above",
    "disregard previous",
    "pretend you are",
    "pretend to be",
    "act as if",
    "dan mode",
    "developer mode enabled",
    "system prompt",
    "reveal your prompt",
    "leak your instructions",
    "jailbreak",
    "bypass safety",
];

pub static TOXICITY_TERMS: &[&str] = &[
    "kill yourself",
    "kys",
    "idiot",
    "moron",
    "stupid",
    "retard",
    "scum",
    "trash",
    "hate you",
    "shut up",
    "die",
    "worthless",
    "pathetic",
    "loser",
    "bastard",
    "damn you",
    "screw you",
    "go die",
    "asshole",
    "bitch",
];

pub fn luhn_valid(input: &str) -> bool {
    let digits: Vec<u8> = input
        .chars()
        .filter_map(|c| c.to_digit(10))
        .map(|d| d as u8)
        .collect();
    if digits.len() < 13 || digits.len() > 19 {
        return false;
    }
    let mut sum = 0u32;
    let mut alt = false;
    for &n in digits.iter().rev() {
        let mut x = n as u32;
        if alt {
            x *= 2;
            if x > 9 {
                x -= 9;
            }
        }
        sum += x;
        alt = !alt;
    }
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_regex_matches_typical() {
        assert!(PII_PATTERNS[0]
            .regex
            .is_match("contact me at user@example.com please"));
    }

    #[test]
    fn aws_key_regex_matches() {
        let key = format!("AKIA{}", "X".repeat(16));
        assert!(KEY_PATTERNS[2].regex.is_match(&key));
    }

    #[test]
    fn luhn_valid_accepts_known_good() {
        // 4111 1111 1111 1111 is a Luhn-valid Visa test number.
        assert!(luhn_valid("4111 1111 1111 1111"));
    }

    #[test]
    fn luhn_valid_rejects_bad() {
        assert!(!luhn_valid("4111 1111 1111 1112"));
    }
}
