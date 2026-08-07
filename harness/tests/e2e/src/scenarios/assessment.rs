use anyhow::{bail, Result};

use crate::report::HardGateReport;

use super::{CriterionAward, CriterionSpec, ObjectiveEvaluation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssessmentKind {
    Required,
    Signal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AssessmentSpec {
    id: &'static str,
    weight: u8,
    description: &'static str,
    kind: AssessmentKind,
}

impl AssessmentSpec {
    pub(super) const fn required(id: &'static str, weight: u8, description: &'static str) -> Self {
        Self {
            id,
            weight,
            description,
            kind: AssessmentKind::Required,
        }
    }

    pub(super) const fn signal(id: &'static str, weight: u8, description: &'static str) -> Self {
        Self {
            id,
            weight,
            description,
            kind: AssessmentKind::Signal,
        }
    }

    pub(super) const fn weight(self) -> u8 {
        self.weight
    }

    pub(super) fn binary(self, passed: bool, reason: impl Into<String>) -> AssessmentResult {
        AssessmentResult {
            spec: self,
            awarded: if passed { self.weight } else { 0 },
            gate_passed: matches!(self.kind, AssessmentKind::Required).then_some(passed),
            reason: reason.into(),
        }
    }

    pub(super) fn points(self, awarded: u8, reason: impl Into<String>) -> Result<AssessmentResult> {
        if self.kind == AssessmentKind::Required {
            bail!(
                "required assessment {} must be evaluated as pass or fail",
                self.id
            );
        }
        if awarded > self.weight {
            bail!(
                "assessment {} awarded {awarded} points, exceeding its weight {}",
                self.id,
                self.weight
            );
        }
        Ok(AssessmentResult {
            spec: self,
            awarded,
            gate_passed: None,
            reason: reason.into(),
        })
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
pub(super) struct AssessmentResult {
    spec: AssessmentSpec,
    awarded: u8,
    gate_passed: Option<bool>,
    reason: String,
}

pub(super) fn criteria(specs: &[AssessmentSpec]) -> Vec<CriterionSpec> {
    specs
        .iter()
        .copied()
        .map(AssessmentSpec::criterion)
        .collect()
}

pub(super) fn objective(
    results: impl IntoIterator<Item = AssessmentResult>,
) -> ObjectiveEvaluation {
    let mut hard_gates = Vec::new();
    let mut awards = Vec::new();

    for result in results {
        if let Some(passed) = result.gate_passed {
            hard_gates.push(HardGateReport {
                id: result.spec.id.to_string(),
                passed,
                reason: result.reason.clone(),
            });
        }
        awards.push(CriterionAward {
            id: result.spec.id.to_string(),
            awarded: result.awarded,
            reason: result.reason,
        });
    }

    ObjectiveEvaluation { hard_gates, awards }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED: AssessmentSpec =
        AssessmentSpec::required("required", 70, "A required outcome.");
    const SIGNAL: AssessmentSpec = AssessmentSpec::signal("signal", 30, "A quality signal.");

    #[test]
    fn criteria_preserve_assessment_definitions() {
        let criteria = criteria(&[REQUIRED, SIGNAL]);

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
    fn required_pass_produces_a_passing_gate_and_full_award() {
        let evaluation = objective([REQUIRED.binary(true, "satisfied")]);

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
    fn required_failure_produces_a_failed_gate_and_zero_award() {
        let evaluation = objective([REQUIRED.binary(false, "missing")]);

        assert!(!evaluation.hard_gates[0].passed);
        assert_eq!(evaluation.awards[0].awarded, 0);
    }

    #[test]
    fn signal_produces_an_award_without_a_gate() {
        let passed = objective([SIGNAL.binary(true, "observed")]);
        let failed = objective([SIGNAL.binary(false, "missing")]);

        assert!(passed.hard_gates.is_empty());
        assert_eq!(passed.awards[0].id, "signal");
        assert_eq!(passed.awards[0].awarded, 30);
        assert!(failed.hard_gates.is_empty());
        assert_eq!(failed.awards[0].awarded, 0);
    }

    #[test]
    fn signal_accepts_partial_points_within_its_weight() {
        let evaluation = objective([SIGNAL.points(12, "partial").unwrap()]);

        assert!(evaluation.hard_gates.is_empty());
        assert_eq!(evaluation.awards[0].awarded, 12);
    }

    #[test]
    fn signal_rejects_points_above_its_weight() {
        assert_eq!(
            SIGNAL.points(31, "too many").unwrap_err().to_string(),
            "assessment signal awarded 31 points, exceeding its weight 30"
        );
    }

    #[test]
    fn required_assessments_reject_partial_points() {
        assert_eq!(
            REQUIRED.points(35, "partial").unwrap_err().to_string(),
            "required assessment required must be evaluated as pass or fail"
        );
    }
}
