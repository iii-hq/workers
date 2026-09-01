import type { JsonValue } from '@iii-dev/console-ui'

export type Driver = 'postgres' | 'mysql' | 'sqlite' | 'unknown'

export const DEFAULT_HISTORY_MAX_ENTRIES = 200
export const DEFAULT_HISTORY_MAX_BYTES = 262_144

const ENVIRONMENT_VALUE_PATTERN = /^\$\{([^}:]+)(?::([^}]*))?\}$/
const NUMBER_PATTERN = /^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/

export function isEnvironmentValue(value: string): boolean {
  return ENVIRONMENT_VALUE_PATTERN.test(value)
}

export function environmentValueDefault(value: string): string | undefined {
  return ENVIRONMENT_VALUE_PATTERN.exec(value)?.[2]
}

export function isRawTypedValue(value: JsonValue | undefined): value is string {
  return typeof value === 'string'
}

export function numberLiteralForRawValue(value: string, fallback: number): number {
  const candidate = environmentValueDefault(value)
  if (candidate !== undefined && NUMBER_PATTERN.test(candidate)) {
    const parsed = Number(candidate)
    if (Number.isFinite(parsed)) return parsed
  }
  return fallback
}

export function booleanLiteralForRawValue(value: string, fallback: boolean): boolean {
  const candidate = environmentValueDefault(value)
  if (candidate === 'true' || candidate === 'True' || candidate === 'TRUE') {
    return true
  }
  if (candidate === 'false' || candidate === 'False' || candidate === 'FALSE') {
    return false
  }
  return fallback
}

export function selectLiteralForRawValue(value: string, options: readonly string[], fallback: string): string {
  const candidate = environmentValueDefault(value)
  return candidate !== undefined && options.includes(candidate) ? candidate : fallback
}

export interface DatabaseFocusRequest {
  key: string
  exactField: string
  databaseIndex: number
  databaseName?: string
}

/** Stable request identity and selection target for one host-provided deep link. */
export function databaseFocusRequest(
  names: readonly string[],
  focusField: readonly string[] | undefined,
): DatabaseFocusRequest | null {
  if (!focusField || focusField.length === 0) return null
  const path = focusField.map(String)
  const databaseName = path[0] === 'databases' ? path[1] : undefined
  return {
    key: JSON.stringify(path),
    exactField: path.join('.'),
    databaseIndex: databaseName === undefined ? -1 : names.indexOf(databaseName),
    databaseName,
  }
}

/** Validate an atomic map-key rename without mutating the configuration. */
export function databaseHandleError(current: string, names: readonly string[], draft: string): string | undefined {
  const next = draft.trim()
  if (!next) return 'Enter a database handle.'
  if (next !== current && names.includes(next)) {
    return `A database named “${next}” already exists.`
  }
  return undefined
}

/** Persisted paths in DatabaseConfig, with `{name}` for the map key. */
export const DATABASE_CONFIG_FIELD_PATHS = [
  'databases.{name}.url',
  'databases.{name}.capture',
  'databases.{name}.tls.mode',
  'databases.{name}.tls.ca_cert',
  'databases.{name}.tls.trust_native',
  'databases.{name}.pool.max',
  'databases.{name}.pool.idle_timeout_ms',
  'databases.{name}.pool.acquire_timeout_ms',
  'history_max_entries',
  'history_max_bytes',
] as const

/** Match the Rust worker's case-insensitive URL-scheme detection. */
export function driverOf(url: string): Driver {
  const normalized = url.toLowerCase()
  if (normalized.startsWith('postgres://') || normalized.startsWith('postgresql://')) {
    return 'postgres'
  }
  if (normalized.startsWith('mysql://')) return 'mysql'
  if (normalized.startsWith('sqlite:')) return 'sqlite'
  return 'unknown'
}

/** Infer from a lone template's default without replacing the stored value. */
export function driverOfConfiguredUrl(url: string): Driver {
  const templateDefault = environmentValueDefault(url)
  return driverOf(templateDefault ?? url)
}

/** TLS remains editable even when a defaultless environment URL hides its driver. */
export function shouldShowTlsForUrl(url: string): boolean {
  // A whole-value template may resolve to a different driver than its default.
  // Keep TLS editable even when that default happens to be SQLite. A fixed
  // SQLite scheme with only its path templated remains unambiguously local.
  if (isEnvironmentValue(url)) return true
  const driver = driverOfConfiguredUrl(url)
  if (driver === 'sqlite') return false
  return driver === 'postgres' || driver === 'mysql' || url.includes('${')
}

/** Keep whole-value expressions readable; mask every other non-SQLite URL. */
export function shouldMaskConfiguredUrl(url: string): boolean {
  if (isEnvironmentValue(url)) return false
  return driverOfConfiguredUrl(url) !== 'sqlite'
}
