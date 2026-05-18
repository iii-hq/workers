import { describe, expect, it } from 'vitest';
import { parsePolicyReply } from '../../src/approval-gate/policy-consult.js';

describe('parsePolicyReply', () => {
  it('decodes allow with rule_id', () => {
    const o = parsePolicyReply({ decision: 'allow', rule_id: 'git/status' });
    expect(o).toEqual({ kind: 'allow', rule_id: 'git/status' });
  });

  it('decodes deny with constraint', () => {
    const o = parsePolicyReply({
      decision: 'deny',
      rule_id: 'git/no-force-push',
      matched_constraint: {
        field: 'command',
        operator: 'matches',
        value: '^git push --force',
      },
    });
    expect(o.kind).toBe('deny');
    if (o.kind === 'deny') {
      expect(o.rule_id).toBe('git/no-force-push');
      expect(o.matched_constraint?.field).toBe('command');
    }
  });

  it('falls through to needs_approval for unknown / missing decision', () => {
    expect(parsePolicyReply({})).toEqual({ kind: 'needs_approval' });
    expect(parsePolicyReply({ decision: 'weird' })).toEqual({ kind: 'needs_approval' });
  });
});
