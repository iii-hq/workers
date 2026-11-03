/**
 * The `secrets` worker, as the configuration form uses it.
 *
 * A Cloud credential does not need to live in the WhatsApp configuration: the
 * form stores the value in the secrets worker and writes only a reference,
 * `secret://NAME`, which the WhatsApp worker resolves as consumer `whatsapp`.
 * A pasted value travels browser → engine → `secrets::set` and is kept nowhere
 * else. Storing merges the consumers a secret already has, so saving it for
 * WhatsApp never revokes another worker's access. The store's masked hint is
 * never shown: on a short token it gives away most of the value.
 */

import type { Host } from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'

export const SECRET_CONSUMER = 'whatsapp'
const NAME_RE = /^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/

function refName(value: unknown, prefix: string): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  if (trimmed.slice(0, prefix.length).toLowerCase() !== prefix) return null
  const name = trimmed.slice(prefix.length)
  return NAME_RE.test(name) ? name : null
}

/** The name in a `secret://NAME` value, `null` for anything else. */
export const secretRefName = (value: unknown) => refName(value, 'secret://')
/** The name in an `env://NAME` value, `null` for anything else. */
export const envRefName = (value: unknown) => refName(value, 'env://')

/**
 * Where a credential typed into this configuration is stored: the name of its
 * environment variable, suffixed with the configuration id when it is not the
 * default one, so two WhatsApp configurations never share a secret by accident.
 */
export function credentialSecretName(environment: string, configurationId: string): string {
  if (configurationId === 'whatsapp') return environment
  return `${environment}_${configurationId.toUpperCase().replace(/[^A-Z0-9_.-]/g, '_')}`.slice(0, 128)
}

/** One secret's metadata, never its value. */
export interface SecretMeta {
  name: string
  consumers: string[]
  updated_at?: string
}

function parseSecretMeta(raw: unknown): SecretMeta | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const rec = raw as Record<string, unknown>
  if (typeof rec.name !== 'string') return null
  return {
    name: rec.name,
    consumers: Array.isArray(rec.consumers) ? rec.consumers.filter((c): c is string => typeof c === 'string') : [],
    updated_at: typeof rec.updated_at === 'string' ? rec.updated_at : undefined,
  }
}

export function isMissingFunction(error: unknown): boolean {
  return errorCode(error)?.toLowerCase() === 'function_not_found'
    || /function[_ ]not[_ ]found|function \S+ (?:is )?not (?:found|registered)/i.test(errorMessage(error))
}

/**
 * One secret's metadata: `undefined` when the secrets worker is not running,
 * `null` when it holds nothing by that name (for `env`: the variable is not
 * shared with any worker).
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

export const SECRETS_DOWN_MESSAGE = 'The secrets worker is not running. Start it to keep credentials out of configuration.'

/** Create or rotate `name` with `value`, readable by WhatsApp and whoever could read it before. Resolves to the reference to save. */
export async function storeSecret(host: Host, input: { name: string; value: string; description: string }): Promise<string> {
  const existing = await getSecret(host, input.name).catch(() => null)
  if (existing === undefined) throw new Error(SECRETS_DOWN_MESSAGE)
  await host.iii.trigger<unknown>('secrets::set', {
    name: input.name,
    value: input.value,
    consumers: [...new Set([...(existing?.consumers ?? []), SECRET_CONSUMER])],
    description: input.description,
  }, { timeoutMs: 20_000 })
  return `secret://${input.name}`
}

/** "updated 5m ago" for a secret's `updated_at`, or `null` when unknown. */
export function updatedLabel(updatedAt: string | undefined, now = Date.now()): string | null {
  if (!updatedAt) return null
  const at = Date.parse(updatedAt)
  if (Number.isNaN(at)) return null
  const minutes = Math.round(Math.max(0, now - at) / 60000)
  if (minutes < 1) return 'updated just now'
  if (minutes < 60) return `updated ${minutes}m ago`
  const hours = Math.round(minutes / 60)
  return hours < 48 ? `updated ${hours}h ago` : `updated ${Math.round(hours / 24)}d ago`
}

/**
 * The status line of a referenced credential and whether it is a problem.
 * `meta`: `undefined` while reading, `null` when missing. `secretsUp`:
 * `false` when the secrets worker is not running.
 */
export function referenceStatus(kind: 'secret' | 'env', meta: SecretMeta | null | undefined, secretsUp: boolean | undefined, now = Date.now()): { text: string; warn: boolean } {
  if (secretsUp === false) return { text: 'The secrets worker is not running', warn: true }
  if (meta === undefined) return { text: 'Checking the secrets store…', warn: false }
  if (meta === null) return { text: kind === 'secret' ? 'Missing from the secrets store' : 'Not shared with any worker', warn: true }
  if (!meta.consumers.includes(SECRET_CONSUMER)) return { text: 'Not shared with the WhatsApp worker', warn: true }
  if (kind === 'env') return { text: 'Read from the secrets worker’s env file', warn: false }
  return { text: ['Stored in secrets', updatedLabel(meta.updated_at, now)].filter(Boolean).join(' · '), warn: false }
}
