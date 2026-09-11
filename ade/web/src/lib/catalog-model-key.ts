import { CATALOG_MODEL_KEY_SEP } from '@/types/chat'

export function makeCatalogModelKey(provider: string, modelId: string): string {
  return `${provider}${CATALOG_MODEL_KEY_SEP}${modelId}`
}

/** Parse `provider::id` from the router catalog into harness::send fields. */
export function parseCatalogModelKey(
  model: string,
): { provider: string; id: string } | null {
  const sep = CATALOG_MODEL_KEY_SEP
  const i = model.indexOf(sep)
  if (i <= 0) return null
  const provider = model.slice(0, i)
  const id = model.slice(i + sep.length)
  if (!provider || !id) return null
  return { provider, id }
}
