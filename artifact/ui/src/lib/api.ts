/**
 * Wire types mirroring `src/store.rs` and the function responses in
 * `src/functions/*`, plus the one place the UI calls the worker.
 */

import type { Host } from '@iii-dev/console-ui'

export type ArtifactFormat = 'html' | 'markdown' | 'svg'

export interface VersionMeta {
  version: number
  created_at: number
  bytes: number
  note: string | null
}

export interface ArtifactRecord {
  id: string
  title: string
  kind: string
  format: ArtifactFormat
  latest_version: number
  versions: VersionMeta[]
  created_at: number
  updated_at: number
}

export interface IndexEntry {
  id: string
  title: string
  kind: string
  format: ArtifactFormat
  latest_version: number
  updated_at: number
}

export interface GetResponse {
  artifact: ArtifactRecord
  version: number
  content: string
}

export interface ListResponse {
  artifacts: IndexEntry[]
  total: number
  truncated: boolean
}

export interface ShareView {
  token: string
  artifact_id: string
  version: number
  public: boolean
  created_at: number
  expires_at: number
  expires_at_utc: string
  local_url: string
  public_url: string | null
  tunnel_status: string | null
  tunnel_error: string | null
}

export interface SharesResponse {
  shares: ShareView[]
}

export interface ShareRequest {
  id: string
  version?: number
  hours?: number
  public?: boolean
}

/** The page id the chat card opens. */
export const PAGE_ID = 'artifact-manager'

/** Functions whose calls change what the page shows. */
export const MUTATING_FUNCTIONS = [
  'artifact::create',
  'artifact::update',
  'artifact::delete',
  'artifact::share',
  'artifact::revoke',
] as const

/** Sharing waits for a starting tunnel (`tunnel_ready_timeout_ms`). */
const SHARE_TIMEOUT_MS = 40_000

export function createApi(host: Host) {
  const call = <T,>(id: string, payload: Record<string, unknown>, timeoutMs?: number) =>
    host.iii.call<T>(id, payload, timeoutMs ? { timeoutMs } : undefined)
  return {
    list: () => call<ListResponse>('artifact::list', {}),
    get: (id: string, version?: number) =>
      call<GetResponse>('artifact::get', version ? { id, version } : { id }),
    remove: (id: string) => call<{ deleted: boolean }>('artifact::delete', { id }),
    share: (req: ShareRequest) =>
      call<ShareView>('artifact::share', { ...req }, SHARE_TIMEOUT_MS),
    shares: (id?: string) => call<SharesResponse>('artifact::shares', id ? { id } : {}),
    revoke: (token: string) => call<{ revoked: boolean }>('artifact::revoke', { token }),
  }
}

export type ArtifactApi = ReturnType<typeof createApi>

/** `in 23h`, `in 45m`, `expired`: how long a share has left. */
export function expiresLabel(expiresAt: number, now = Date.now()): string {
  const minutes = Math.floor((expiresAt * 1000 - now) / 60_000)
  if (minutes <= 0) return 'expired'
  if (minutes < 60) return `in ${minutes}m`
  const hours = Math.floor(minutes / 60)
  return hours < 48 ? `in ${hours}h` : `in ${Math.floor(hours / 24)}d`
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

/** The artifact record inside any artifact::* response that carries one. */
export function recordOf(value: unknown): ArtifactRecord | null {
  if (!isRecord(value)) return null
  const candidate = isRecord(value.artifact) ? value.artifact : value
  return typeof candidate.id === 'string' &&
    typeof candidate.title === 'string' &&
    typeof candidate.latest_version === 'number'
    ? (candidate as unknown as ArtifactRecord)
    : null
}

export function shareOf(value: unknown): ShareView | null {
  return isRecord(value) && typeof value.token === 'string' && typeof value.local_url === 'string'
    ? (value as unknown as ShareView)
    : null
}
