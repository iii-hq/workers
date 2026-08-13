type JsonObject = Record<string, unknown>

function object(value: unknown): JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as JsonObject) : {}
}

export function validateRouterConfig(value: unknown, providerIds: readonly string[]): Map<string, string> {
  const errors = new Map<string, string>()
  const root = object(value)
  const settings = object(root.settings)
  const positive = ['stream_timeout_ms', 'idle_timeout_ms', 'output_token_max'] as const
  for (const key of positive) {
    const current = settings[key]
    if (current !== undefined && (typeof current !== 'number' || !Number.isInteger(current) || current <= 0)) {
      errors.set(`/settings/${key}`, 'must be a positive integer')
    }
  }
  const retry = settings.retry_max
  if (retry !== undefined && (typeof retry !== 'number' || !Number.isInteger(retry) || retry < 0 || retry > 10)) {
    errors.set('/settings/retry_max', 'must be an integer between 0 and 10')
  }
  if (
    typeof settings.idle_timeout_ms === 'number' &&
    typeof settings.stream_timeout_ms === 'number' &&
    settings.idle_timeout_ms > settings.stream_timeout_ms
  ) {
    errors.set('/settings/idle_timeout_ms', 'must not exceed stream timeout')
  }

  if (typeof root.default_provider === 'string' && !providerIds.includes(root.default_provider)) {
    errors.set('/default_provider', 'references a provider that is not connected')
  }

  const heuristics = Array.isArray(root.routing_heuristics) ? root.routing_heuristics : []
  heuristics.forEach((raw, index) => {
    const row = object(raw)
    if (typeof row.pattern === 'string') {
      try {
        new RegExp(row.pattern)
      } catch {
        errors.set(`/routing_heuristics/${index}/pattern`, 'must be a valid regular expression')
      }
    }
    if (typeof row.provider === 'string' && !providerIds.includes(row.provider)) {
      errors.set(`/routing_heuristics/${index}/provider`, 'references a provider that is not connected')
    }
  })

  const providers = object(root.providers)
  for (const [id, raw] of Object.entries(providers)) {
    const slice = object(raw)
    if (slice.api_url !== undefined) {
      let valid = false
      if (typeof slice.api_url === 'string') {
        try {
          const parsed = new URL(slice.api_url)
          valid = parsed.protocol === 'http:' || parsed.protocol === 'https:'
        } catch {
          valid = false
        }
      }
      if (!valid) {
        errors.set(`/providers/${id}/api_url`, 'must be an absolute http(s) URL')
      }
    }
    if (
      slice.max_tokens !== undefined &&
      (typeof slice.max_tokens !== 'number' || !Number.isInteger(slice.max_tokens) || slice.max_tokens <= 0)
    ) {
      errors.set(`/providers/${id}/max_tokens`, 'must be a positive integer')
    }
  }
  return errors
}
