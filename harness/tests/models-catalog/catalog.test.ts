import { describe, expect, it } from 'vitest';
import { loadEmbeddedCatalog, syncGet, syncList } from '../../src/models-catalog/catalog.js';

describe('embedded catalog', () => {
  it('loads non-empty', async () => {
    const all = await loadEmbeddedCatalog();
    expect(all.length).toBeGreaterThan(0);
  });

  it('finds a known anthropic model with xhigh support', async () => {
    const m = await syncGet('anthropic', 'claude-opus-4-7');
    expect(m).not.toBeNull();
    expect(m?.context_window).toBeGreaterThan(0);
  });

  it('list filters by provider', async () => {
    const anth = await syncList({ provider: 'anthropic' });
    expect(anth.every((m) => m.provider === 'anthropic')).toBe(true);
    expect(anth.length).toBeGreaterThan(0);
  });

  it('list filters by xhigh capability', async () => {
    const xhigh = await syncList({
      capability: { type: 'thinking_level', level: 'xhigh' },
    });
    expect(xhigh.every((m) => m.supports_xhigh)).toBe(true);
  });
});
