/**
 * The `secrets` worker, as the configuration form uses it.
 *
 * Connection credentials never live in the `database` configuration: the
 * form stores the value in the secrets worker and writes only a reference,
 * `secret://NAME`, which the database worker resolves (as consumer
 * `database`) when it builds the pool. A pasted value travels browser →
 * engine → `secrets::set` and is kept nowhere else.
 *
 * Mirrors the ADE's own client (ade/web/src/lib/secrets.ts): storing merges
 * the consumers a secret already has, so saving a password for the database
 * worker never revokes another worker's access to the same name.
 */

import type { Host } from '@iii-dev/console-ui'
import { z } from 'zod'
import { errText } from './errors'

export const SECRET_REF_PREFIX = 'secret://'
export const ENV_REF_PREFIX = 'env://'
/** The worker that resolves these references. */
export const SECRET_CONSUMER = 'database'

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

export const secretMetaSchema = z.object({
  name: z.string(),
  hint: z.string().nullish(),
  consumers: z
    .array(z.string())
    .nullish()
    .transform((v) => v ?? []),
  store: z.string().nullish(),
  description: z.string().nullish(),
  updated_at: z.string().nullish(),
})
export type SecretMeta = z.infer<typeof secretMetaSchema>

/** A call to a function the engine does not have — the worker is not running. */
export function isMissingFunction(error: unknown): boolean {
  return /function[_ ]not[_ ]found|not[_ ]found|no such function|not registered/i.test(errText(error))
}

/**
 * One secret's metadata — never its value. `undefined` when the secrets
 * worker is not running, `null` when it runs but holds nothing by that name.
 */
export async function getSecret(host: Host, name: string): Promise<SecretMeta | null | undefined> {
  let raw: unknown
  try {
    raw = await host.iii.trigger<unknown>('secrets::get', { name }, { timeoutMs: 10_000 })
  } catch (error) {
    if (isMissingFunction(error)) return undefined
    throw error
  }
  if (raw == null) return null
  const parsed = secretMetaSchema.safeParse(raw)
  return parsed.success ? parsed.data : null
}

/** Consumers a store call should leave in place: the existing ones plus ours. */
export function mergeConsumers(existing: readonly string[] | undefined, required: readonly string[]): string[] {
  return [...new Set([...(existing ?? []), ...required])]
}

/**
 * Create or rotate `name` with `value`, readable by the database worker and
 * by whoever could read it before. Resolves to the reference the
 * configuration should hold.
 */
export async function storeSecret(
  host: Host,
  input: { name: string; value: string; description: string },
): Promise<string> {
  const existing = await getSecret(host, input.name).catch(() => null)
  if (existing === undefined) {
    throw new Error('The secrets worker is not running — start it to keep credentials out of configuration.')
  }
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
