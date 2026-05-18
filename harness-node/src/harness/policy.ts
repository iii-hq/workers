/**
 * Permissions file (`iii-permissions.yaml`) loader, watcher, and evaluator.
 * Direct port of `harness/src/policy.rs`. First-match-wins; bare strings
 * are allow-only shorthand.
 */

import { readFile } from 'node:fs/promises';
import { dirname, resolve as resolvePath } from 'node:path';
import chokidar, { type FSWatcher } from 'chokidar';
import { parse as parseYaml } from 'yaml';
import { logger } from '../runtime/otel.js';

export type Action = 'allow' | 'deny';

export type ConstraintSpec = { equals: unknown } | { matches: string };

export type RuleSpec =
  | string
  | {
      rule_id: string;
      function: string;
      action: Action;
      args?: Record<string, ConstraintSpec>;
    };

export type PermissionsFile = {
  version?: number;
  rules: RuleSpec[];
};

type CompiledConstraint =
  | { kind: 'equals'; value: unknown }
  | { kind: 'matches'; regex: RegExp; pattern: string };

type CompiledRule = {
  rule_id: string;
  function_id: string;
  action: Action;
  /** Sorted by field name so reported `matched_constraint` is deterministic. */
  constraints: Array<readonly [string, CompiledConstraint]>;
};

export type MatchedConstraint = {
  field: string;
  /** Lowercase operator name (`"equals"` or `"matches"`). */
  operator: string;
  value: unknown;
};

export type Decision =
  | { kind: 'allow'; rule_id: string }
  | { kind: 'deny'; rule_id: string; matched_constraint: MatchedConstraint | null }
  | { kind: 'needs_approval' };

export class Permissions {
  private constructor(private readonly rules: CompiledRule[]) {}

  static empty(): Permissions {
    return new Permissions([]);
  }

  ruleCount(): number {
    return this.rules.length;
  }

  static parse(text: string): Permissions {
    const file = parseYaml(text) as PermissionsFile | null;
    if (!file || typeof file !== 'object') return new Permissions([]);
    const rawRules = Array.isArray(file.rules) ? file.rules : [];
    const compiled: CompiledRule[] = [];
    const seen = new Set<string>();
    for (let idx = 0; idx < rawRules.length; idx++) {
      const spec = rawRules[idx];
      let rule: CompiledRule;
      if (typeof spec === 'string') {
        rule = {
          rule_id: spec,
          function_id: spec,
          action: 'allow',
          constraints: [],
        };
      } else if (spec && typeof spec === 'object') {
        const obj = spec as Record<string, unknown>;
        if (typeof obj.rule_id !== 'string' || obj.rule_id.length === 0) {
          throw new Error(
            `rule ${idx} (${JSON.stringify(obj.function)}): rule_id is required on the detailed form`,
          );
        }
        if (typeof obj.function !== 'string') {
          throw new Error(`rule ${idx}: function is required`);
        }
        if (obj.action !== 'allow' && obj.action !== 'deny') {
          throw new Error(`rule ${idx}: action must be 'allow' or 'deny'`);
        }
        const constraints: Array<readonly [string, CompiledConstraint]> = [];
        const args = (obj.args ?? {}) as Record<string, ConstraintSpec>;
        for (const [field, c] of Object.entries(args)) {
          if (c && typeof c === 'object' && 'equals' in c) {
            constraints.push([field, { kind: 'equals', value: (c as { equals: unknown }).equals }]);
          } else if (c && typeof c === 'object' && 'matches' in c) {
            const pattern = (c as { matches: string }).matches;
            try {
              const regex = new RegExp(pattern);
              constraints.push([field, { kind: 'matches', regex, pattern }] as const);
            } catch (err) {
              throw new Error(
                `rule ${obj.rule_id}: invalid regex ${JSON.stringify(pattern)} on field ${field}: ${String(err)}`,
              );
            }
          } else {
            throw new Error(`rule ${obj.rule_id}: unknown constraint shape on field ${field}`);
          }
        }
        constraints.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
        rule = {
          rule_id: obj.rule_id,
          function_id: obj.function,
          action: obj.action,
          constraints,
        };
      } else {
        throw new Error(`rule ${idx}: unrecognised shape`);
      }
      if (seen.has(rule.rule_id)) {
        logger.warn('duplicate rule_id in iii-permissions.yaml', {
          rule_id: rule.rule_id,
        });
      }
      seen.add(rule.rule_id);
      compiled.push(rule);
    }
    return new Permissions(compiled);
  }

  static async loadFromPath(path: string): Promise<Permissions> {
    const text = await readFile(path, 'utf8');
    return Permissions.parse(text);
  }

  check(function_id: string, args: unknown): Decision {
    for (const rule of this.rules) {
      if (rule.function_id !== function_id) continue;
      const matched = matchConstraints(args, rule.constraints);
      if (matched.kind === 'mismatch') continue;
      if (rule.action === 'allow') {
        return { kind: 'allow', rule_id: rule.rule_id };
      }
      return {
        kind: 'deny',
        rule_id: rule.rule_id,
        matched_constraint: matched.kind === 'with' ? matched.matched : null,
      };
    }
    return { kind: 'needs_approval' };
  }
}

type MatchOutcome =
  | { kind: 'mismatch' }
  | { kind: 'no_constraint' }
  | { kind: 'with'; matched: MatchedConstraint };

function matchConstraints(
  args: unknown,
  constraints: Array<readonly [string, CompiledConstraint]>,
): MatchOutcome {
  if (constraints.length === 0) return { kind: 'no_constraint' };
  const obj = args && typeof args === 'object' ? (args as Record<string, unknown>) : {};
  let firstMatched: MatchedConstraint | null = null;
  for (const [field, c] of constraints) {
    const actual = obj[field];
    if (c.kind === 'equals') {
      if (!deepEqual(actual, c.value)) return { kind: 'mismatch' };
      if (!firstMatched) firstMatched = { field, operator: 'equals', value: c.value };
    } else {
      if (typeof actual !== 'string') return { kind: 'mismatch' };
      if (!c.regex.test(actual)) return { kind: 'mismatch' };
      if (!firstMatched) firstMatched = { field, operator: 'matches', value: c.pattern };
    }
  }
  return firstMatched ? { kind: 'with', matched: firstMatched } : { kind: 'no_constraint' };
}

function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== typeof b) return false;
  if (a === null || b === null) return false;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!deepEqual(a[i], b[i])) return false;
    return true;
  }
  if (typeof a === 'object' && typeof b === 'object') {
    const ao = a as Record<string, unknown>;
    const bo = b as Record<string, unknown>;
    const ka = Object.keys(ao);
    const kb = Object.keys(bo);
    if (ka.length !== kb.length) return false;
    for (const k of ka) if (!deepEqual(ao[k], bo[k])) return false;
    return true;
  }
  return false;
}

/**
 * Live handle around an `Permissions` value that's swapped atomically
 * by a chokidar watcher. Exposes `current()` for the registered
 * function to read.
 */
export class PermissionsHandle {
  private current_: Permissions = Permissions.empty();
  private watcher_: FSWatcher | null = null;
  private readonly path_: string;

  constructor(path: string) {
    this.path_ = resolvePath(path);
  }

  current(): Permissions {
    return this.current_;
  }

  async load(): Promise<void> {
    try {
      this.current_ = await Permissions.loadFromPath(this.path_);
      logger.info('loaded iii-permissions.yaml', {
        path: this.path_,
        rules: this.current_.ruleCount(),
      });
    } catch (err) {
      logger.warn(
        'iii-permissions.yaml not loaded; starting with empty permissions (every call needs approval)',
        { path: this.path_, err: String(err) },
      );
      this.current_ = Permissions.empty();
    }
  }

  startWatching(): void {
    if (this.watcher_) return;
    // Watch the parent directory non-recursively so editor save+rename
    // sequences (delete+recreate) are still seen.
    const target = dirname(this.path_);
    let scheduled: NodeJS.Timeout | null = null;
    const reload = async () => {
      try {
        const next = await Permissions.loadFromPath(this.path_);
        this.current_ = next;
        logger.info('reloaded iii-permissions.yaml', {
          path: this.path_,
          rules: next.ruleCount(),
        });
      } catch (err) {
        logger.warn('failed to reload iii-permissions.yaml; keeping previous version', {
          path: this.path_,
          err: String(err),
        });
      }
    };
    const debounceReload = () => {
      if (scheduled) clearTimeout(scheduled);
      scheduled = setTimeout(reload, 150);
    };
    try {
      this.watcher_ = chokidar.watch(target, {
        depth: 0,
        ignoreInitial: true,
        awaitWriteFinish: { stabilityThreshold: 100, pollInterval: 50 },
      });
      this.watcher_.on('change', (p) => {
        if (resolvePath(p) === this.path_) debounceReload();
      });
      this.watcher_.on('add', (p) => {
        if (resolvePath(p) === this.path_) debounceReload();
      });
    } catch (err) {
      logger.warn('permissions watcher failed to start; reloads will require restart', {
        path: target,
        err: String(err),
      });
    }
  }

  async stop(): Promise<void> {
    if (this.watcher_) {
      await this.watcher_.close().catch(() => {});
      this.watcher_ = null;
    }
  }
}

export async function loadAndWatch(path: string): Promise<PermissionsHandle> {
  const handle = new PermissionsHandle(path);
  await handle.load();
  handle.startWatching();
  return handle;
}

/** Convert a Decision to the wire shape returned by `policy::check_permissions`. */
export function decisionToValue(decision: Decision): Record<string, unknown> {
  if (decision.kind === 'allow') {
    return { decision: 'allow', rule_id: decision.rule_id };
  }
  if (decision.kind === 'deny') {
    const out: Record<string, unknown> = {
      decision: 'deny',
      rule_id: decision.rule_id,
      rule_action: 'deny',
    };
    if (decision.matched_constraint) out.matched_constraint = decision.matched_constraint;
    return out;
  }
  return { decision: 'needs_approval' };
}
