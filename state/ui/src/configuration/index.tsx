/**
 * Deliberate configuration form for the `state` entry, registered through
 * `host.configForms` and rendered inside global Settings.
 *
 * The form edits the working draft via `onChange`; dirty tracking,
 * save/reset, and error mapping stay host-owned (the console's SaveBar
 * below the form drives `configuration::set`). Mirrors StateConfig
 * (workers/state/src/config.rs): adapter {name, config}, triggers_enabled,
 * max_value_bytes, save_interval_ms.
 */

import {
  Chip,
  type ConfigFormProps,
  Input,
  type JsonValue,
  Select,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import { type JsonObject, persistenceModeFor, withPersistenceMode } from './model'

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? { ...value } : {}
}

function asString(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

function describedBy(...ids: Array<string | undefined>): string | undefined {
  const value = ids.filter(Boolean).join(' ')
  return value || undefined
}

function configurationErrorId(pointer: string): string {
  const suffix = pointer.replace(/[^a-zA-Z0-9_-]+/g, '-') || 'root'
  return `state-cfg-error-${suffix}`
}

function errorIdFor(errors: ConfigFormProps['errors'], pointer: string): string | undefined {
  return errors?.has(pointer) ? configurationErrorId(pointer) : undefined
}

function controlA11y(errors: ConfigFormProps['errors'], pointer: string, descriptionId: string) {
  const errorId = errorIdFor(errors, pointer)
  return {
    'aria-invalid': errorId ? true : undefined,
    'aria-describedby': describedBy(descriptionId, errorId),
  }
}

export function StateConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const adapter = asObject(value.adapter)
  const adapterName = asString(adapter.name) || 'kv'
  const adapterConfig = asObject(adapter.config)
  const storeMethod = asString(adapterConfig.store_method)
  const persistenceMode = persistenceModeFor(storeMethod)
  const triggersEnabled = value.triggers_enabled !== false

  const commit = (next: JsonObject) => props.onChange(next)

  /** Update `adapter.config`, dropping the config blob when it empties. */
  const commitAdapterConfig = (config: JsonObject) => {
    const nextAdapter: JsonObject = { ...adapter, name: adapterName }
    if (Object.keys(config).length > 0) nextAdapter.config = config
    else delete nextAdapter.config
    commit({ ...value, adapter: nextAdapter })
  }

  const setAdapterConfig = (mutate: (config: JsonObject) => void) => {
    const config = asObject(adapter.config)
    mutate(config)
    commitAdapterConfig(config)
  }

  const setAdapterName = (name: string) => {
    if (name === adapterName) return
    // Adapter families have disjoint config keys. Switching families starts
    // with a clean config so stale keys cannot fail validation.
    commit({ ...value, adapter: { name } })
  }

  const setOptionalNumber = (field: string, raw: string) => {
    const next = { ...value }
    if (raw.trim() === '') delete next[field]
    else if (!Number.isNaN(Number(raw))) next[field] = Number(raw)
    commit(next)
  }

  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const path = props.focusField
    if (!path?.length || !rootRef.current) return
    const exact = rootRef.current.querySelector<HTMLElement>(`[data-path="${CSS.escape(path.join('.'))}"]`)
    const leaf = rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(path[path.length - 1])}"]`)
    const topLevel = rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(path[0])}"]`)
    const target = exact ?? leaf ?? topLevel
    target?.scrollIntoView({ block: 'center' })
    const focusable = target?.matches('input, button, [tabindex]')
      ? target
      : target?.querySelector<HTMLElement>('input, button, [tabindex]')
    focusable?.focus()
  }, [props.focusField])

  return (
    <div className="state-ui-form" ref={rootRef}>
      <SettingsSection
        title="Storage adapter"
        description="Choose where state is stored and configure the selected adapter."
      >
        <SettingsList>
          <SettingsRow
            data-field="adapter"
            data-path="adapter"
            label="Adapter"
            description={
              <span id="state-cfg-adapter-description">
                Use the local key-value store or connect to a shared Redis instance.
              </span>
            }
            meta={<Chip tone="warning">Restart required</Chip>}
            control={
              <div className="state-ui-config-select">
                <Select
                  aria-label="Storage adapter"
                  {...controlA11y(props.errors, '/adapter', 'state-cfg-adapter-description')}
                  value={adapterName}
                  options={[
                    { value: 'kv', label: 'KV · In-process store' },
                    { value: 'redis', label: 'Redis · Shared store' },
                  ]}
                  onChange={setAdapterName}
                />
              </div>
            }
          />

          {adapterName === 'kv' ? (
            <>
              <SettingsRow
                data-field="store_method"
                data-path="adapter.config.store_method"
                label="Persistence"
                description={
                  <span id="state-cfg-store-method-description">
                    Keep values in memory or persist them to a local file.
                  </span>
                }
                meta={<Chip tone="warning">Restart required</Chip>}
                control={
                  <div className="state-ui-config-select">
                    <Select
                      aria-label="Persistence"
                      {...controlA11y(
                        props.errors,
                        '/adapter/config/store_method',
                        'state-cfg-store-method-description',
                      )}
                      value={persistenceMode}
                      options={[
                        { value: 'file_based', label: 'File · Persisted to disk' },
                        { value: 'in_memory', label: 'Memory · Lost on restart' },
                      ]}
                      onChange={(next) => {
                        if (next === persistenceMode) return
                        commitAdapterConfig(withPersistenceMode(adapterConfig, next))
                      }}
                    />
                  </div>
                }
              />
              {persistenceMode === 'file_based' ? (
                <SettingsRow
                  data-field="file_path"
                  data-path="adapter.config.file_path"
                  label={<label htmlFor="state-cfg-file-path">File path</label>}
                  description={
                    <span id="state-cfg-file-path-description">
                      Uses the engine built-in format. Point to an existing state file to keep its data.
                    </span>
                  }
                  meta={<Chip tone="warning">Restart required</Chip>}
                  control={
                    <Input
                      id="state-cfg-file-path"
                      className="state-ui-config-control"
                      type="text"
                      value={asString(adapterConfig.file_path)}
                      placeholder="./data/state_store.db"
                      spellCheck={false}
                      autoComplete="off"
                      aria-label="File path"
                      {...controlA11y(props.errors, '/adapter/config/file_path', 'state-cfg-file-path-description')}
                      onChange={(next) =>
                        setAdapterConfig((config) => {
                          if (next === '') delete config.file_path
                          else config.file_path = next
                        })
                      }
                    />
                  }
                />
              ) : null}
            </>
          ) : (
            <SettingsRow
              data-field="redis_url"
              data-path="adapter.config.redis_url"
              label={<label htmlFor="state-cfg-redis-url">Redis URL</label>}
              description={
                <span id="state-cfg-redis-url-description">Connection URL for the shared Redis instance.</span>
              }
              meta={<Chip tone="warning">Restart required</Chip>}
              control={
                <Input
                  id="state-cfg-redis-url"
                  className="state-ui-config-control"
                  type="text"
                  value={asString(adapterConfig.redis_url)}
                  placeholder="redis://localhost:6379"
                  spellCheck={false}
                  autoComplete="off"
                  aria-label="Redis URL"
                  {...controlA11y(props.errors, '/adapter/config/redis_url', 'state-cfg-redis-url-description')}
                  onChange={(next) =>
                    setAdapterConfig((config) => {
                      if (next === '') delete config.redis_url
                      else config.redis_url = next
                    })
                  }
                />
              }
            />
          )}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Runtime behavior"
        description="Set write notifications, payload limits, and file persistence cadence."
      >
        <SettingsList>
          <SettingsRow
            data-field="triggers_enabled"
            data-path="triggers_enabled"
            label={<label htmlFor="state-cfg-triggers">State change triggers</label>}
            description={
              <span id="state-cfg-triggers-description">
                Notify the live State page and worker subscriptions after writes.
              </span>
            }
            layout="inline"
            control={
              <Switch
                id="state-cfg-triggers"
                checked={triggersEnabled}
                aria-label="State change triggers"
                {...controlA11y(props.errors, '/triggers_enabled', 'state-cfg-triggers-description')}
                onChange={(event) => commit({ ...value, triggers_enabled: event.currentTarget.checked })}
              />
            }
          />
          <SettingsRow
            data-field="max_value_bytes"
            data-path="max_value_bytes"
            label={<label htmlFor="state-cfg-max-bytes">Maximum value size</label>}
            description={
              <span id="state-cfg-max-bytes-description">
                Reject values larger than this number of bytes. Leave empty for no limit.
              </span>
            }
            control={
              <Input
                id="state-cfg-max-bytes"
                className="state-ui-config-control state-ui-config-number"
                type="number"
                inputMode="numeric"
                min={1}
                value={typeof value.max_value_bytes === 'number' ? String(value.max_value_bytes) : ''}
                aria-label="Maximum value size in bytes"
                {...controlA11y(props.errors, '/max_value_bytes', 'state-cfg-max-bytes-description')}
                onChange={(next) => setOptionalNumber('max_value_bytes', next)}
              />
            }
          />
          <SettingsRow
            data-field="save_interval_ms"
            data-path="save_interval_ms"
            label={<label htmlFor="state-cfg-save-interval">Save interval</label>}
            description={
              <span id="state-cfg-save-interval-description">
                Flush cadence in milliseconds for the file-backed KV adapter. Leave empty to use the default.
              </span>
            }
            control={
              <Input
                id="state-cfg-save-interval"
                className="state-ui-config-control state-ui-config-number"
                type="number"
                inputMode="numeric"
                min={100}
                max={3_600_000}
                value={typeof value.save_interval_ms === 'number' ? String(value.save_interval_ms) : ''}
                aria-label="Save interval in milliseconds"
                {...controlA11y(props.errors, '/save_interval_ms', 'state-cfg-save-interval-description')}
                onChange={(next) => setOptionalNumber('save_interval_ms', next)}
              />
            }
          />
        </SettingsList>
      </SettingsSection>

      {props.errors && props.errors.size > 0 ? (
        <div className="state-ui-error" role="alert">
          {[...props.errors.entries()].map(([pointer, message]) => (
            <div id={configurationErrorId(pointer)} key={pointer || message}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
