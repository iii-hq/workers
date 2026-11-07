use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Default, Deserialize)]
pub(super) struct Labels {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub admissible: Vec<String>,
    #[serde(default)]
    pub equivalents: Vec<Vec<String>>,
    #[serde(default)]
    pub forbidden: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct Score {
    pub relevant: usize,
    pub noise: Vec<String>,
    pub missing_required: Vec<String>,
    pub forbidden_selected: Vec<String>,
    pub duplicate_equivalents: Vec<Vec<String>>,
    pub precision: Option<f64>,
    pub required_recall: Option<f64>,
    pub expected_abstention: bool,
    pub ok: bool,
}

/// Acceptance policy separate from factual recall: similarity discovers
/// candidates but never proves a relation. Both lexical and hybrid absence
/// may justify abstention; original labels and factual score stay unchanged.
#[derive(Debug, Serialize)]
pub(super) struct InjectionPolicy {
    pub factual: Score,
    pub abstention_without_support: bool,
    pub unsupported_required: Vec<String>,
    pub missed_supported: Vec<String>,
    pub unrelated_injected: Vec<String>,
    pub duplicated: Vec<Vec<String>>,
    pub forbidden_selected: Vec<String>,
    pub ungrounded_injection: bool,
    pub unsupported_injected: Vec<String>,
    pub relevance_ok: bool,
    pub integral_ok: bool,
}

/// Single-candidate audit for focused tests. Named single-fact support must
/// instead be established against its full candidate family by admission_reasons.
pub(super) fn lexical_support(e: &super::Evidence) -> bool {
    let (matched, terms) = if e.requested_slot_spans.is_some() {
        (&e.relation_matched, e.relation_terms)
    } else {
        (&e.matched, e.terms)
    };
    !e.subject_conflict
        && !e.attribute_conflict
        && !e.predicate_conflict
        && (!e.personal_scope_required || e.personal_scope_matched)
        && (!e.target_scope_required || e.target_scope_matched)
        && (e.attribute_match
            || e.personal_match
            || (!matched.is_empty() && matched.len() >= terms.min(2)))
}

/// A pair attested by the unchanged fixture's expected/stale relationship,
/// shared subject/attribute and statement order. Not inferred from case names.
#[derive(Debug, Serialize)]
pub(super) struct UpdateProof {
    old: String,
    current: String,
    source: String,
}

impl UpdateProof {
    pub fn from_original(
        old: &super::Memory,
        current: &super::Memory,
        expected: &[String],
        stale: &[String],
        source: &str,
    ) -> Option<Self> {
        let shared_subject = old
            .entities
            .iter()
            .any(|entity| entity != "user" && current.entities.contains(entity));
        let shared_attribute = old.tags.iter().any(|tag| current.tags.contains(tag));
        (old.id != current.id
            && expected.contains(&current.id)
            && stale.contains(&old.id)
            && shared_subject
            && shared_attribute
            && current.last_stated() > old.last_stated()
            && current.text != old.text
            && !source.is_empty())
        .then(|| Self {
            old: old.id.clone(),
            current: current.id.clone(),
            source: source.into(),
        })
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Responsibility {
    pub related_superseded: Vec<String>,
    pub equivalent_duplicates: Vec<Vec<String>>,
    pub third_party: Vec<String>,
    pub missing_required: Vec<String>,
    pub proof_sources: Vec<String>,
    pub temporal_only: bool,
}

impl Labels {
    /// Attribute responsibility only. Score/noise/forbidden/integral stay red.
    pub fn responsibility(&self, selected: &[String], proofs: &[UpdateProof]) -> Responsibility {
        let score = self.score(selected);
        let mut related_superseded = Vec::new();
        let mut third_party = Vec::new();
        let mut proof_sources = Vec::new();
        for id in &score.noise {
            let proof = proofs.iter().find(|proof| {
                &proof.old == id
                    && proof.old != proof.current
                    && !proof.source.is_empty()
                    && self.forbidden.contains(id)
                    && selected.contains(&proof.current)
                    && self.required.iter().any(|required| {
                        self.alternatives(required)
                            .contains(&proof.current.as_str())
                    })
            });
            if let Some(proof) = proof {
                related_superseded.push(id.clone());
                proof_sources.push(proof.source.clone());
            } else {
                third_party.push(id.clone());
            }
        }
        let temporal_only = score.missing_required.is_empty()
            && third_party.is_empty()
            && (!related_superseded.is_empty() || !score.duplicate_equivalents.is_empty())
            && score
                .forbidden_selected
                .iter()
                .all(|id| related_superseded.contains(id));
        Responsibility {
            related_superseded,
            third_party,
            equivalent_duplicates: score.duplicate_equivalents,
            missing_required: score.missing_required,
            proof_sources,
            temporal_only,
        }
    }

    /// Capability-aware injection acceptance. Factual precision/recall are
    /// always scored against the unchanged labels; abstention is not recall.
    pub fn injection_policy(
        &self,
        selected: &[String],
        supported: &HashSet<String>,
        _query_vector_present: bool,
    ) -> InjectionPolicy {
        let factual = self.score(selected);
        let supported_required: Vec<&String> = self
            .required
            .iter()
            .filter(|key| {
                self.alternatives(key)
                    .iter()
                    .any(|a| supported.contains(*a))
            })
            .collect();
        let unsupported_required = self
            .required
            .iter()
            .filter(|key| !supported_required.contains(key))
            .cloned()
            .collect::<Vec<_>>();
        let missed_supported = supported_required
            .iter()
            .filter(|key| factual.missing_required.contains(key))
            .map(|key| (**key).clone())
            .collect::<Vec<_>>();
        let abstention_without_support =
            !self.required.is_empty() && supported_required.is_empty() && selected.is_empty();
        let unsupported_injected = selected
            .iter()
            .filter(|id| !supported.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let ungrounded_injection = !unsupported_injected.is_empty();
        // Forbidden facts may be unrelated layer leakage, not only stale.
        // Keep them blocking here; a caller must prove a temporal-only cause.
        let unrelated_injected = factual.noise.clone();
        let relevance_ok =
            missed_supported.is_empty() && unrelated_injected.is_empty() && !ungrounded_injection;
        let integral_ok = relevance_ok
            && factual.forbidden_selected.is_empty()
            && factual.duplicate_equivalents.is_empty();
        InjectionPolicy {
            abstention_without_support,
            unsupported_required,
            missed_supported,
            unrelated_injected,
            duplicated: factual.duplicate_equivalents.clone(),
            forbidden_selected: factual.forbidden_selected.clone(),
            ungrounded_injection,
            unsupported_injected,
            relevance_ok,
            integral_ok,
            factual,
        }
    }

    fn alternatives<'a>(&'a self, key: &'a str) -> Vec<&'a str> {
        self.equivalents
            .iter()
            .find(|g| g.iter().any(|k| k == key))
            .map(|g| g.iter().map(String::as_str).collect())
            .unwrap_or_else(|| vec![key])
    }

    pub fn validate(&self, known: &HashSet<String>) {
        let mut named = HashSet::new();
        for key in self
            .required
            .iter()
            .chain(&self.admissible)
            .chain(&self.forbidden)
        {
            assert!(known.contains(key), "unknown label {key}");
            assert!(named.insert(key), "conflicting/repeated label {key}");
        }
        let mut grouped = HashSet::new();
        let mut required_groups = HashMap::new();
        for (i, group) in self.equivalents.iter().enumerate() {
            assert!(group.len() >= 2, "equivalence needs two distinct facts");
            assert!(
                group
                    .iter()
                    .any(|k| self.required.contains(k) || self.admissible.contains(k)),
                "unused equivalence"
            );
            for key in group {
                assert!(known.contains(key) && !self.forbidden.contains(key));
                assert!(grouped.insert(key), "overlapping equivalences");
                if self.required.contains(key) {
                    assert!(
                        required_groups.insert(i, key).is_none(),
                        "redundant mandatory equivalents"
                    );
                }
            }
        }
    }

    pub fn score(&self, selected: &[String]) -> Score {
        let allowed: HashSet<&str> = self
            .required
            .iter()
            .chain(&self.admissible)
            .flat_map(|k| self.alternatives(k))
            .collect();
        let relevant = selected
            .iter()
            .filter(|k| allowed.contains(k.as_str()))
            .count();
        let noise: Vec<String> = selected
            .iter()
            .filter(|k| !allowed.contains(k.as_str()))
            .cloned()
            .collect();
        let missing_required: Vec<String> = self
            .required
            .iter()
            .filter(|k| {
                !self
                    .alternatives(k)
                    .iter()
                    .any(|a| selected.iter().any(|s| s.as_str() == *a))
            })
            .cloned()
            .collect();
        let forbidden_selected: Vec<String> = selected
            .iter()
            .filter(|k| self.forbidden.contains(k))
            .cloned()
            .collect();
        let duplicate_equivalents: Vec<Vec<String>> = self
            .equivalents
            .iter()
            .filter_map(|g| {
                let held: Vec<String> =
                    g.iter().filter(|k| selected.contains(k)).cloned().collect();
                (held.len() > 1).then_some(held)
            })
            .collect();
        let ok = noise.is_empty()
            && missing_required.is_empty()
            && forbidden_selected.is_empty()
            && duplicate_equivalents.is_empty();
        Score {
            relevant,
            precision: (!selected.is_empty()).then(|| relevant as f64 / selected.len() as f64),
            required_recall: (!self.required.is_empty()).then(|| {
                (self.required.len() - missing_required.len()) as f64 / self.required.len() as f64
            }),
            expected_abstention: allowed.is_empty(),
            noise,
            missing_required,
            forbidden_selected,
            duplicate_equivalents,
            ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn strings(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }
    fn labels() -> Labels {
        Labels {
            required: strings(&["window"]),
            admissible: strings(&["holiday"]),
            equivalents: vec![strings(&["window", "window-paraphrase"])],
            ..Labels::default()
        }
    }
    #[test]
    fn temporal_attribution_fake_layer_update_is_not_an_original_update() {
        let base = super::super::Memory {
            id: "hidden".into(),
            text: "Hidden layer format".into(),
            entities: vec!["user".into()],
            tags: vec!["format".into()],
            confidence: serde_json::from_value(serde_json::json!("stated")).unwrap(),
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: 1,
            updated_at: 1,
            stated_at: Some(1),
            invalid_at: None,
            superseded_by: None,
            revision: 0,
        };
        let current = super::super::Memory {
            id: "layer".into(),
            text: "Different layer format".into(),
            stated_at: Some(2),
            ..base.clone()
        };
        assert!(UpdateProof::from_original(
            &base,
            &current,
            &strings(&["layer"]),
            &[],
            "fake update"
        )
        .is_none());
        assert!(UpdateProof::from_original(
            &base,
            &current,
            &strings(&["layer"]),
            &strings(&["hidden"]),
            "fake update"
        )
        .is_none());
        let old = super::super::Memory {
            entities: strings(&["build-system"]),
            ..base
        };
        let new = super::super::Memory {
            entities: strings(&["build-system"]),
            ..current
        };
        assert!(UpdateProof::from_original(
            &old,
            &new,
            &strings(&["layer"]),
            &strings(&["hidden"]),
            "original expected/stale fixture"
        )
        .is_some());
    }

    #[test]
    fn temporal_attribution_requires_current_no_third_party_and_attested_update() {
        let l = Labels {
            required: strings(&["current"]),
            forbidden: strings(&["old"]),
            ..Labels::default()
        };
        let proofs = [UpdateProof {
            old: "old".into(),
            current: "current".into(),
            source: "original fixture expected/stale pair + subject + chronology".into(),
        }];
        let selected = strings(&["current", "old"]);
        let attribution = l.responsibility(&selected, &proofs);
        assert!(attribution.temporal_only);
        assert_eq!(attribution.related_superseded, strings(&["old"]));
        assert!(attribution.third_party.is_empty());
        assert!(!l.score(&selected).ok);
        assert_eq!(l.score(&selected).noise, strings(&["old"]));
        assert!(!l.responsibility(&strings(&["old"]), &proofs).temporal_only);
        assert!(
            !l.responsibility(&strings(&["current", "old", "foreign"]), &proofs)
                .temporal_only
        );
        assert!(!l.responsibility(&selected, &[]).temporal_only);
        let layer = Labels {
            required: strings(&["layer"]),
            forbidden: strings(&["hidden"]),
            ..Labels::default()
        };
        assert!(
            !layer
                .responsibility(&strings(&["layer", "hidden"]), &proofs)
                .temporal_only
        );
        let duplicate = labels();
        assert!(
            duplicate
                .responsibility(&strings(&["window", "window-paraphrase"]), &[])
                .temporal_only
        );
        assert!(
            !duplicate
                .responsibility(&strings(&["window", "window-paraphrase", "foreign"]), &[])
                .temporal_only
        );
    }

    #[test]
    fn fallback_policy_abstains_without_relabeling_factual_recall() {
        let l = labels();
        let p = l.injection_policy(&[], &HashSet::new(), false);
        assert!(p.abstention_without_support && p.integral_ok);
        assert!(!p.factual.ok);
        assert_eq!(p.factual.required_recall, Some(0.0));
        assert_eq!(p.unsupported_required, strings(&["window"]));
        assert!(
            !l.injection_policy(&strings(&["coffee"]), &HashSet::new(), false)
                .integral_ok
        );
        assert!(
            !l.injection_policy(&strings(&["window"]), &HashSet::new(), false)
                .integral_ok
        );
    }

    #[test]
    fn hybrid_policy_requires_sufficient_relation_even_for_unknown_evidence() {
        let partial = super::super::Evidence {
            matched: vec!["detail".into()],
            terms: 4,
            ..super::super::Evidence::default()
        };
        assert!(!lexical_support(&partial));
        let slot_only = super::super::Evidence {
            relation_matched: vec![],
            requested_slot_spans: Some(std::iter::once(0..2).collect()),
            ..partial
        };
        assert!(!lexical_support(&slot_only));
    }

    #[test]
    fn fallback_policy_never_waives_cached_noise_or_supported_detail() {
        let mut l = labels();
        l.required.push("restriction".into());
        let supported = strings(&["window"]).into_iter().collect();
        let partial = l.injection_policy(&strings(&["window"]), &supported, false);
        assert!(partial.integral_ok);
        let lost = l.injection_policy(&[], &supported, false);
        assert!(!lost.integral_ok && !lost.abstention_without_support);
        assert_eq!(lost.missed_supported, strings(&["window"]));
        assert_eq!(partial.unsupported_required, strings(&["restriction"]));
        assert_eq!(partial.factual.required_recall, Some(0.5));
        let cached = l.injection_policy(&strings(&["window", "coffee"]), &supported, true);
        assert!(!cached.integral_ok);
        assert!(cached.missed_supported.is_empty());
        assert_eq!(cached.unsupported_required, strings(&["restriction"]));
        assert_eq!(cached.unrelated_injected, strings(&["coffee"]));
        let unsupported_detail =
            l.injection_policy(&strings(&["window", "restriction"]), &supported, true);
        assert!(!unsupported_detail.integral_ok);
        assert_eq!(
            unsupported_detail.unsupported_injected,
            strings(&["restriction"])
        );
    }

    #[test]
    fn fallback_policy_preserves_supported_and_cached_required_details() {
        let l = labels();
        let supported = strings(&["window", "window-paraphrase"])
            .into_iter()
            .collect();
        assert!(!l.injection_policy(&[], &supported, false).integral_ok);
        assert!(
            l.injection_policy(&strings(&["window"]), &supported, false)
                .integral_ok
        );
        let cached = l.injection_policy(&[], &HashSet::new(), true);
        assert!(cached.integral_ok && cached.abstention_without_support);
        assert!(cached.missed_supported.is_empty());
        assert_eq!(cached.factual.required_recall, Some(0.0));
        assert!(!l.injection_policy(&[], &supported, true).integral_ok);
        assert!(
            l.injection_policy(&strings(&["window"]), &supported, true)
                .integral_ok
        );
        let duplicate_support = strings(&["window", "window-paraphrase"])
            .into_iter()
            .collect();
        let duplicate = l.injection_policy(
            &strings(&["window", "window-paraphrase"]),
            &duplicate_support,
            false,
        );
        assert!(duplicate.relevance_ok && !duplicate.integral_ok);
    }

    #[test]
    fn admissible_extra_is_not_noise() {
        let s = labels().score(&strings(&["window", "holiday"]));
        assert!(s.ok);
        assert_eq!(s.precision, Some(1.0));
    }
    #[test]
    fn admissible_extra_cannot_replace_missing_required() {
        let s = labels().score(&strings(&["holiday"]));
        assert!(!s.ok);
        assert_eq!(s.missing_required, strings(&["window"]));
    }
    #[test]
    fn unrelated_extra_is_noise() {
        let s = labels().score(&strings(&["window", "coffee"]));
        assert!(!s.ok);
        assert_eq!(s.noise, strings(&["coffee"]));
    }
    #[test]
    fn equivalent_duplicate_is_not_complementary_detail() {
        let l = labels();
        assert!(l.score(&strings(&["window-paraphrase", "holiday"])).ok);
        assert!(!l
            .score(&strings(&["window", "window-paraphrase"]))
            .duplicate_equivalents
            .is_empty());
        let l = Labels {
            required: strings(&["window", "holiday"]),
            admissible: vec![],
            ..labels()
        };
        assert!(l.score(&strings(&["window", "holiday"])).ok);
        assert!(!l.score(&strings(&["window"])).ok);
    }
    #[test]
    fn release_days_fixture_requires_both_distinct_details() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("cases.json")).unwrap();
        let c = cases
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "holdout-duplicate-en")
            .unwrap();
        let l: Labels = serde_json::from_value(c.clone()).unwrap();
        assert!(
            l.score(&strings(&[
                "en-work-deploy-window",
                "en-work-no-friday-deploys"
            ]))
            .ok
        );
        assert!(!l.score(&strings(&["en-work-deploy-window"])).ok);
        assert!(
            !l.score(&strings(&[
                "en-work-deploy-window",
                "dup-deploy-window",
                "en-work-no-friday-deploys"
            ]))
            .ok
        );
    }
    #[test]
    #[should_panic(expected = "conflicting/repeated label")]
    fn invalid_labels_do_not_silently_pass() {
        let l = Labels {
            required: strings(&["window"]),
            forbidden: strings(&["window"]),
            ..Labels::default()
        };
        l.validate(&strings(&["window"]).into_iter().collect());
    }
}
