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

// Mirrors github/src/webhooks (WebhookConfig / NotificationPolicy ::default).
const GITHUB_WEBHOOK_DEFAULTS: JsonObject = {
  enabled: false,
  storage_path: './data/github-webhooks/store.sqlite3',
  tunnel_id: 'webhooks',
  queue: 'github-webhooks',
  max_body_bytes: 1048576,
  max_pending: 10000,
  max_watch_days: 30,
  orphan_grace_minutes: 60,
}
const GITHUB_NOTIFICATION_DEFAULTS: JsonObject = {
  profile: 'all',
  ignore_self: false,
  quiet_ms: 15000,
  max_wait_ms: 120000,
  max_items: 30,
  max_comment_chars: 2000,
  ignored_actors: [],
  suppress_bot_noise: true,
  batch_window_ms: 10000,
  success_checks: [],
  notify_ci_failures: true,
  notify_ci_success: true,
  notify_resolved_threads: false,
}

function withMissing(value: JsonObject, defaults: JsonObject): JsonObject {
  const missing = Object.keys(defaults).filter((key) => !Object.hasOwn(value, key))
  if (missing.length === 0) return value
  const next: JsonObject = { ...value }
  for (const key of missing) next[key] = structuredClone(defaults[key])
  return next
}

/**
 * The github worker fills absent webhook/notification keys with serde
 * defaults, so an unset field is not "empty" at runtime. Materialize those
 * effective values for display; stored and unknown keys are never changed.
 */
export function normalizeGithubConfiguration(value: JsonValue): JsonValue {
  if (!isObject(value)) return value
  const webhooks = isObject(value.webhooks) ? value.webhooks : {}
  const notifications = isObject(webhooks.notifications) ? webhooks.notifications : {}
  const nextNotifications = withMissing(notifications, GITHUB_NOTIFICATION_DEFAULTS)
  const nextWebhooks = withMissing(webhooks, GITHUB_WEBHOOK_DEFAULTS)
  if (nextWebhooks === value.webhooks && nextNotifications === webhooks.notifications) return value
  return { ...value, webhooks: { ...nextWebhooks, notifications: nextNotifications } }
}

export function normalizeWorkerConfiguration(configurationId: string, value: JsonValue): JsonValue {
  if (configurationId === 'telegram-bot') {
    return normalizeTelegramBotConfiguration(value)
  }
  if (configurationId === 'github') {
    return normalizeGithubConfiguration(value)
  }
  return value
}
