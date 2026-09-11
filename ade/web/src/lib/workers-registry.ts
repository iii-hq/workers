/**
 * Read-only client for the public workers registry (`api.workers.iii.dev`),
 * the catalog `compose::add` and the CLI resolve worker names against. The
 * registry answers with `access-control-allow-origin: *`, so the browser
 * reads it directly and the model picker can offer providers to install
 * before any directory worker is present.
 *
 * Only the `tag=provider` listing is used here; the row shape mirrors the
 * registry's `WorkerListItem` (see `iii-directory/src/functions/registry.rs`).
 */

export const WORKERS_REGISTRY_URL = 'https://api.workers.iii.dev'
/** Registry tag every llm-router provider worker publishes under. */
export const PROVIDER_TAG = 'provider'
/** The registry pages at 20; providers fit in one page, this is a guard. */
const MAX_PAGES = 10

export interface RegistryWorker {
  /** Registry slug — also the `compose::add` worker name. */
  name: string
  description: string | null
  /** Latest published version. */
  version: string | null
  tags: string[]
  totalDownloads: number
  authorName: string | null
  authorVerified: boolean
}

export interface RegistryWorkersPage {
  workers: RegistryWorker[]
  nextCursor: string | null
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null
}

export function parseRegistryWorkersPage(body: unknown): RegistryWorkersPage {
  const root = asRecord(body)
  const rows = Array.isArray(root?.workers) ? root.workers : []
  const workers: RegistryWorker[] = []
  for (const raw of rows) {
    const row = asRecord(raw)
    const name = asString(row?.name)
    if (!row || !name) continue
    const author = asRecord(row.author)
    workers.push({
      name,
      description: asString(row.description),
      version: asString(row.version),
      tags: Array.isArray(row.tags)
        ? row.tags.filter((tag): tag is string => typeof tag === 'string')
        : [],
      totalDownloads:
        typeof row.total_downloads === 'number' && row.total_downloads >= 0
          ? row.total_downloads
          : 0,
      authorName: asString(author?.name),
      authorVerified: author?.verified === true,
    })
  }
  const pagination = asRecord(root?.pagination)
  return {
    workers,
    nextCursor:
      pagination?.has_more === true ? asString(pagination.next_cursor) : null,
  }
}

export interface FetchRegistryProvidersOptions {
  fetch?: typeof fetch
  signal?: AbortSignal
  baseUrl?: string
}

/**
 * Every worker tagged `provider`, in the registry's order (downloads,
 * descending). Follows pagination; rejects on a network error or a non-2xx
 * status so the caller can show a retry.
 */
export async function fetchRegistryProviders({
  fetch: fetchImpl = fetch,
  signal,
  baseUrl = WORKERS_REGISTRY_URL,
}: FetchRegistryProvidersOptions = {}): Promise<RegistryWorker[]> {
  const workers: RegistryWorker[] = []
  const seen = new Set<string>()
  let cursor: string | null = null
  for (let page = 0; page < MAX_PAGES; page++) {
    const url = new URL('/w', baseUrl)
    url.searchParams.set('tag', PROVIDER_TAG)
    if (cursor) url.searchParams.set('cursor', cursor)
    const response = await fetchImpl(url.toString(), {
      signal,
      headers: { accept: 'application/json' },
    })
    if (!response.ok) {
      throw new Error(`registry returned HTTP ${response.status}`)
    }
    const parsed = parseRegistryWorkersPage(await response.json())
    for (const worker of parsed.workers) {
      if (seen.has(worker.name)) continue
      seen.add(worker.name)
      workers.push(worker)
    }
    cursor = parsed.nextCursor
    if (!cursor) break
  }
  return workers
}

/**
 * The llm-router provider id a registry worker most likely declares:
 * `provider-openai` → `openai`; a worker without the prefix (`cursor`) keeps
 * its name. Declared ids may still differ in punctuation
 * (`provider-opencode-go` declares `opencode_go`), so compare with
 * `registryWorkerMatchesProvider`.
 */
export function providerIdForRegistryWorker(name: string): string {
  return name.replace(/^provider-/, '')
}

function normalizeProviderKey(value: string): string {
  return value.toLowerCase().replace(/[-_.]/g, '')
}

export function registryWorkerMatchesProvider(
  workerName: string,
  providerId: string,
): boolean {
  return (
    normalizeProviderKey(providerIdForRegistryWorker(workerName)) ===
    normalizeProviderKey(providerId)
  )
}
