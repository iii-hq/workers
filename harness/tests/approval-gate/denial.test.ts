/**
 * Adversarial tests for denial-envelope assembly. The envelopes are handed
 * to the model and surfaced to the operator, so the things under attack are:
 *   - secrets in args must be redacted in BOTH wire envelopes;
 *   - oversized args must be clipped before they reach the envelope;
 *   - the user-facing envelope must not leak permissions-internal fields;
 *   - the deny reason must embed an arbitrary matched value without breaking.
 */
import { describe, expect, it } from 'vitest';
import {
  permissionsDenyEnvelope,
  reasonForPermissionsDeny,
  userDenyEnvelope,
} from '../../src/approval-gate/denial.js';

describe('args redaction reaches the envelope', () => {
  it('redacts secrets in args for the permissions envelope', () => {
    const env = permissionsDenyEnvelope('shell::exec', 'r1', null, {
      url: 'https://x',
      authorization: 'Bearer leak',
      nested: { api_key: 'leak2', keep: 'ok' },
    });
    expect(env.args_excerpt).toEqual({
      url: 'https://x',
      authorization: '<redacted>',
      nested: { api_key: '<redacted>', keep: 'ok' },
    });
  });

  it('redacts secrets in args for the user envelope', () => {
    const env = userDenyEnvelope('shell::exec', null, { password: 'hunter2', keep: 1 });
    expect(env.args_excerpt).toEqual({ password: '<redacted>', keep: 1 });
  });

  it('clips an oversized arg value inside the envelope', () => {
    const env = permissionsDenyEnvelope('shell::exec', 'r1', null, { blob: 'x'.repeat(500) });
    const excerpt = env.args_excerpt as { blob: string };
    expect([...excerpt.blob].length).toBe(257);
    expect(excerpt.blob.endsWith('…')).toBe(true);
  });
});

describe('permissionsDenyEnvelope', () => {
  it('carries rule_id, rule_action and matched_constraint', () => {
    const env = permissionsDenyEnvelope(
      'shell::exec',
      'git/no-force-push',
      { field: 'command', operator: 'matches', value: '^git push --force' },
      { command: 'git push --force origin main' },
    );
    expect(env.denied_by).toBe('permissions');
    expect(env.rule_action).toBe('deny');
    expect(env.matched_constraint?.field).toBe('command');
    expect(env.reason).toContain('git/no-force-push');
  });

  it('omits matched_constraint and uses the no-constraint phrasing when none matched', () => {
    const env = permissionsDenyEnvelope('shell::exec', 'blanket-deny', null, {});
    expect(env.matched_constraint).toBeUndefined();
    expect(env.reason).toContain('blocked by policy');
  });
});

describe('userDenyEnvelope — must not leak permissions internals', () => {
  it('falls back to the default reason and carries no policy fields', () => {
    const env = userDenyEnvelope('shell::exec', null, {});
    expect(env.denied_by).toBe('user');
    expect(env.reason).toBe('Rejected by operator.');
    expect(env.rule_id).toBeUndefined();
    expect(env.rule_action).toBeUndefined();
    expect(env.matched_constraint).toBeUndefined();
  });

  it('preserves an explicit operator reason verbatim', () => {
    const env = userDenyEnvelope('shell::exec', 'looks like data exfiltration', {});
    expect(env.reason).toBe('looks like data exfiltration');
  });
});

describe('reasonForPermissionsDeny — embeds an arbitrary matched value safely', () => {
  it('JSON-encodes a value containing quotes and shell metacharacters', () => {
    const reason = reasonForPermissionsDeny('shell::exec', 'r1', {
      field: 'command',
      operator: 'eq',
      value: '"; rm -rf / #',
    });
    expect(reason).toContain(JSON.stringify('"; rm -rf / #'));
  });

  it('encodes a structured (object) matched value without throwing', () => {
    expect(() =>
      reasonForPermissionsDeny('shell::exec', 'r1', {
        field: 'env',
        operator: 'contains',
        value: { nested: [1, 2, 3] },
      }),
    ).not.toThrow();
  });
});
