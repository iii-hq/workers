import { describe, expect, it } from 'vitest';
import { observeConfigChanges } from '../src/configuration.js';
import { fakeIii } from './_helpers/fake-iii.js';

describe('observeConfigChanges on iii-sdk 0.26', () => {
  it('captures the output with `capture`, not the removed `deliver` field', async () => {
    const fake = fakeIii();
    await observeConfigChanges(fake.iii, async () => {});
    const source = fake.observed[0]?.source as Record<string, unknown>;
    expect(source).not.toHaveProperty('deliver');
    expect(source.capture).toEqual(['output']);
    expect(source.filters).toEqual({
      // The module reads III_CONFIG_NAME at import; a worker shell may set it.
      'input.id': { $eq: process.env.III_CONFIG_NAME?.trim() || 'claude-code' },
      'input.event_type': { $eq: 'configuration:updated' },
    });
  });
});
