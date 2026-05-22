import { describe, expect, it } from 'vitest';
import { syncGet, syncList } from '../../src/models-catalog/catalog.js';

describe('catalog seed: lmstudio', () => {
  it('lists only `lmstudio-local` for provider=lmstudio', async () => {
    const models = await syncList({ provider: 'lmstudio' });
    const ids = models.map((m) => m.id);
    expect(ids).toEqual(['lmstudio-local']);
  });

  it('seeds `lmstudio-local` with openai-completions wire format and tool support', async () => {
    const m = await syncGet('lmstudio', 'lmstudio-local');
    expect(m, 'lmstudio-local should be seeded').not.toBeNull();
    expect(m?.api).toBe('openai-completions');
    expect(m?.display_name).toBe('LM Studio (local)');
    expect(m?.context_window).toBe(32768);
    expect(m?.max_output_tokens).toBe(8192);
    expect(m?.supports_tools).toBe(true);
    expect(m?.supports_thinking).toBe(false);
    expect(m?.supports_vision).toBe(false);
    expect(m?.supports_cache).toBe(false);
    expect(m?.transports).toEqual(['sse']);
  });

  it('falls back to the lmstudio-local template for any user-supplied LM Studio model id', async () => {
    for (const id of [
      'qwen/qwen3-4b-2507',
      'lmstudio-community/Meta-Llama-3.1-8B-Instruct-GGUF',
      'whatever-the-user-loaded',
    ]) {
      const m = await syncGet('lmstudio', id);
      expect(m, `lmstudio + ${id} should fall back to the placeholder`).not.toBeNull();
      // The fallback returns the placeholder's id, not the requested id.
      expect(m?.id).toBe('lmstudio-local');
      expect(m?.supports_tools).toBe(true);
    }
  });

  it('keeps the strict null-on-miss behaviour for non-lmstudio providers', async () => {
    expect(await syncGet('kimi', 'kimi-k2-fictional')).toBeNull();
    expect(await syncGet('openai', 'gpt-99')).toBeNull();
  });
});
