import type { ConfigFormProps, JsonValue } from '@iii-dev/console-ui'
import { Input, Select, StatusPanel } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import { Field } from '../components'

type JsonObject = { [key: string]: JsonValue }

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? { ...value } : {}
}

function asString(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

export function CronConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const adapter = asObject(value.adapter)
  const adapterName = asString(adapter.name) || 'local'
  const adapterConfig = asObject(adapter.config)

  const setAdapterName = (name: string) => {
    props.onChange({ ...value, adapter: { name } })
  }

  const setRedisUrl = (redisUrl: string) => {
    const config = { ...adapterConfig }
    if (redisUrl === '') delete config.redis_url
    else config.redis_url = redisUrl

    const nextAdapter: JsonObject = { ...adapter, name: adapterName }
    if (Object.keys(config).length > 0) nextAdapter.config = config
    else delete nextAdapter.config
    props.onChange({ ...value, adapter: nextAdapter })
  }

  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const field = props.focusField?.at(-1)
    if (!field || !rootRef.current) return
    const target = rootRef.current.querySelector<HTMLElement>(`[data-field="${field}"]`)
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  return (
    <div className="cron-ui-form" ref={rootRef}>
      <span className="cron-ui-form-caption">custom form · shipped by the cron worker</span>

      <Field label="Lock adapter" hint="Use redis when more than one cron worker can schedule the same jobs.">
        <Select
          value={adapterName}
          data-field="adapter"
          options={[
            { value: 'local', label: 'local · process-local locks' },
            { value: 'redis', label: 'redis · shared distributed locks' },
          ]}
          onChange={setAdapterName}
          aria-label="Lock adapter"
        />
      </Field>

      {adapterName === 'redis' ? (
        <Field label="Redis URL" htmlFor="cron-cfg-redis-url">
          <Input
            id="cron-cfg-redis-url"
            data-field="redis_url"
            value={asString(adapterConfig.redis_url)}
            placeholder="redis://localhost:6379"
            onChange={setRedisUrl}
          />
        </Field>
      ) : null}

      {props.errors && props.errors.size > 0 ? (
        <StatusPanel
          variant="alert"
          headline="This configuration cannot be saved yet"
          detail={[...props.errors.entries()]
            .map(([pointer, message]) => (pointer ? `${pointer}: ${message}` : message))
            .join('\n')}
        />
      ) : null}
    </div>
  )
}
