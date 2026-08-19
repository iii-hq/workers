import { useEffect, useRef } from 'react'
import type { ConfigFormProps, JsonValue } from '@iii-dev/console-ui'

type JsonObject = { [key: string]: JsonValue }

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? { ...value }
    : {}
}

function asString(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

export function CronConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const adapter = asObject(value.adapter)
  const adapterName = asString(adapter.name) || 'local'
  const adapterConfig = asObject(adapter.config)

  const commit = (next: JsonObject) => props.onChange(next)

  const setAdapterName = (name: string) => {
    commit({ ...value, adapter: { name } })
  }

  const setRedisUrl = (redisUrl: string) => {
    const config = asObject(adapter.config)
    if (redisUrl === '') delete config.redis_url
    else config.redis_url = redisUrl

    const nextAdapter: JsonObject = { ...adapter, name: adapterName }
    if (Object.keys(config).length > 0) nextAdapter.config = config
    else delete nextAdapter.config
    commit({ ...value, adapter: nextAdapter })
  }

  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const field = props.focusField?.at(-1)
    if (!field || !rootRef.current) return
    const target = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${field}"]`,
    )
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  return (
    <div className="cron-ui-form" ref={rootRef}>
      <span className="cron-ui-form-caption">
        custom form · shipped by the cron worker
      </span>

      <div className="cron-ui-field">
        <label htmlFor="cron-cfg-adapter">lock adapter</label>
        <select
          id="cron-cfg-adapter"
          data-field="adapter"
          className="cron-ui-select"
          value={adapterName}
          onChange={(event) => setAdapterName(event.target.value)}
        >
          <option value="local">local: process-local locks</option>
          <option value="redis">redis: shared distributed locks</option>
        </select>
        <span className="cron-ui-hint">
          use redis when more than one cron worker can schedule the same jobs
        </span>
      </div>

      {adapterName === 'redis' ? (
        <div className="cron-ui-field">
          <label htmlFor="cron-cfg-redis-url">redis url</label>
          <input
            id="cron-cfg-redis-url"
            data-field="redis_url"
            className="cron-ui-input"
            type="text"
            value={asString(adapterConfig.redis_url)}
            placeholder="redis://localhost:6379"
            onChange={(event) => setRedisUrl(event.target.value)}
          />
        </div>
      ) : null}

      {props.errors && props.errors.size > 0 ? (
        <div className="cron-ui-error">
          {[...props.errors.entries()].map(([pointer, message]) => (
            <div key={pointer}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
