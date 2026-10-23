/**
 * The `secrets` worker, as the configuration form uses it.
 *
 * Bucket credentials never need to live in the `storage` configuration: the
 * form stores the value in the secrets worker and writes only a reference,
 * `secret://NAME`, which the storage worker resolves (as consumer `storage`)
 * when it builds the bucket. A pasted value travels browser → engine →
 * `secrets::set` and is kept nowhere else.
 *
 * Mirrors the database worker's client: storing merges the consumers a
 * secret already has, so saving a key for the storage worker never revokes
 * another worker's access to the same name. The store's masked `hint` is
 * read but never shown: on a short key it gives away most of the value.
 */

import type { Host } from '@iii-dev/console-ui'
import { isMissingFunction } from './errors'

export const SECRET_REF_PREFIX = 'secret://'
export const ENV_REF_PREFIX = 'env://'
/** The worker that resolves these references. */
export const SECRET_CONSUMER = 'storage'

const NAME_RE = /^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/

export function secretRef(name: string): string {
  return `${SECRET_REF_PREFIX}${name}`
}

function refName(value: unknown, prefix: string): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  if (trimmed.slice(0, prefix.length).toLowerCase() !== prefix) return null
  const name = trimmed.slice(prefix.length)
  return NAME_RE.test(name) ? name : null
}

/** The name in a `secret://NAME` value, `null` for anything else. */
export function secretRefName(value: unknown): string | null {
  return refName(value, SECRET_REF_PREFIX)
}

/** The name in an `env://NAME` value, `null` for anything else. */
export function envRefName(value: unknown): string | null {
  return refName(value, ENV_REF_PREFIX)
}

/** One secret's metadata — never its value. */
export interface SecretMeta {
  name: string
  consumers: string[]
  store?: string
  /** Masked by the store; kept for completeness, never rendered. */
  hint?: string
  description?: string
  updated_at?: string
}

export function parseSecretMeta(raw: unknown): SecretMeta | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const rec = raw as Record<string, unknown>
  if (typeof rec.name !== 'string') return null
  const text = (v: unknown) => (typeof v === 'string' ? v : undefined)
  return {
    name: rec.name,
    consumers: Array.isArray(rec.consumers) ? rec.consumers.filter((c): c is string => typeof c === 'string') : [],
    store: text(rec.store),
    hint: text(rec.hint),
    description: text(rec.description),
    updated_at: text(rec.updated_at),
  }
}

/**
 * One secret's metadata. `undefined` when the secrets worker is not running,
 * `null` when it runs but holds nothing by that name (for the `env` store:
 * the variable is not shared with anyone).
 */
export async function getSecret(host: Host, name: string, store?: 'env'): Promise<SecretMeta | null | undefined> {
  let raw: unknown
  try {
    raw = await host.iii.trigger<unknown>('secrets::get', store ? { name, store } : { name }, { timeoutMs: 10_000 })
  } catch (error) {
    if (isMissingFunction(error)) return undefined
    throw error
  }
  return raw == null ? null : parseSecretMeta(raw)
}

/** Consumers a store call should leave in place: the existing ones plus ours. */
export function mergeConsumers(existing: readonly string[] | undefined, required: readonly string[]): string[] {
  return [...new Set([...(existing ?? []), ...required])]
}

export const SECRETS_DOWN_MESSAGE =
  'The secrets worker is not running — start it to keep credentials out of configuration.'

/**
 * Create or rotate `name` with `value`, readable by the storage worker and
 * by whoever could read it before. Resolves to the reference the
 * configuration should hold.
 */
export async function storeSecret(
  host: Host,
  input: { name: string; value: string; description: string },
): Promise<string> {
  const existing = await getSecret(host, input.name).catch(() => null)
  if (existing === undefined) throw new Error(SECRETS_DOWN_MESSAGE)
  await host.iii.trigger<unknown>(
    'secrets::set',
    {
      name: input.name,
      value: input.value,
      consumers: mergeConsumers(existing?.consumers, [SECRET_CONSUMER]),
      description: input.description,
    },
    { timeoutMs: 20_000 },
  )
  return secretRef(input.name)
}

/** "updated 5m ago" for a secret's `updated_at`, or `null` when unknown. */
export function updatedLabel(updatedAt: string | undefined, now = Date.now()): string | null {
  if (!updatedAt) return null
  const at = Date.parse(updatedAt)
  if (Number.isNaN(at)) return null
  const seconds = Math.max(0, Math.round((now - at) / 1000))
  if (seconds < 60) return 'updated just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `updated ${minutes}m ago`
  const hours = Math.round(minutes / 60)
  if (hours < 48) return `updated ${hours}h ago`
  return `updated ${Math.round(hours / 24)}d ago`
}

/**
 * A `secret://` credential's line: never the store's masked hint (its first
 * and last characters say too much of a short key), only that it is stored
 * and since when. `meta`: `undefined` while reading, `null` when missing.
 */
export function secretStatusText(
  meta: SecretMeta | null | undefined,
  secretsUp: boolean | undefined,
  now = Date.now(),
): string {
  if (secretsUp === false) return 'The secrets worker is not running'
  if (meta === undefined) return 'Checking the secrets store…'
  if (meta === null) return 'Missing from the secrets store — the bucket cannot resolve it'
  return ['Stored in secrets', updatedLabel(meta.updated_at, now)].filter(Boolean).join(' · ')
}

/** An `env://` credential's line: whether the secrets worker's env file sets it, never a hint of the value. */
export function envStatusText(meta: SecretMeta | null | undefined, secretsUp: boolean | undefined): string {
  if (secretsUp === false) return 'The secrets worker is not running'
  if (meta === undefined) return 'A variable of the secrets worker’s env file'
  if (meta === null) return 'Not shared with the storage worker'
  if (!meta.hint) return 'Not set in the secrets worker’s env file'
  return 'Set in the secrets worker’s env file'
}
