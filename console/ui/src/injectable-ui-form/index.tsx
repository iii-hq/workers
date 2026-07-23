/**
 * Custom configuration form for the `console` configuration entry — the
 * injectable-UI toggle board.
 *
 * One bordered card per worker that ships injected console UI (or is
 * currently toggled off), showing the worker's title and description with a
 * switch on the right: the card reads active while its UI is enabled and
 * dims when disabled. Toggles edit `injectableUi.disabledWorkers` in the
 * entry's draft via `onChange`; dirty tracking and the actual save stay
 * host-owned (SaveBar → `configuration::set`), and the console worker
 * applies a saved change live — tabs drop or load the affected workers'
 * assets without any restart.
 *
 * Worker identity comes from `console::ui-manifest` (`workers` summary);
 * titles and descriptions come from `configuration::list`, matched by the
 * convention that a worker's configuration id is its name. Workers without
 * a configuration entry fall back to their name.
 *
 * The console's own UI is deliberately absent from the board — the worker
 * refuses to disable it (it ships this very form), so offering the switch
 * would be a lie.
 */

import { useEffect, useMemo, useRef, useState } from 'react'
import type { ConfigFormProps, Host, JsonValue } from '@iii-dev/console-ui'

type JsonObject = { [key: string]: JsonValue }

interface ManifestWorker {
  worker: string
  enabled: boolean
  assets: number
}

interface UiManifest {
  disabled: boolean
  workers?: ManifestWorker[]
}

interface ConfigurationEntry {
  id: string
  name?: string
  description?: string
}

interface ConfigurationList {
  configurations?: ConfigurationEntry[]
}

interface WorkerRow {
  worker: string
  title: string
  description: string
  assets: number
}

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function disabledWorkersOf(value: JsonValue): string[] {
  const section = asObject(asObject(value).injectableUi)
  const list = section.disabledWorkers
  if (!Array.isArray(list)) return []
  return list.filter((w): w is string => typeof w === 'string')
}

export function InjectableUiConfigForm(props: ConfigFormProps & { host: Host }) {
  const { host } = props
  const [workers, setWorkers] = useState<WorkerRow[] | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)

  const disabled = useMemo(() => disabledWorkersOf(props.value), [props.value])

  useEffect(() => {
    let cancelled = false
    Promise.all([
      host.iii.trigger<UiManifest>('console::ui-manifest', {}),
      host.iii
        .trigger<ConfigurationList>('configuration::list', {})
        .catch(() => ({ configurations: [] }) as ConfigurationList),
    ])
      .then(([manifest, configs]) => {
        if (cancelled) return
        const byId = new Map(
          (configs.configurations ?? []).map((c) => [c.id, c]),
        )
        const rows = (manifest.workers ?? [])
          .filter((w) => w.worker !== 'console')
          .map((w) => {
            const entry = byId.get(w.worker)
            return {
              worker: w.worker,
              title: entry?.name || w.worker,
              description: entry?.description ?? '',
              assets: w.assets,
            }
          })
        setWorkers(rows)
      })
      .catch((e) => {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : String(e))
      })
    return () => {
      cancelled = true
    }
  }, [host])

  // Workers disabled in the draft but absent from the manifest (nothing
  // registered right now — e.g. the worker is down) must stay visible so
  // they can be re-enabled.
  const rows = useMemo(() => {
    if (!workers) return null
    const known = new Set(workers.map((w) => w.worker))
    const ghosts = disabled
      .filter((w) => !known.has(w) && w !== 'console')
      .map((worker) => ({
        worker,
        title: worker,
        description: '',
        assets: 0,
      }))
    return [...workers, ...ghosts].sort((a, b) =>
      a.worker.localeCompare(b.worker),
    )
  }, [workers, disabled])

  const toggle = (worker: string, enable: boolean) => {
    const next = enable
      ? disabled.filter((w) => w !== worker)
      : [...disabled.filter((w) => w !== worker), worker]
    const value = asObject(props.value)
    props.onChange({
      ...value,
      injectableUi: { ...asObject(value.injectableUi), disabledWorkers: next },
    })
  }

  // Deep-link focus (`#/workers/configuration/console/injectableUi`): a
  // custom form honors `focusField` itself.
  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const target = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${field}"]`,
    )
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  return (
    <div className="console-ui-form" ref={rootRef}>
      <span className="console-ui-form-caption">
        custom form · shipped by the console worker
      </span>

      <div className="console-ui-form-section" data-field="injectableUi">
        <span className="console-ui-form-title">injectable worker ui</span>
        <span className="console-ui-form-hint">
          workers can ship pages, renderers, and forms into this console at
          runtime. turn a worker off to hold its assets — every open tab
          drops them on save, and turning it back on restores them without a
          restart.
        </span>

        {rows === null && loadError === null ? (
          <div className="console-ui-toggle-list" aria-hidden>
            <div className="console-ui-toggle-card console-ui-toggle-skeleton" />
            <div className="console-ui-toggle-card console-ui-toggle-skeleton" />
          </div>
        ) : null}

        {loadError !== null ? (
          <div className="console-ui-form-error">
            could not load the worker list: {loadError}
          </div>
        ) : null}

        {rows !== null && rows.length === 0 ? (
          <div className="console-ui-form-empty">
            no worker is shipping injectable ui right now — this board fills
            in as workers register console:script / console:style assets.
          </div>
        ) : null}

        {rows !== null && rows.length > 0 ? (
          <div className="console-ui-toggle-list" role="group">
            {rows.map((row) => {
              const enabled = !disabled.includes(row.worker)
              return (
                <button
                  key={row.worker}
                  type="button"
                  role="switch"
                  aria-checked={enabled}
                  className="console-ui-toggle-card"
                  data-active={enabled ? 'true' : 'false'}
                  onClick={() => toggle(row.worker, !enabled)}
                >
                  <span className="console-ui-toggle-copy">
                    <span className="console-ui-toggle-title">
                      {row.title}
                      <span className="console-ui-toggle-meta">
                        {row.assets > 0
                          ? `${row.assets} asset${row.assets === 1 ? '' : 's'}`
                          : 'nothing registered right now'}
                      </span>
                    </span>
                    {row.description ? (
                      <span className="console-ui-toggle-description">
                        {row.description}
                      </span>
                    ) : null}
                  </span>
                  <span className="console-ui-switch" aria-hidden />
                </button>
              )
            })}
          </div>
        ) : null}
      </div>

      {props.errors && props.errors.size > 0 ? (
        <div className="console-ui-form-error">
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
