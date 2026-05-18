/**
 * Embedded model seed loaded from `models.json`. Used both as the
 * sync-fallback catalog and as the bus-state seed when the engine's
 * `models:` prefix is empty.
 */

import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Model } from './types.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

let cached: Model[] | null = null;

export async function loadEmbeddedCatalog(): Promise<Model[]> {
  if (cached) return cached;
  const text = await readFile(join(__dirname, 'models.json'), 'utf8');
  const parsed = JSON.parse(text) as { models?: Model[] };
  cached = parsed.models ?? [];
  return cached;
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
  return all.find((m) => m.provider === provider && m.id === model_id) ?? null;
}
