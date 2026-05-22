import { describe, expect, it } from 'vitest';
import { ENV_VAR_MAP, envVarFor } from '../../src/auth-credentials/types.js';

describe('env map: llamacpp', () => {
  it('resolves `llamacpp` to LLAMACPP_API_KEY', () => {
    expect(envVarFor('llamacpp')).toBe('LLAMACPP_API_KEY');
  });

  it('exposes the slug through ENV_VAR_MAP', () => {
    const slugs = ENV_VAR_MAP.map(([provider]) => provider);
    expect(slugs).toContain('llamacpp');
  });
});
