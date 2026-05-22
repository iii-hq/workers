import { describe, expect, it } from 'vitest';
import { syncGet, syncList } from '../../src/models-catalog/catalog.js';

describe('catalog seed: llamacpp', () => {
  it('lists only `llamacpp-local` for provider=llamacpp', async () => {
    const models = await syncList({ provider: 'llamacpp' });
    const ids = models.map((m) => m.id);
    expect(ids).toEqual(['llamacpp-local']);
  });

  it('seeds `llamacpp-local` with openai-completions wire format and tool support', async () => {
    const m = await syncGet('llamacpp', 'llamacpp-local');
    expect(m, 'llamacpp-local should be seeded').not.toBeNull();
    expect(m?.api).toBe('openai-completions');
    expect(m?.display_name).toBe('llama.cpp (local)');
    expect(m?.context_window).toBe(32768);
    expect(m?.max_output_tokens).toBe(8192);
    expect(m?.supports_tools).toBe(true);
    expect(m?.supports_thinking).toBe(false);
    expect(m?.supports_vision).toBe(false);
    expect(m?.supports_cache).toBe(false);
    expect(m?.transports).toEqual(['sse']);
  });

  it('falls back to the llamacpp-local template for any user-loaded model id', async () => {
    for (const id of [
      'Meta-Llama-3.1-8B-Instruct',
      'qwen2.5-7b-instruct-q4_k_m',
      'whatever-llama-server-loaded',
    ]) {
      const m = await syncGet('llamacpp', id);
      expect(m, `llamacpp + ${id} should fall back to the placeholder`).not.toBeNull();
      expect(m?.id).toBe('llamacpp-local');
      expect(m?.supports_tools).toBe(true);
    }
  });
});
