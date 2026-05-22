import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { DEFAULT_API_URL, loadWorkerConfig } from '../../src/provider-lmstudio/config.js';

describe('loadWorkerConfig (lmstudio)', () => {
  const original = process.env.LMSTUDIO_BASE_URL;

  beforeEach(() => {
    delete process.env.LMSTUDIO_BASE_URL;
  });

  afterEach(() => {
    if (original === undefined) delete process.env.LMSTUDIO_BASE_URL;
    else process.env.LMSTUDIO_BASE_URL = original;
  });

  it('uses the localhost default when no yaml value and no env var', () => {
    const cfg = loadWorkerConfig({});
    expect(cfg.default_api_url).toBe(DEFAULT_API_URL);
    expect(cfg.default_max_tokens).toBe(8192);
  });

  it('uses the yaml value when no env var is set', () => {
    const cfg = loadWorkerConfig({
      provider_lmstudio: {
        default_api_url: 'http://192.168.1.10:1234/v1/chat/completions',
        default_max_tokens: 4096,
      },
    });
    expect(cfg.default_api_url).toBe('http://192.168.1.10:1234/v1/chat/completions');
    expect(cfg.default_max_tokens).toBe(4096);
  });

  it('LMSTUDIO_BASE_URL env var overrides the yaml value (full URL form)', () => {
    process.env.LMSTUDIO_BASE_URL = 'http://192.168.1.206:1234/v1/chat/completions';
    const cfg = loadWorkerConfig({
      provider_lmstudio: { default_api_url: DEFAULT_API_URL },
    });
    expect(cfg.default_api_url).toBe('http://192.168.1.206:1234/v1/chat/completions');
  });

  it('LMSTUDIO_BASE_URL appends /v1/chat/completions when given just a base origin', () => {
    process.env.LMSTUDIO_BASE_URL = 'http://192.168.1.206:1234';
    const cfg = loadWorkerConfig({});
    expect(cfg.default_api_url).toBe('http://192.168.1.206:1234/v1/chat/completions');
  });

  it('LMSTUDIO_BASE_URL trims a trailing slash before appending the path', () => {
    process.env.LMSTUDIO_BASE_URL = 'http://lan-host:1234/';
    const cfg = loadWorkerConfig({});
    expect(cfg.default_api_url).toBe('http://lan-host:1234/v1/chat/completions');
  });

  it('LMSTUDIO_BASE_URL empty/whitespace falls through to yaml/default', () => {
    process.env.LMSTUDIO_BASE_URL = '   ';
    const cfg = loadWorkerConfig({
      provider_lmstudio: { default_api_url: 'http://yaml-host:5678/v1/chat/completions' },
    });
    expect(cfg.default_api_url).toBe('http://yaml-host:5678/v1/chat/completions');
  });

  it('rejects malformed LMSTUDIO_BASE_URL and falls through to yaml', () => {
    process.env.LMSTUDIO_BASE_URL = 'not a url at all';
    const cfg = loadWorkerConfig({
      provider_lmstudio: { default_api_url: 'http://yaml-host:5678/v1/chat/completions' },
    });
    expect(cfg.default_api_url).toBe('http://yaml-host:5678/v1/chat/completions');
  });

  it('rejects non-http(s) schemes and falls through to yaml', () => {
    process.env.LMSTUDIO_BASE_URL = 'file:///etc/passwd';
    const cfg = loadWorkerConfig({
      provider_lmstudio: { default_api_url: 'http://yaml-host:5678/v1/chat/completions' },
    });
    expect(cfg.default_api_url).toBe('http://yaml-host:5678/v1/chat/completions');
  });

  it('falls back to the localhost DEFAULT when BOTH env and yaml are malformed', () => {
    process.env.LMSTUDIO_BASE_URL = 'not-a-url';
    const cfg = loadWorkerConfig({
      provider_lmstudio: { default_api_url: 'also-not-a-url' },
    });
    expect(cfg.default_api_url).toBe(DEFAULT_API_URL);
  });
});
