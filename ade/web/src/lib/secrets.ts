/**
 * Browser client for the `secrets` worker — the one place the console stores
 * a credential. Configuration keeps only a reference (`secret://NAME`); the
 * value lives encrypted in the secrets worker and is resolved by the workers
 * a secret names as its consumers.
 *
 * A key found on the machine is imported by the secrets worker itself
 * (`secrets::import`), so its value never reaches the browser. A pasted key
 * travels browser → engine → `secrets::set` and is kept nowhere else.
 */

import { getIiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'

export const SECRET_REF_PREFIX = 'secret://'
export const SECRETS_WORKER = 'secrets'

/** `secret://ANTHROPIC_API_KEY` */
export function secretRef(name: string): string {
  return `${SECRET_REF_PREFIX}${name}`
}

/** The name in a `secret://NAME` value, `null` for anything else. */
export function secretRefName(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  if (!trimmed.startsWith(SECRET_REF_PREFIX)) return null
  const name = trimmed.slice(SECRET_REF_PREFIX.length)
  return /^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/.test(name) ? name : null
}

/** `${VAR}` and nothing else: the engine expands it from its environment. */
export function envRefName(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const match = /^\$\{([A-Za-z_][A-Za-z0-9_]*)\}$/.exec(value.trim())
  return match ? match[1] : null
}

export type KeySourceKind = 'process_env' | 'dotenv' | 'login_shell'

/** Where `secrets::detect` saw a key. Never the value — `hint` is masked. */
export interface KeySource {
  kind: KeySourceKind
  location: string
  hint: string
  matches_stored: boolean
}

export interface KeyDetection {
  name: string
  stored: boolean
  stored_hint?: string | null
  sources: KeySource[]
}

export interface SecretMeta {
  name: string
  ref: string
  hint: string
  fingerprint: string
  consumers: string[]
  description?: string | null
  created_at: string
  updated_at: string
  last_resolved_at?: string | null
  last_resolved_by?: string | null
}

/** How a key reaches the secrets store. */
export type KeyInput =
  | { mode: 'import'; source: KeySourceKind }
  | { mode: 'paste'; value: string }
  /** Already stored under its name: only the reference is (re)written. */
  | { mode: 'stored' }

const SOURCE_LABEL: Record<KeySourceKind, string> = {
  login_shell: 'your shell profile',
  dotenv: "this project's .env",
  process_env: 'the secrets worker environment',
}

export function sourceLabel(source: KeySource | KeySourceKind): string {
  const kind = typeof source === 'string' ? source : source.kind
  return (
    SOURCE_LABEL[kind] ??
    (typeof source === 'string' ? source : source.location)
  )
}

/** The source a detected key should be imported from: the shell first. */
export function preferredSource(
  detection: KeyDetection | null | undefined,
): KeySource | null {
  if (!detection) return null
  const order: KeySourceKind[] = ['login_shell', 'dotenv', 'process_env']
  for (const kind of order) {
    const source = detection.sources.find((entry) => entry.kind === kind)
    if (source) return source
  }
  return null
}

/** A key input is complete when it names a source or carries enough text. */
export function keyInputReady(value: KeyInput | undefined): boolean {
  if (!value) return false
  if (value.mode === 'paste') return value.value.trim().length >= 8
  return true
}

/** The input a key chooser starts on: stored, then found, else paste. */
export function defaultKeyInput(detection: KeyDetection | null): KeyInput {
  if (detection?.stored) return { mode: 'stored' }
  const found = preferredSource(detection)
  return found
    ? { mode: 'import', source: found.kind }
    : { mode: 'paste', value: '' }
}

/** Consumers a store call should leave in place: the existing ones plus ours. */
export function mergeConsumers(
  existing: readonly string[] | undefined,
  required: readonly string[],
): string[] {
  return [...new Set([...(existing ?? []), ...required])]
}

export function isMissingFunction(error: unknown): boolean {
  return /function[_ ]not[_ ]found|not[_ ]found|no such function/i.test(
    normalizeErrorMessage(error),
  )
}

/** `null` when the secrets worker is not running. */
export async function detectKeys(
  names: readonly string[],
): Promise<KeyDetection[] | null> {
  const client = await getIiiClient()
  try {
    const result = await client.trigger<{ results?: KeyDetection[] }>(
      'secrets::detect',
      { names },
      { timeoutMs: 15_000 },
    )
    return Array.isArray(result?.results) ? result.results : []
  } catch (error) {
    if (isMissingFunction(error)) return null
    throw error
  }
}

/**
 * One secret's metadata. `undefined` when the secrets worker is not running,
 * `null` when it runs but holds no secret by that name.
 */
export async function getSecret(
  name: string,
): Promise<SecretMeta | null | undefined> {
  const client = await getIiiClient()
  try {
    return await client.trigger<SecretMeta | null>(
      'secrets::get',
      { name },
      { timeoutMs: 10_000 },
    )
  } catch (error) {
    if (isMissingFunction(error)) return undefined
    throw error
  }
}

/**
 * Put a key in the store under `name`, readable by `consumers` (merged with
 * whoever could already read it, so storing for the router never revokes
 * another worker's access). `stored` only widens access to the existing one.
 */
export async function storeKey(
  name: string,
  input: KeyInput,
  consumers: readonly string[],
  description: string,
): Promise<SecretMeta> {
  const client = await getIiiClient()
  const existing = await getSecret(name).catch(() => null)
  const merged = mergeConsumers(existing?.consumers, consumers)
  if (input.mode === 'stored') {
    if (!existing) throw new Error(`${name} is not in the secrets store`)
    if (merged.length === existing.consumers.length) return existing
    return client.trigger<SecretMeta>(
      'secrets::access',
      { name, consumers: merged },
      { timeoutMs: 10_000 },
    )
  }
  return client.trigger<SecretMeta>(
    input.mode === 'import' ? 'secrets::import' : 'secrets::set',
    input.mode === 'import'
      ? { name, source: input.source, consumers: merged, description }
      : { name, value: input.value, consumers: merged, description },
    { timeoutMs: 20_000 },
  )
}
