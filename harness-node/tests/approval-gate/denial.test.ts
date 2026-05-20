import { describe, expect, it } from 'vitest';
import { permissionsDenyEnvelope, userDenyEnvelope } from '../../src/approval-gate/denial.js';
import { redact } from '../../src/approval-gate/redact.js';

describe('permissionsDenyEnvelope', () => {
  it('carries rule_id, rule_action and matched_constraint', () => {
    const env = permissionsDenyEnvelope(
      'shell::exec',
      'git/no-force-push',
      { field: 'command', operator: 'matches', value: '^git push --force' },
      { command: 'git push --force origin main' },
    );
    expect(env.status).toBe('denied');
    expect(env.denied_by).toBe('permissions');
    expect(env.rule_id).toBe('git/no-force-push');
    expect(env.rule_action).toBe('deny');
    expect(env.matched_constraint?.field).toBe('command');
    expect(env.reason).toContain('git/no-force-push');
  });
});

describe('userDenyEnvelope', () => {
  it('uses default reason when none provided', () => {
    const env = userDenyEnvelope('shell::exec', null, {});
    expect(env.denied_by).toBe('user');
    expect(env.reason).toBe('Rejected by operator.');
    expect(env.rule_id).toBeUndefined();
  });
});

describe('redact (denial args_excerpt)', () => {
  it('redacts secret-shaped keys (case-insensitive)', () => {
    const r = redact({
      url: 'https://example.com',
      password: 'hunter2',
      TOKEN: 'abc',
      headers: { Authorization: 'Bearer xyz' },
    }) as Record<string, unknown>;
    expect(r.url).toBe('https://example.com');
    expect(r.password).toBe('<redacted>');
    expect(r.TOKEN).toBe('<redacted>');
    expect((r.headers as Record<string, unknown>).Authorization).toBe('<redacted>');
  });

  it('clips long string values', () => {
    const long = 'x'.repeat(500);
    const r = redact({ blob: long }) as Record<string, unknown>;
    expect((r.blob as string).endsWith('…')).toBe(true);
  });
});
