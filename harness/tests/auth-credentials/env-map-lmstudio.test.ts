import { describe, expect, it } from 'vitest';
import { ENV_VAR_MAP, envVarFor } from '../../src/auth-credentials/types.js';

describe('env map: lmstudio', () => {
  it('resolves `lmstudio` to LMSTUDIO_API_KEY', () => {
    expect(envVarFor('lmstudio')).toBe('LMSTUDIO_API_KEY');
  });

  it('exposes the slug through ENV_VAR_MAP', () => {
    const slugs = ENV_VAR_MAP.map(([provider]) => provider);
    expect(slugs).toContain('lmstudio');
  });
});
