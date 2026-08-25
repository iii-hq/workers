import { describe, expect, it } from 'vitest';
import { readStatus } from '../../src/terminal/auth.js';

describe('who pays for a session', () => {
  it('reads a subscription login', () => {
    const status = readStatus(
      JSON.stringify({
        loggedIn: true,
        authMethod: 'claude.ai',
        apiProvider: 'firstParty',
        email: 'dev@example.com',
        orgName: 'iii',
        subscriptionType: 'team',
      }),
    );
    expect(status.billing).toBe('subscription');
    expect(status.label).toContain('team subscription');
    expect(status.label).toContain('dev@example.com');
    expect(status.api_key_source).toBe('');
  });

  it('reports the API key when one is set, because the key outranks the login', () => {
    // The CLI keeps saying authMethod: claude.ai here — it is still signed in.
    // Billing is not: "ANTHROPIC_API_KEY ... takes precedence over your
    // claude.ai login", and a bogus key 401s the turn.
    const status = readStatus(
      JSON.stringify({
        loggedIn: true,
        authMethod: 'claude.ai',
        apiKeySource: 'ANTHROPIC_API_KEY',
        email: null,
        orgName: null,
        subscriptionType: null,
      }),
    );
    expect(status.billing).toBe('api-key');
    expect(status.label).toContain('API key billing');
    expect(status.label).toContain('ANTHROPIC_API_KEY');
    expect(status.logged_in).toBe(true);
  });

  it('says so when the login is signed in but a key still wins', () => {
    const status = readStatus(
      JSON.stringify({
        loggedIn: true,
        authMethod: 'claude.ai',
        apiKeySource: 'ANTHROPIC_API_KEY',
        email: 'dev@example.com',
        subscriptionType: 'max',
      }),
    );
    expect(status.detail).toContain('NOT billed');
    expect(status.detail).toContain('dev@example.com');
  });

  it('reads a host with no credentials at all', () => {
    const status = readStatus(JSON.stringify({ loggedIn: false }));
    expect(status.billing).toBe('none');
    expect(status.label).toBe('not signed in');
    expect(status.detail).toContain('/login');
  });

  it('does not guess when the answer is not JSON', () => {
    expect(readStatus('command not found: claude').billing).toBe('unknown');
    expect(readStatus('').billing).toBe('unknown');
  });
});
