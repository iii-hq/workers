use anyhow::{bail, Result};

use crate::report::HardGateReport;

use super::{CriterionAward, CriterionSpec, ObjectiveEvaluation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatePolicy {
    HardGated,
    ScoreOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AssessmentSpec {
    id: &'static str,
    weight: u8,
    description: &'static str,
    gate_policy: GatePolicy,
}

impl AssessmentSpec {
    pub(super) const fn hard_gated(
        id: &'static str,
        weight: u8,
        description: &'static str,
    ) -> Self {
        Self {
            id,
            weight,
            description,
            gate_policy: GatePolicy::HardGated,
        }
    }

    pub(super) const fn score_only(
        id: &'static str,
        weight: u8,
        description: &'static str,
    ) -> Self {
        Self {
            id,
            weight,
            description,
            gate_policy: GatePolicy::ScoreOnly,
        }
    }

    pub(super) const fn weight(self) -> u8 {
        self.weight
    }

    pub(super) fn full_or_zero(
        self,
        satisfied: bool,
        details: impl Into<String>,
    ) -> AssessmentOutcome {
        AssessmentOutcome {
            spec: self,
            awarded: if satisfied { self.weight } else { 0 },
            gate_passed: matches!(self.gate_policy, GatePolicy::HardGated).then_some(satisfied),
            details: details.into(),
        }
    }

    /// Records a check that could not run behind an explicit prerequisite gate.
    /// Prefer [`prerequisite_failure`] when constructing the complete evaluation.
    fn skipped_due_to_prerequisite(self, details: impl Into<String>) -> AssessmentOutcome {
        AssessmentOutcome {
            spec: self,
            awarded: 0,
            gate_passed: None,
            details: details.into(),
        }
    }

    pub(super) fn award(
        self,
        awarded: u8,
        details: impl Into<String>,
    ) -> Result<AssessmentOutcome> {
        if self.gate_policy == GatePolicy::HardGated {
            bail!(
                "assessment '{}': award(awarded={awarded}) cannot emit a hard gate for a hard-gated assessment; use full_or_zero(...) or gate_and_points(...)",
                self.id,
            );
        }
        self.validate_award("award", awarded)?;
        Ok(AssessmentOutcome {
            spec: self,
            awarded,
            gate_passed: None,
            details: details.into(),
        })
    }

    pub(super) fn gate_and_points(
        self,
        gate_passed: bool,
        awarded: u8,
        details: impl Into<String>,
    ) -> Result<AssessmentOutcome> {
        if self.gate_policy == GatePolicy::ScoreOnly {
            bail!(
                "assessment '{}': gate_and_points(gate_passed={gate_passed}, awarded={awarded}) cannot emit a hard gate for a score-only assessment; use full_or_zero(...) or award(...)",
                self.id,
            );
        }
        self.validate_award("gate_and_points", awarded)?;
        Ok(AssessmentOutcome {
            spec: self,
            awarded,
            gate_passed: Some(gate_passed),
            details: details.into(),
        })
    }

    fn validate_award(self, operation: &str, awarded: u8) -> Result<()> {
        if awarded > self.weight {
            bail!(
                "assessment '{}': {operation}(awarded={awarded}) exceeds max_points={}; expected awarded in 0..={}",
                self.id,
                self.weight,
                self.weight
            );
        }
        Ok(())
    }

    fn criterion(self) -> CriterionSpec {
        CriterionSpec {
            id: self.id,
            weight: self.weight,
            description: self.description,
        }
    }
}

#[derive(Debug)]
pub(super) struct AssessmentOutcome {
    spec: AssessmentSpec,
    awarded: u8,
    gate_passed: Option<bool>,
    details: String,
}

pub(super) fn criteria(specs: &[AssessmentSpec]) -> Vec<CriterionSpec> {
    specs
        .iter()
        .copied()
        .map(AssessmentSpec::criterion)
        .collect()
}

pub(super) fn build_evaluation(
    results: impl IntoIterator<Item = AssessmentOutcome>,
) -> ObjectiveEvaluation {
    let mut hard_gates = Vec::new();
    let mut awards = Vec::new();

    for result in results {
        if let Some(passed) = result.gate_passed {
            hard_gates.push(HardGateReport {
                id: result.spec.id.to_string(),
                passed,
                reason: result.details.clone(),
            });
        }
        awards.push(CriterionAward {
            id: result.spec.id.to_string(),
            awarded: result.awarded,
            reason: result.details,
        });
    }

    ObjectiveEvaluation { hard_gates, awards }
}

pub(super) fn prerequisite_failure(
    specs: &[AssessmentSpec],
    gate_id: impl Into<String>,
    details: impl Into<String>,
) -> ObjectiveEvaluation {
    let gate_id = gate_id.into();
    let reason = format!("prerequisite gate '{gate_id}' failed: {}", details.into());
    let mut evaluation = build_evaluation(
        specs
            .iter()
            .copied()
            .map(|spec| spec.skipped_due_to_prerequisite(reason.clone())),
    );
    evaluation.hard_gates.push(HardGateReport {
        id: gate_id,
        passed: false,
        reason,
    });
    evaluation
}

#[cfg(test)]
mod tests {
    use super::*;

    const HARD_GATED: AssessmentSpec =
        AssessmentSpec::hard_gated("required", 70, "A required outcome.");
    const SCORE_ONLY: AssessmentSpec =
        AssessmentSpec::score_only("signal", 30, "A quality signal.");

    #[test]
    fn criteria_preserve_assessment_definitions() {
        let criteria = criteria(&[HARD_GATED, SCORE_ONLY]);

        assert_eq!(criteria.len(), 2);
        assert_eq!(criteria[0].id, "required");
        assert_eq!(criteria[0].weight, 70);
        assert_eq!(criteria[0].description, "A required outcome.");
        assert_eq!(criteria[1].id, "signal");
        assert_eq!(criteria[1].weight, 30);
        assert_eq!(criteria[1].description, "A quality signal.");
        assert_eq!(
            criteria
                .iter()
                .map(|criterion| u16::from(criterion.weight))
                .sum::<u16>(),
            100
        );
    }

    #[test]
    fn hard_gated_pass_produces_a_passing_gate_and_full_award() {
        let evaluation = build_evaluation([HARD_GATED.full_or_zero(true, "satisfied")]);

        assert_eq!(evaluation.hard_gates.len(), 1);
        assert_eq!(evaluation.hard_gates[0].id, "required");
        assert!(evaluation.hard_gates[0].passed);
        assert_eq!(evaluation.hard_gates[0].reason, "satisfied");
        assert_eq!(evaluation.awards.len(), 1);
        assert_eq!(evaluation.awards[0].id, "required");
        assert_eq!(evaluation.awards[0].awarded, 70);
        assert_eq!(evaluation.awards[0].reason, "satisfied");
    }

    #[test]
    fn hard_gated_failure_produces_a_failed_gate_and_zero_award() {
        let evaluation = build_evaluation([HARD_GATED.full_or_zero(false, "missing")]);

        assert!(!evaluation.hard_gates[0].passed);
        assert_eq!(evaluation.awards[0].awarded, 0);
    }

    #[test]
    fn score_only_produces_an_award_without_a_gate() {
        let passed = build_evaluation([SCORE_ONLY.full_or_zero(true, "observed")]);
        let failed = build_evaluation([SCORE_ONLY.full_or_zero(false, "missing")]);

        assert!(passed.hard_gates.is_empty());
        assert_eq!(passed.awards[0].id, "signal");
        assert_eq!(passed.awards[0].awarded, 30);
        assert!(failed.hard_gates.is_empty());
        assert_eq!(failed.awards[0].awarded, 0);
    }

    #[test]
    fn score_only_accepts_partial_awards_within_its_weight() {
        let evaluation = build_evaluation([SCORE_ONLY.award(12, "partial").unwrap()]);

        assert!(evaluation.hard_gates.is_empty());
        assert_eq!(evaluation.awards[0].awarded, 12);
    }

    #[test]
    fn score_only_rejects_awards_above_its_weight() {
        assert_eq!(
            SCORE_ONLY.award(31, "too many").unwrap_err().to_string(),
            "assessment 'signal': award(awarded=31) exceeds max_points=30; expected awarded in 0..=30"
        );
    }

    #[test]
    fn hard_gated_assessments_reject_the_score_only_award_api() {
        assert_eq!(
            HARD_GATED.award(35, "partial").unwrap_err().to_string(),
            "assessment 'required': award(awarded=35) cannot emit a hard gate for a hard-gated assessment; use full_or_zero(...) or gate_and_points(...)"
        );
    }

    #[test]
    fn hard_gated_assessment_can_award_partial_points_after_passing() {
        let evaluation = build_evaluation([HARD_GATED
            .gate_and_points(true, 35, "passed with partial quality")
            .unwrap()]);

        assert!(evaluation.hard_gates[0].passed);
        assert_eq!(evaluation.awards[0].awarded, 35);
    }

    #[test]
    fn failed_hard_gate_can_retain_independent_quality_points() {
        let evaluation = build_evaluation([HARD_GATED
            .gate_and_points(false, 35, "failed with partial quality")
            .unwrap()]);

        assert!(!evaluation.hard_gates[0].passed);
        assert_eq!(evaluation.awards[0].awarded, 35);
    }

    #[test]
    fn hard_gated_assessment_rejects_awards_above_its_weight() {
        assert_eq!(
            HARD_GATED
                .gate_and_points(true, 71, "too many")
                .unwrap_err()
                .to_string(),
            "assessment 'required': gate_and_points(awarded=71) exceeds max_points=70; expected awarded in 0..=70"
        );
    }

    #[test]
    fn score_only_rejects_the_hard_gate_api() {
        assert_eq!(
            SCORE_ONLY
                .gate_and_points(true, 30, "wrong kind")
                .unwrap_err()
                .to_string(),
            "assessment 'signal': gate_and_points(gate_passed=true, awarded=30) cannot emit a hard gate for a score-only assessment; use full_or_zero(...) or award(...)"
        );
    }

    #[test]
    fn prerequisite_skips_preserve_order_without_duplicate_gates() {
        let evaluation = prerequisite_failure(
            &[HARD_GATED, SCORE_ONLY],
            "database_available",
            "database capability is unavailable",
        );

        assert_eq!(evaluation.hard_gates.len(), 1);
        assert!(!evaluation.hard_gates[0].passed);
        assert_eq!(evaluation.hard_gates[0].id, "database_available");
        assert_eq!(evaluation.awards.len(), 2);
        assert_eq!(evaluation.awards[0].id, "required");
        assert_eq!(evaluation.awards[0].awarded, 0);
        assert_eq!(
            evaluation.awards[0].reason,
            "prerequisite gate 'database_available' failed: database capability is unavailable"
        );
        assert_eq!(evaluation.awards[1].id, "signal");
        assert_eq!(evaluation.awards[1].awarded, 0);
        assert_eq!(
            evaluation.awards[1].reason,
            "prerequisite gate 'database_available' failed: database capability is unavailable"
        );
    }
}
