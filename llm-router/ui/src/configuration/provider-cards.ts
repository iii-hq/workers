/**
 * Build the provider card list from both the registered schema and the saved
 * configuration. A fresh installation has a null configuration value, while
 * its schema already contains the providers that operators can configure.
 *
 * Runtime status comes from `router::provider::list`: a card that cannot
 * serve must not look identical to one that can.
 */

type JsonObject = Record<string, unknown>

function asObject(value: unknown): JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as JsonObject) : {}
}

export function schemaProviderIds(schema: Record<string, unknown> | null): string[] {
  const properties = asObject(schema?.properties)
  const providers = asObject(properties.providers)
  return Object.keys(asObject(providers.properties))
}

export function providerCardIds(schema: Record<string, unknown> | null, value: unknown): string[] {
  const configuredProviders = asObject(asObject(value).providers)
  return [...new Set([...schemaProviderIds(schema), ...Object.keys(configuredProviders)])]
}

export type LiveProvider = {
  id: string
  display_name: string
  available: boolean
}

export type ProviderRuntimeStatus = 'unknown' | 'loaded' | 'not-loaded' | 'not-connected'

export type ProviderFilter = 'all' | 'needs-key' | 'ready' | 'not-loaded'

export type ProviderBucket = Exclude<ProviderFilter, 'all'>

/** Wire shape of `router::provider::list`. */
export function parseProviderList(raw: unknown): LiveProvider[] {
  const rows = asObject(raw).providers
  if (!Array.isArray(rows)) return []
  const out: LiveProvider[] = []
  for (const row of rows) {
    const o = asObject(row)
    const id = typeof o.id === 'string' ? o.id : ''
    if (!id) continue
    out.push({
      id,
      display_name: typeof o.display_name === 'string' ? o.display_name : id,
      // Absent on older routers — treat as available (previous behavior).
      available: o.available !== false,
    })
  }
  return out
}

/**
 * `live === null` means the list has not been fetched yet, so the form
 * must not flash "not loaded" on every mount.
 */
export function providerRuntimeStatus(
  id: string,
  live: LiveProvider[] | null,
  schemaIds: string[],
): ProviderRuntimeStatus {
  if (live === null) return 'unknown'
  const row = live.find((p) => p.id === id)
  if (row) return row.available ? 'loaded' : 'not-loaded'
  if (schemaIds.includes(id)) return 'not-loaded'
  return 'not-connected'
}

export function providerBucket(status: ProviderRuntimeStatus, hasKey: boolean): ProviderBucket {
  if (status === 'not-loaded' || status === 'not-connected') return 'not-loaded'
  return hasKey ? 'ready' : 'needs-key'
}

const BUCKET_ORDER: Record<ProviderBucket, number> = {
  'needs-key': 0,
  ready: 1,
  'not-loaded': 2,
}

export function visibleProviderIds(args: {
  ids: string[]
  schemaIds: string[]
  live: LiveProvider[] | null
  hasKey: (id: string) => boolean
  filter: ProviderFilter
}): string[] {
  const ranked = args.ids.map((id) => {
    const status = providerRuntimeStatus(id, args.live, args.schemaIds)
    const bucket = providerBucket(status, args.hasKey(id))
    return { id, bucket }
  })
  const filtered = args.filter === 'all' ? ranked : ranked.filter((row) => row.bucket === args.filter)
  return filtered
    .sort((a, b) => {
      const d = BUCKET_ORDER[a.bucket] - BUCKET_ORDER[b.bucket]
      return d !== 0 ? d : a.id.localeCompare(b.id)
    })
    .map((row) => row.id)
}

const TITLE_PARTS: Record<string, string> = {
  openai: 'OpenAI',
  github: 'GitHub',
  llamacpp: 'llama.cpp',
  xai: 'xAI',
  zai: 'Z.ai',
}

export function humanizeProviderId(id: string): string {
  return id
    .split(/[-_]+/)
    .map((part) => {
      const mapped = TITLE_PARTS[part.toLowerCase()]
      if (mapped) return mapped
      return part.charAt(0).toUpperCase() + part.slice(1)
    })
    .join(' ')
}

export function providerDisplayName(id: string, liveName?: string): string {
  if (liveName && liveName !== id) return liveName
  return humanizeProviderId(id)
}

export function sliceHasKey(slice: unknown): boolean {
  const key = asObject(slice).api_key
  return typeof key === 'string' && key.trim().length > 0
}

/** Suggest an env-var name for a provider: `ANTHROPIC_API_KEY`. */
export function suggestedEnvVar(providerId: string): string {
  const slug = providerId.toUpperCase().replace(/[^A-Z0-9]+/g, '_')
  return `${slug || 'PROVIDER'}_API_KEY`
}

/** Empty-field hint — must not look like a value already saved in the box. */
export function apiKeyPlaceholder(providerId: string): string {
  return `env var, e.g. \${${suggestedEnvVar(providerId)}}`
}
