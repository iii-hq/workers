use crate::{ErrorCode, Question, ScoreLevel};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        #[serde(deserialize_with = "unique_map")]
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        #[serde(deserialize_with = "unique_map")]
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
        #[serde(deserialize_with = "unique_map")]
        legend: BTreeMap<String, ScoreLevel>,
    },
}
impl Answer {
    pub fn as_noul(&self) -> Option<f64> {
        match self {
            Self::Noul { noul } => Some(*noul),
            _ => None,
        }
    }
}

/// Validate the requested type and domain without replacing provider judgments.
pub fn validate_answer(question: &Question, answer: &Answer) -> Result<(), ErrorCode> {
    let valid = match (question, answer) {
        (Question::Noul { .. }, Answer::Noul { noul }) => probability(*noul),
        (
            Question::Choice { criteria, .. },
            Answer::Choice {
                choice,
                probabilities,
                confidence,
            },
        ) => {
            criteria.contains_key(choice)
                && criteria.keys().eq(probabilities.keys())
                && distribution(probabilities)
                && probability(*confidence)
        }
        (
            Question::Score { criteria, .. },
            Answer::Score {
                score,
                probabilities,
                confidence,
                legend,
            },
        ) => {
            let count = criteria.len();
            score.is_finite()
                && count >= 2
                && (0.0..=(count - 1) as f64).contains(score)
                && probabilities.len() == count
                && legend.len() == count
                && (0..count).all(|index| {
                    let key = index.to_string();
                    probabilities.contains_key(&key) && legend.get(&key) == Some(&criteria[index])
                })
                && distribution(probabilities)
                && probability(*confidence)
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ErrorCode::InvalidResponse)
    }
}
fn probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
fn distribution(values: &BTreeMap<String, f64>) -> bool {
    // Decimal rounding in documented provider responses is not corruption.
    !values.is_empty()
        && values.values().all(|value| probability(*value))
        && (values.values().sum::<f64>() - 1.0).abs() <= 0.020_000_001
}
pub(crate) fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map with unique keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = map.next_entry()? {
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate map key"));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Visitor(std::marker::PhantomData))
}
