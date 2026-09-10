import type { JsonValue } from '@iii-dev/console-ui'
import { isObject, type JsonObject } from './value'

const TELEGRAM_TIMEOUT_ALIASES = ['harness_send_timeout_ms', 'approval_timeout_ms', 'state_timeout_ms'] as const

const TELEGRAM_IGNORED_ALIASES = ['thinking_display', 'use_rich', 'edit_throttle_ms'] as const

function firstConfiguredTimeoutAlias(value: JsonObject): JsonValue | undefined {
  for (const alias of TELEGRAM_TIMEOUT_ALIASES) {
    const candidate = value[alias]
    // Rust deserializes these fields as Option<u64>, so null has the same
    // fallback behavior as an absent alias. Invalid non-null values remain
    // visible to schema/server validation instead of being coerced here.
    if (candidate !== undefined && candidate !== null) return candidate
  }
  return undefined
}

/**
 * Convert deprecated telegram-bot keys to the shape emitted by its current
 * Rust config. Only known aliases are removed; opaque fields are retained so
 * a newer worker can round-trip configuration through an older Console UI.
 */
export function normalizeTelegramBotConfiguration(value: JsonValue): JsonValue {
  if (!isObject(value)) return value

  const hasTopLevelAlias =
    TELEGRAM_TIMEOUT_ALIASES.some((alias) => Object.hasOwn(value, alias)) ||
    TELEGRAM_IGNORED_ALIASES.some((alias) => Object.hasOwn(value, alias))

  const updates = value.updates
  const updatesConfig = isObject(updates) ? updates.config : undefined
  const hasWebhookUrlAlias =
    isObject(updates) && updates.name === 'webhook' && isObject(updatesConfig) && Object.hasOwn(updatesConfig, 'url')

  if (!hasTopLevelAlias && !hasWebhookUrlAlias) return value

  const normalized: JsonObject = { ...value }
  if ((normalized.timeout_ms === undefined || normalized.timeout_ms === null) && hasTopLevelAlias) {
    const timeout = firstConfiguredTimeoutAlias(value)
    if (timeout !== undefined) normalized.timeout_ms = timeout
  }

  for (const alias of TELEGRAM_TIMEOUT_ALIASES) delete normalized[alias]
  for (const alias of TELEGRAM_IGNORED_ALIASES) delete normalized[alias]

  if (hasWebhookUrlAlias && isObject(updates) && isObject(updatesConfig)) {
    const config: JsonObject = { ...updatesConfig }
    if (!Object.hasOwn(config, 'base_url')) config.base_url = config.url
    delete config.url
    normalized.updates = { ...updates, config }
  }

  return normalized
}

export function normalizeWorkerConfiguration(configurationId: string, value: JsonValue): JsonValue {
  if (configurationId === 'telegram-bot') {
    return normalizeTelegramBotConfiguration(value)
  }
  return value
}
