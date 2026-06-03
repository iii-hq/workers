import { describe, expect, it } from 'vitest';
import {
  normalizeHarnessConfig,
  providerDiscoveryFingerprint,
  providersAffectedByConfigChange,
} from '../../../src/runtime/harness-config.js';

describe('providersAffectedByConfigChange', () => {
  it('returns [] when only permissions change', () => {
    const oldCfg = normalizeHarnessConfig({
      permissions: { default_mode: 'manual' },
      providers: { anthropic: { api_key: 'sk-a' } },
    });
    const newCfg = normalizeHarnessConfig({
      permissions: { default_mode: 'auto' },
      providers: { anthropic: { api_key: 'sk-a' } },
    });
    expect(providersAffectedByConfigChange(oldCfg, newCfg)).toEqual([]);
  });

  it('returns anthropic when its api_key changes', () => {
    const oldCfg = normalizeHarnessConfig({
      permissions: { default_mode: 'manual' },
      providers: {
        anthropic: { api_key: 'sk-old' },
        openai: { api_key: 'sk-o' },
      },
    });
    const newCfg = normalizeHarnessConfig({
      permissions: { default_mode: 'manual' },
      providers: {
        anthropic: { api_key: 'sk-new' },
        openai: { api_key: 'sk-o' },
      },
    });
    expect(providersAffectedByConfigChange(oldCfg, newCfg)).toEqual(['anthropic']);
  });

  it('treats empty and missing api_key consistently', () => {
    const withEmpty = providerDiscoveryFingerprint({ api_key: '' });
    const missing = providerDiscoveryFingerprint({});
    expect(withEmpty).toBe(missing);
    expect(withEmpty).toContain('api_key=');
  });

  it('detects api_url changes', () => {
    const oldCfg = normalizeHarnessConfig({
      permissions: { default_mode: 'manual' },
      providers: { lmstudio: { api_url: 'http://localhost:1234/v1/chat/completions' } },
    });
    const newCfg = normalizeHarnessConfig({
      permissions: { default_mode: 'manual' },
      providers: { lmstudio: { api_url: 'http://localhost:5678/v1/chat/completions' } },
    });
    expect(providersAffectedByConfigChange(oldCfg, newCfg)).toEqual(['lmstudio']);
  });
});
