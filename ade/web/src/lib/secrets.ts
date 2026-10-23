/**
 * Browser client for the `secrets` worker — the one place the console keeps
 * a credential. Configuration keeps only a reference: `secret://NAME` when
 * the value is encrypted in the secrets worker's vault, `env://NAME` when it
 * is an environment variable the secrets worker reads (the env file its
 * configuration names, `.env` by default,
 * else its own environment). Either way it is resolved only by the workers
 * the secret names as its consumers.
 *
 * A key found on the machine is imported by the secrets worker itself
 * (`secrets::import`), so its value never reaches the browser. A pasted key
 * travels browser → engine → `secrets::set` and is kept nowhere else.
 */

import { getIiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'

export const SECRET_REF_PREFIX = 'secret://'
export const ENV_REF_PREFIX = 'env://'
export const SECRETS_WORKER = 'secrets'

/**
 * Where the secrets worker keeps a value: `vault` (encrypted, `secret://`)
 * or `env` (an environment variable, `env://`).
 */
export type KeyStore = 'vault' | 'env'

/** `secret://ANTHROPIC_API_KEY` */
export function secretRef(name: string): string {
  return `${SECRET_REF_PREFIX}${name}`
}

/** `env://ANTHROPIC_API_KEY` */
export function envRef(name: string): string {
  return `${ENV_REF_PREFIX}${name}`
}

function refName(value: unknown, prefix: string): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  if (trimmed.slice(0, prefix.length).toLowerCase() !== prefix) return null
  const name = trimmed.slice(prefix.length)
  return /^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/.test(name) ? name : null
}

/** The name in a `secret://NAME` value, `null` for anything else. */
export function secretRefName(value: unknown): string | null {
  return refName(value, SECRET_REF_PREFIX)
}

/** The name in an `env://NAME` value, `null` for anything else. */
export function envRefName(value: unknown): string | null {
  return refName(value, ENV_REF_PREFIX)
}

/** `${VAR}` and nothing else: the engine expands it from its environment. */
export function engineVarName(value: unknown): string | null {
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
  /** Absent from secrets workers that only had the vault. */
  store?: KeyStore
  /** Masked value; empty for an environment variable that is not set. */
  hint: string
  /** Vault only. */
  fingerprint?: string
  /** Env store: the env file's path (or the worker's environment) it is read from. */
  location?: string | null
  consumers: string[]
  description?: string | null
  created_at: string
  updated_at: string
  last_resolved_at?: string | null
  last_resolved_by?: string | null
}

/**
 * How a key reaches the secrets worker. `store` (default `vault`) is where
 * it is kept; `env` keeps it as a variable in the project's `.env`.
 */
export type KeyInput =
  | { mode: 'import'; source: KeySourceKind; store?: KeyStore }
  | { mode: 'paste'; value: string; store?: KeyStore }
  /** Already in the vault under its name: only the reference is (re)written. */
  | { mode: 'stored' }
  /** The environment variable as it already is: only shared and referenced. */
  | { mode: 'env' }

export function keyStore(input: KeyInput): KeyStore {
  if (input.mode === 'env') return 'env'
  if (input.mode === 'stored') return 'vault'
  return input.store ?? 'vault'
}

/** The reference configuration gets for a key kept as `input` says. */
export function keyReference(name: string, input: KeyInput): string {
  return keyStore(input) === 'env' ? envRef(name) : secretRef(name)
}

const SOURCE_LABEL: Record<KeySourceKind, string> = {
  login_shell: 'your shell profile',
  dotenv: "this project's .env",
  process_env: 'the secrets worker environment',
}

export const DEFAULT_ENV_FILE = '.env'

/** The env file's name from its path (`/p/.env.staging` → `.env.staging`). */
export function envFileName(path: string | null | undefined): string {
  const name = path?.split(/[\\/]/).pop()?.trim()
  return name || DEFAULT_ENV_FILE
}

export function sourceLabel(source: KeySource | KeySourceKind): string {
  if (
    typeof source !== 'string' &&
    source.kind === 'dotenv' &&
    source.location
  ) {
    // The env file the secrets worker is configured with, by name.
    return `this project's ${envFileName(source.location)}`
  }
  const kind = typeof source === 'string' ? source : source.kind
  return (
    SOURCE_LABEL[kind] ??
    (typeof source === 'string' ? source : source.location)
  )
}

/**
 * Where the env store would read the variable now: this project's `.env`,
 * else the secrets worker's own environment. The login shell is not one of
 * them — a key found only there has to be copied into `.env`.
 */
export function envSource(
  detection: KeyDetection | null | undefined,
): KeySource | null {
  return (
    detection?.sources.find((source) => source.kind === 'dotenv') ??
    detection?.sources.find((source) => source.kind === 'process_env') ??
    null
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

/**
 * The input a key chooser starts on. In the vault: the stored key, then one
 * found on the machine, else paste. In the env store: the variable as it is,
 * then a copy of the one in the shell profile, else paste into `.env`.
 */
export function defaultKeyInput(
  detection: KeyDetection | null,
  store: KeyStore = 'vault',
): KeyInput {
  if (store === 'env') {
    if (envSource(detection)) return { mode: 'env' }
    const shell = detection?.sources.find((s) => s.kind === 'login_shell')
    return shell
      ? { mode: 'import', source: shell.kind, store: 'env' }
      : { mode: 'paste', value: '', store: 'env' }
  }
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

/** What `secrets::status` reports; never key material. */
export interface SecretsStatus {
  vault_path: string
  key_source: 'env' | 'file'
  key_path?: string
  count: number
  version: number
  /** The env file `env://` references read (from the worker's configuration). */
  env_file?: string
  env_count?: number
}

/** `undefined` when the secrets worker is not running. */
export async function getSecretsStatus(): Promise<SecretsStatus | undefined> {
  const client = await getIiiClient()
  try {
    return await client.trigger<SecretsStatus>(
      'secrets::status',
      {},
      { timeoutMs: 10_000 },
    )
  } catch (error) {
    if (isMissingFunction(error)) return undefined
    throw error
  }
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
 * `null` when it runs but holds no secret by that name (for `env`: the
 * variable is not shared).
 */
export async function getSecret(
  name: string,
  store: KeyStore = 'vault',
): Promise<SecretMeta | null | undefined> {
  const client = await getIiiClient()
  try {
    return await client.trigger<SecretMeta | null>(
      'secrets::get',
      store === 'env' ? { name, store } : { name },
      { timeoutMs: 10_000 },
    )
  } catch (error) {
    if (isMissingFunction(error)) return undefined
    throw error
  }
}

/**
 * Keep a key under `name` where `input` says, readable by `consumers`
 * (merged with whoever could already read it, so storing for the router
 * never revokes another worker's access). `stored` only widens access to the
 * vault's key; `env` only shares the variable as it is.
 */
export async function storeKey(
  name: string,
  input: KeyInput,
  consumers: readonly string[],
  description: string,
): Promise<SecretMeta> {
  const client = await getIiiClient()
  const store = keyStore(input)
  const existing = await getSecret(name, store).catch(() => null)
  const merged = mergeConsumers(existing?.consumers, consumers)
  if (input.mode === 'env' || store === 'env') {
    if (input.mode === 'env') {
      return client.trigger<SecretMeta>(
        'secrets::access',
        { name, consumers: merged, store },
        { timeoutMs: 10_000 },
      )
    }
    return client.trigger<SecretMeta>(
      input.mode === 'import' ? 'secrets::import' : 'secrets::set',
      input.mode === 'import'
        ? { name, source: input.source, consumers: merged, description, store }
        : {
            name,
            value: input.mode === 'paste' ? input.value : '',
            consumers: merged,
            description,
            store,
          },
      { timeoutMs: 20_000 },
    )
  }
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
