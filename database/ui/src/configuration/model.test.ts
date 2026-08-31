// biome-ignore-all lint/suspicious/noTemplateCurlyInString: these literals intentionally exercise ${VAR} configuration templates.
import { describe, expect, it } from 'vitest'
import {
  booleanLiteralForRawValue,
  DATABASE_CONFIG_FIELD_PATHS,
  databaseFocusRequest,
  driverOf,
  driverOfConfiguredUrl,
  isEnvironmentValue,
  isRawTypedValue,
  numberLiteralForRawValue,
  selectLiteralForRawValue,
  shouldMaskConfiguredUrl,
  shouldShowTlsForUrl,
} from './model'

describe('database configuration model', () => {
  it('tracks every persisted path from the Rust schema', () => {
    expect(DATABASE_CONFIG_FIELD_PATHS).toEqual([
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
    ])
    expect(DATABASE_CONFIG_FIELD_PATHS).not.toContain('databases.{name}.driver')
  })

  it('mirrors the worker driver inference, including case-insensitive schemes', () => {
    expect(driverOf('postgres://localhost/app')).toBe('postgres')
    expect(driverOf('PostgreSQL://localhost/app')).toBe('postgres')
    expect(driverOf('MYSQL://localhost/app')).toBe('mysql')
    expect(driverOf('SQLite:./data/app.db')).toBe('sqlite')
    expect(driverOf('https://example.com')).toBe('unknown')
  })

  it('keeps URL templates visible while deriving driver context and TLS availability', () => {
    expect(isEnvironmentValue('${DATABASE_URL}')).toBe(true)
    expect(driverOfConfiguredUrl('${DATABASE_URL:postgres://db/app}')).toBe('postgres')
    expect(driverOfConfiguredUrl('${DATABASE_URL:mysql://db/app}')).toBe('mysql')
    expect(driverOfConfiguredUrl('${DATABASE_URL}')).toBe('unknown')
    expect(shouldShowTlsForUrl('${DATABASE_URL}')).toBe(true)
    expect(shouldMaskConfiguredUrl('${DATABASE_URL}')).toBe(false)
    expect(shouldMaskConfiguredUrl('postgres://user:secret@db/app')).toBe(true)
    expect(shouldMaskConfiguredUrl('postgres://user:secret@db/app?application_name=${APP_NAME}')).toBe(true)
    expect(shouldMaskConfiguredUrl('sqlite:${DATABASE_PATH}')).toBe(false)
    expect(shouldShowTlsForUrl('sqlite:./data/app.db')).toBe(false)
    expect(shouldShowTlsForUrl('sqlite:${DATABASE_PATH}')).toBe(false)
    expect(shouldShowTlsForUrl('${DATABASE_URL:sqlite:./data/app.db}')).toBe(true)
  })

  it('creates a stable focus request per field path and database-name set', () => {
    const first = databaseFocusRequest(['primary', 'analytics'], ['databases', 'analytics', 'url'])
    const same = databaseFocusRequest(['primary', 'analytics'], ['databases', 'analytics', 'url'])
    const renamed = databaseFocusRequest(['primary', 'warehouse'], ['databases', 'analytics', 'url'])

    expect(first).toEqual({
      key: first?.key,
      exactField: 'databases.analytics.url',
      databaseIndex: 1,
      databaseName: 'analytics',
    })
    expect(same?.key).toBe(first?.key)
    expect(renamed?.key).not.toBe(first?.key)
    expect(databaseFocusRequest(['primary'], undefined)).toBeNull()
  })

  it('preserves typed raw values and derives explicit number, boolean, and select replacements', () => {
    expect(isRawTypedValue('${POOL_MAX:24}')).toBe(true)
    expect(numberLiteralForRawValue('${POOL_MAX:24}', 10)).toBe(24)
    expect(numberLiteralForRawValue('${POOL_MAX}', 10)).toBe(10)
    expect(booleanLiteralForRawValue('${TRUST_NATIVE:false}', true)).toBe(false)
    expect(selectLiteralForRawValue('${TLS_MODE:verify-full}', ['disable', 'require', 'verify-full'], 'require')).toBe(
      'verify-full',
    )
  })
})
