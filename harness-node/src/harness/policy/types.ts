/** Types for `iii-permissions.yaml` rules and evaluation results. */

export type Action = 'allow' | 'deny';

export type ConstraintSpec = { equals: unknown } | { matches: string };

export type RuleSpec =
  | string
  | {
      rule_id?: string;
      function: string;
      action: Action;
      args?: Record<string, ConstraintSpec>;
    };

export type MatchedConstraint = {
  field: string;
  operator: string;
  value: unknown;
};

export type Decision =
  | { kind: 'allow'; rule_id: string }
  | { kind: 'deny'; rule_id: string; matched_constraint: MatchedConstraint | null }
  | { kind: 'needs_approval' };
