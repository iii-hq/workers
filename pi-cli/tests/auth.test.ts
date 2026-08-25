import { describe, expect, it } from 'vitest';
import { readStatus } from '../src/auth.js';

describe('who pays for a session', () => {
  it('reads an OAuth credential as a subscription login', () => {
    const status = readStatus(
      JSON.stringify({
        status: 'ready',
        provider: 'anthropic',
        credential: { type: 'oauth', source: 'keychain' },
      }),
      'anthropic',
    );
    expect(status.billing).toBe('subscription');
    expect(status.label).toBe('anthropic subscription');
    expect(status.ready).toBe(true);
  });

  it('reads an api key credential as metered', () => {
    const status = readStatus(
      JSON.stringify({ status: 'ready', provider: 'anthropic', credential: { type: 'api_key' } }),
      'anthropic',
    );
    expect(status.billing).toBe('api-key');
    expect(status.label).toContain('API key billing');
  });

  it('says ready without claiming a kind pi did not report', () => {
    const status = readStatus(JSON.stringify({ status: 'ready', provider: 'openai' }), 'openai');
    expect(status.billing).toBe('unknown');
    expect(status.label).toBe('openai ready');
    expect(status.detail).toContain('kind was not reported');
  });

  it('reads a provider with no credentials, and repeats pi’s reason', () => {
    const status = readStatus(
      JSON.stringify({
        status: 'not_ready',
        provider: 'anthropic',
        reason: 'credentials_not_configured',
      }),
      'anthropic',
    );
    expect(status.billing).toBe('none');
    expect(status.label).toBe('anthropic: not signed in');
    expect(status.detail).toContain('/login');
    expect(status.reason).toBe('credentials_not_configured');
  });

  it('does not guess when the answer is not JSON', () => {
    expect(readStatus('command not found: pi', 'anthropic').billing).toBe('unknown');
  });
});
