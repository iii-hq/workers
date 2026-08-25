import { describe, expect, it } from 'vitest';
import { readProvider, summarize } from '../src/auth.js';

describe('reading one provider', () => {
  it('reads an OAuth credential as a subscription login', () => {
    const status = readProvider(
      JSON.stringify({
        status: 'ready',
        provider: 'anthropic',
        credential: { type: 'oauth', source: 'keychain' },
      }),
      'anthropic',
    );
    expect(status).toEqual({
      provider: 'anthropic',
      ready: true,
      kind: 'subscription',
      reason: '',
    });
  });

  it('reads an api key credential as metered, in either shape pi prints', () => {
    const nested = readProvider(
      JSON.stringify({ status: 'ready', provider: 'anthropic', credential: { type: 'api_key' } }),
      'anthropic',
    );
    expect(nested.kind).toBe('api-key');
    // What pi actually prints today: a flat `authType`.
    const flat = readProvider(
      JSON.stringify({ status: 'ready', provider: 'openai', authType: 'api_key' }),
      'openai',
    );
    expect(flat).toEqual({ provider: 'openai', ready: true, kind: 'api-key', reason: '' });
  });

  it('says ready without claiming a kind pi did not report', () => {
    const status = readProvider(JSON.stringify({ status: 'ready', provider: 'openai' }), 'openai');
    expect(status.ready).toBe(true);
    expect(status.kind).toBe('');
  });

  it('repeats pi’s reason when a provider is not ready', () => {
    const status = readProvider(
      JSON.stringify({
        status: 'not_ready',
        provider: 'anthropic',
        reason: 'credentials_not_configured',
      }),
      'anthropic',
    );
    expect(status.ready).toBe(false);
    expect(status.reason).toBe('credentials_not_configured');
  });

  it('does not guess when the answer is not JSON', () => {
    expect(readProvider('command not found: pi', 'anthropic').ready).toBe(false);
  });
});

describe('what the badge says', () => {
  it('names the one provider that is ready, and its kind', () => {
    const status = summarize([
      { provider: 'openai', ready: true, kind: 'api-key', reason: '' },
      { provider: 'anthropic', ready: false, kind: '', reason: 'credentials_not_configured' },
    ]);
    expect(status.billing).toBe('api-key');
    expect(status.label).toBe('openai (API key)');
    expect(status.detail).toContain('Not ready: anthropic');
    expect(status.ready).toBe(true);
  });

  it('counts them when several are signed in, and prefers a subscription', () => {
    const status = summarize([
      { provider: 'openai', ready: true, kind: 'api-key', reason: '' },
      { provider: 'anthropic', ready: true, kind: 'subscription', reason: '' },
    ]);
    // A plan behind the turn is the answer worth surfacing over a per-token bill.
    expect(status.billing).toBe('subscription');
    expect(status.label).toBe('2 providers · openai, anthropic');
    expect(status.providers).toHaveLength(2);
  });

  it('says so plainly when nothing is signed in', () => {
    const status = summarize([
      { provider: 'anthropic', ready: false, kind: '', reason: 'credentials_not_configured' },
    ]);
    expect(status.billing).toBe('none');
    expect(status.label).toBe('no provider signed in');
    expect(status.detail).toContain('checked anthropic');
    expect(status.detail).toContain('/login');
  });

  it('does not call an empty store a failure', () => {
    const status = summarize([]);
    expect(status.billing).toBe('none');
    expect(status.detail).toContain('no credentials');
  });
});
