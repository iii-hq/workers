import { describe, expect, it } from 'vitest';
import { syncGet, syncList } from '../../src/models-catalog/catalog.js';

describe('catalog seed: kimi', () => {
  it('lists only K2.6 and K2.5 for provider=kimi', async () => {
    const models = await syncList({ provider: 'kimi' });
    const ids = models.map((m) => m.id).sort();
    expect(ids).toEqual(['kimi-k2.5', 'kimi-k2.6']);
  });

  it('flags K2.6 and K2.5 as thinking + vision + tool capable with a 256k window', async () => {
    for (const id of ['kimi-k2.6', 'kimi-k2.5']) {
      const m = await syncGet('kimi', id);
      expect(m, `model ${id} should be seeded`).not.toBeNull();
      expect(m?.api).toBe('openai-completions');
      expect(m?.context_window).toBe(256000);
      expect(m?.supports_thinking).toBe(true);
      expect(m?.supports_vision).toBe(true);
      expect(m?.supports_tools).toBe(true);
      expect(m?.supports_cache).toBe(true);
      expect(m?.transports).toEqual(['sse']);
    }
  });

  it('returns null for removed legacy ids', async () => {
    for (const id of ['kimi-k2-0905-preview', 'kimi-k2-turbo-preview', 'moonshot-v1-128k']) {
      expect(await syncGet('kimi', id), `${id} should no longer be seeded`).toBeNull();
    }
  });

  it('returns null for unknown kimi ids', async () => {
    expect(await syncGet('kimi', 'kimi-k2-fictional')).toBeNull();
  });
});
