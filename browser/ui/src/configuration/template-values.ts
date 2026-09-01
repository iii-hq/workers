import type { JsonValue } from '@iii-dev/console-ui'

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
