import { beforeEach, describe, expect, it, vi } from 'vitest';
import { buildConfig } from '../../src/provider-anthropic/auth.js';
import type { WorkerConfig } from '../../src/provider-anthropic/config.js';
import type { ISdk } from '../../src/runtime/iii.js';

const WORKER: WorkerConfig = {
  default_api_url: 'https://api.anthropic.com/v1/messages',
  default_max_tokens: 8192,
};

const { resolveProviderMock } = vi.hoisted(() => ({
  resolveProviderMock: vi.fn(),
}));

vi.mock('../../src/runtime/provider-resolve.js', () => ({
  resolveProvider: resolveProviderMock,
}));

function resolved(max_tokens: number | null) {
  return {
    configured: true,
    credential: { type: 'api_key', key: 'sk-test' },
    api_url: null,
    max_tokens,
    source: 'stored',
  };
}

/** Fake ISdk whose models::get returns the given catalog entry (or throws). */
function iiiWithCatalog(entry: unknown, opts: { throws?: boolean } = {}): ISdk {
  const trigger = vi.fn().mockImplementation(async (req: { function_id: string }) => {
    if (req.function_id === 'models::get') {
      if (opts.throws) throw new Error('bus timeout');
      return entry;
    }
    return null;
  });
  return { trigger } as unknown as ISdk;
}

describe('buildConfig max_tokens resolution', () => {
  beforeEach(() => {
    resolveProviderMock.mockReset();
  });

  it('defaults to min(catalog max, 32k) — 64k model clamps to 32k', async () => {
    resolveProviderMock.mockResolvedValue(resolved(null));
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    const cfg = await buildConfig(iii, WORKER, 'claude-sonnet-4-6');
    expect(cfg.max_tokens).toBe(32_000);
    expect(cfg.catalog?.max_output_tokens).toBe(64_000);
  });

  it('uses the catalog max when below the cap', async () => {
    resolveProviderMock.mockResolvedValue(resolved(null));
    const iii = iiiWithCatalog({ id: 'claude-haiku-4-5', max_output_tokens: 16_000 });
    const cfg = await buildConfig(iii, WORKER, 'claude-haiku-4-5');
    expect(cfg.max_tokens).toBe(16_000);
  });

  it('registry override wins below the model ceiling', async () => {
    resolveProviderMock.mockResolvedValue(resolved(50_000));
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    const cfg = await buildConfig(iii, WORKER, 'claude-sonnet-4-6');
    expect(cfg.max_tokens).toBe(50_000);
  });

  it('registry override is clamped to the model ceiling', async () => {
    resolveProviderMock.mockResolvedValue(resolved(100_000));
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    const cfg = await buildConfig(iii, WORKER, 'claude-sonnet-4-6');
    expect(cfg.max_tokens).toBe(64_000);
  });

  it('falls back to the registry/worker default when the catalog lookup fails', async () => {
    resolveProviderMock.mockResolvedValue(resolved(null));
    const cfg = await buildConfig(iiiWithCatalog(null, { throws: true }), WORKER, 'claude-x');
    expect(cfg.max_tokens).toBe(WORKER.default_max_tokens);
    expect(cfg.catalog).toBeUndefined();
  });

  it('falls back to the worker default when the model is unknown', async () => {
    resolveProviderMock.mockResolvedValue(resolved(null));
    const cfg = await buildConfig(iiiWithCatalog(null), WORKER, 'claude-unknown');
    expect(cfg.max_tokens).toBe(WORKER.default_max_tokens);
  });

  it('throws without a credential', async () => {
    resolveProviderMock.mockResolvedValue({
      configured: false,
      credential: null,
      api_url: null,
      max_tokens: null,
      source: null,
    });
    await expect(buildConfig(iiiWithCatalog(null), WORKER, 'claude-x')).rejects.toThrow(
      /no credential/,
    );
  });
});
