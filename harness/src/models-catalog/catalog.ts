/**
 * Embedded model seed loaded from `models.json`. Used both as the
 * sync-fallback catalog and as the bus-state seed when the engine's
 * `models:` prefix is empty.
 *
 * `models.json` is imported statically so esbuild can inline it into the
 * single-file bundle (`dist/bundle/index.mjs`). The async signatures are
 * preserved so callers (state.ts) don't need to change.
 */

import modelsJson from './models.json' with { type: 'json' };
import type { Model } from './types.js';

const EMBEDDED: readonly Model[] = (modelsJson as { models?: Model[] }).models ?? [];

export async function loadEmbeddedCatalog(): Promise<Model[]> {
  return [...EMBEDDED];
}

export type ListFilter = {
  provider?: string;
  capability?: import('./types.js').Capability;
};

export async function syncList(filter: ListFilter): Promise<Model[]> {
  const all = await loadEmbeddedCatalog();
  const { supportsModel } = await import('./types.js');
  return all
    .filter((m) => filter.provider === undefined || m.provider === filter.provider)
    .filter((m) => filter.capability === undefined || supportsModel(m, filter.capability));
}

export async function syncGet(provider: string, model_id: string): Promise<Model | null> {
  const all = await loadEmbeddedCatalog();
  const exact = all.find((m) => m.provider === provider && m.id === model_id);
  if (exact) return exact;
  // LM Studio model IDs are user-controlled (any GGUF the user has loaded),
  // so an exact catalog match is the exception not the rule. Fall back to the
  // `lmstudio-local` placeholder so capability gating (supports_tools, etc.)
  // still works for arbitrary user-supplied IDs. Other providers keep the
  // strict null-on-miss behaviour.
  if (provider === 'lmstudio') {
    return all.find((m) => m.provider === 'lmstudio' && m.id === 'lmstudio-local') ?? null;
  }
  return null;
}
