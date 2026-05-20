import { describe, expect, it } from 'vitest';
import { ENV_VAR_MAP, envVarFor } from '../../src/auth-credentials/types.js';

describe('env map: kimi', () => {
  it('resolves `kimi` to MOONSHOT_API_KEY', () => {
    expect(envVarFor('kimi')).toBe('MOONSHOT_API_KEY');
  });

  it('keeps the legacy `kimi-coding` slug pointing at the same env var', () => {
    expect(envVarFor('kimi-coding')).toBe('MOONSHOT_API_KEY');
  });

  it('exposes both slugs through ENV_VAR_MAP', () => {
    const slugs = ENV_VAR_MAP.map(([provider]) => provider);
    expect(slugs).toContain('kimi');
    expect(slugs).toContain('kimi-coding');
  });
});
