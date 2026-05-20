import { isDeepStrictEqual } from 'node:util';
import type { Action, ConstraintSpec, MatchedConstraint, RuleSpec } from './types.js';

export type CompiledConstraint =
  | { kind: 'equals'; value: unknown }
  | { kind: 'matches'; regex: RegExp; pattern: string };

export type CompiledRule = {
  rule_id: string;
  function_id: string;
  action: Action;
  constraints: Array<readonly [string, CompiledConstraint]>;
};

type MatchOutcome =
  | { kind: 'mismatch' }
  | { kind: 'no_constraint' }
  | { kind: 'with'; matched: MatchedConstraint };

function argsRecord(args: unknown): Record<string, unknown> {
  return args !== null && typeof args === 'object' && !Array.isArray(args)
    ? (args as Record<string, unknown>)
    : {};
}

export function compileRule(spec: RuleSpec, idx: number): CompiledRule {
  if (typeof spec === 'string') {
    if (spec.startsWith('!')) {
      const function_id = spec.slice(1);
      if (!function_id) throw new Error(`rule ${idx}: deny shorthand must be !<function_id>`);
      return { rule_id: function_id, function_id, action: 'deny', constraints: [] };
    }
    return { rule_id: spec, function_id: spec, action: 'allow', constraints: [] };
  }

  if (typeof spec !== 'object' || spec === null) {
    throw new Error(`rule ${idx}: unrecognised shape`);
  }

  const function_id = spec.function;
  if (!function_id) throw new Error(`rule ${idx}: function is required`);
  if (spec.action !== 'allow' && spec.action !== 'deny') {
    throw new Error(`rule ${idx}: action must be 'allow' or 'deny'`);
  }

  const rule_id = spec.rule_id?.length ? spec.rule_id : function_id;
  const constraints = Object.entries(spec.args ?? {}).map(
    ([field, c]) => [field, compileConstraint(rule_id, field, c)] as const,
  );
  constraints.sort(([a], [b]) => a.localeCompare(b));

  return { rule_id, function_id, action: spec.action, constraints };
}

function compileConstraint(rule_id: string, field: string, c: ConstraintSpec): CompiledConstraint {
  if ('equals' in c) return { kind: 'equals', value: c.equals };
  if ('matches' in c) {
    try {
      return { kind: 'matches', regex: new RegExp(c.matches), pattern: c.matches };
    } catch (err) {
      throw new Error(
        `rule ${rule_id}: invalid regex ${JSON.stringify(c.matches)} on field ${field}: ${String(err)}`,
      );
    }
  }
  throw new Error(`rule ${rule_id}: unknown constraint shape on field ${field}`);
}

function constraintMatches(actual: unknown, c: CompiledConstraint): boolean {
  if (c.kind === 'equals') return isDeepStrictEqual(actual, c.value);
  return typeof actual === 'string' && c.regex.test(actual);
}

function matchedConstraint(field: string, c: CompiledConstraint): MatchedConstraint {
  return c.kind === 'equals'
    ? { field, operator: 'equals', value: c.value }
    : { field, operator: 'matches', value: c.pattern };
}

export function matchConstraints(
  args: unknown,
  constraints: Array<readonly [string, CompiledConstraint]>,
): MatchOutcome {
  if (constraints.length === 0) return { kind: 'no_constraint' };

  const obj = argsRecord(args);
  let matched: MatchedConstraint | null = null;
  for (const [field, c] of constraints) {
    if (!constraintMatches(obj[field], c)) return { kind: 'mismatch' };
    if (!matched) matched = matchedConstraint(field, c);
  }
  return matched ? { kind: 'with', matched } : { kind: 'no_constraint' };
}
