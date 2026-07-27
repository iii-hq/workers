/**
 * Custom configuration form for the `state` configuration entry —
 * registered through `host.configForms`, replacing the console's generic
 * schema-driven form for this worker only.
 *
 * The form edits the working draft via `onChange`; dirty tracking,
 * save/reset, and error mapping stay host-owned (the console's SaveBar
 * below the form drives `configuration::set`). Mirrors StateConfig
 * (workers/state/src/config.rs): adapter {name, config}, triggers_enabled,
 * max_value_bytes, save_interval_ms.
 */

import { useEffect, useRef } from 'react'
import type { ConfigFormProps, JsonValue } from '@iii-dev/console-ui'

type JsonObject = { [key: string]: JsonValue }

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

export function StateConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const adapter = asObject(value.adapter)
  const adapterName = asString(adapter.name) || 'kv'
  const adapterConfig = asObject(adapter.config)
  const storeMethod = asString(adapterConfig.store_method)
  const triggersEnabled = value.triggers_enabled !== false

  const commit = (next: JsonObject) => props.onChange(next)

  /** Update `adapter.config`, dropping the config blob when it empties. */
  const setAdapterConfig = (mutate: (cfg: JsonObject) => void) => {
    const cfg = asObject(adapter.config)
    mutate(cfg)
    const nextAdapter: JsonObject = { ...adapter, name: adapterName }
    if (Object.keys(cfg).length > 0) nextAdapter.config = cfg
    else delete nextAdapter.config
    commit({ ...value, adapter: nextAdapter })
  }

  const setAdapterName = (name: string) => {
    // Adapter families have disjoint config keys — switching families
    // starts from a clean config so stale keys don't fail validation.
    commit({ ...value, adapter: { name } })
  }

  const setOptionalNumber = (field: string, raw: string) => {
    const next = { ...value }
    if (raw.trim() === '') delete next[field]
    else if (!Number.isNaN(Number(raw))) next[field] = Number(raw)
    commit(next)
  }

  // Deep-link focus (`#/workers/configuration/state/<field>`): the host's
  // own scroll+focus targets schema-form DOM ids, so a custom form honors
  // `focusField` itself.
  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const target = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${field}"]`,
    )
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  return (
    <div className="state-ui-form" ref={rootRef}>
      <span className="state-ui-form-caption">
        custom form · shipped by the state worker
      </span>

      <div className="state-ui-field">
        <label htmlFor="state-cfg-adapter">storage adapter</label>
        <select
          id="state-cfg-adapter"
          data-field="adapter"
          className="state-ui-select"
          value={adapterName}
          onChange={(e) => setAdapterName(e.target.value)}
        >
          <option value="kv">kv — in-process key-value store</option>
          <option value="redis">redis — shared external store</option>
        </select>
      </div>

      {adapterName === 'kv' ? (
        <>
          <div className="state-ui-field">
            <label htmlFor="state-cfg-store-method">persistence</label>
            <select
              id="state-cfg-store-method"
              data-field="store_method"
              className="state-ui-select"
              value={storeMethod === 'file_based' ? 'file_based' : 'memory'}
              onChange={(e) =>
                setAdapterConfig((cfg) => {
                  if (e.target.value === 'file_based') {
                    cfg.store_method = 'file_based'
                  } else {
                    delete cfg.store_method
                    delete cfg.file_path
                  }
                })
              }
            >
              <option value="memory">in-memory (lost on restart)</option>
              <option value="file_based">file_based (persisted to disk)</option>
            </select>
          </div>
          {storeMethod === 'file_based' ? (
            <div className="state-ui-field">
              <label htmlFor="state-cfg-file-path">file path</label>
              <input
                id="state-cfg-file-path"
                data-field="file_path"
                className="state-ui-input"
                type="text"
                value={asString(adapterConfig.file_path)}
                placeholder="./data/state_store.db"
                onChange={(e) =>
                  setAdapterConfig((cfg) => {
                    if (e.target.value === '') delete cfg.file_path
                    else cfg.file_path = e.target.value
                  })
                }
              />
              <span className="hint">
                same on-disk format as the engine builtin — point at its old
                file to keep existing data
              </span>
            </div>
          ) : null}
        </>
      ) : (
        <div className="state-ui-field">
          <label htmlFor="state-cfg-redis-url">redis url</label>
          <input
            id="state-cfg-redis-url"
            data-field="redis_url"
            className="state-ui-input"
            type="text"
            value={asString(adapterConfig.redis_url)}
            placeholder="redis://localhost:6379"
            onChange={(e) =>
              setAdapterConfig((cfg) => {
                if (e.target.value === '') delete cfg.redis_url
                else cfg.redis_url = e.target.value
              })
            }
          />
        </div>
      )}

      <div className="state-ui-field">
        <span className="state-ui-checkrow">
          <input
            id="state-cfg-triggers"
            data-field="triggers_enabled"
            type="checkbox"
            checked={triggersEnabled}
            onChange={(e) =>
              commit({ ...value, triggers_enabled: e.target.checked })
            }
          />
          <label htmlFor="state-cfg-triggers">
            fire `state` change triggers on writes
          </label>
        </span>
        <span className="hint">
          the console's live state page (and any worker subscriptions) stop
          updating while this is off
        </span>
      </div>

      <div className="state-ui-field">
        <label htmlFor="state-cfg-max-bytes">
          max value size (bytes, empty = unlimited)
        </label>
        <input
          id="state-cfg-max-bytes"
          data-field="max_value_bytes"
          className="state-ui-input"
          type="number"
          min={1}
          value={typeof value.max_value_bytes === 'number' ? value.max_value_bytes : ''}
          onChange={(e) => setOptionalNumber('max_value_bytes', e.target.value)}
        />
      </div>

      <div className="state-ui-field">
        <label htmlFor="state-cfg-save-interval">
          save interval (ms, kv file_based only; empty = default)
        </label>
        <input
          id="state-cfg-save-interval"
          data-field="save_interval_ms"
          className="state-ui-input"
          type="number"
          min={1}
          value={typeof value.save_interval_ms === 'number' ? value.save_interval_ms : ''}
          onChange={(e) => setOptionalNumber('save_interval_ms', e.target.value)}
        />
      </div>

      {props.errors && props.errors.size > 0 ? (
        <div className="state-ui-error">
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
