import { readFile } from 'node:fs/promises';
import { parse as parseYaml } from 'yaml';
import { logger } from '../../runtime/otel.js';
import { compileRule, matchConstraints, type CompiledRule } from './compile.js';
import type { Decision, RuleSpec } from './types.js';

export class Permissions {
  private constructor(private readonly rules: CompiledRule[]) {}

  static empty(): Permissions {
    return new Permissions([]);
  }

  ruleCount(): number {
    return this.rules.length;
  }

  static parse(text: string): Permissions {
    const file = parseYaml(text) as { rules?: RuleSpec[] } | null;
    const rawRules = Array.isArray(file?.rules) ? file.rules : [];
    const compiled: CompiledRule[] = [];
    const seen = new Set<string>();

    for (let idx = 0; idx < rawRules.length; idx++) {
      const spec = rawRules[idx];
      if (spec === undefined) continue;
      const rule = compileRule(spec, idx);
      if (seen.has(rule.rule_id)) {
        logger.warn('duplicate rule_id in iii-permissions.yaml', { rule_id: rule.rule_id });
      }
      seen.add(rule.rule_id);
      compiled.push(rule);
    }
    return new Permissions(compiled);
  }

  static async loadFromPath(path: string): Promise<Permissions> {
    return Permissions.parse(await readFile(path, 'utf8'));
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
