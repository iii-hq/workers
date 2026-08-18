/**
 * Custom configuration form for the `discovery` entry: the two knobs with
 * real explanations, plus the exact `<discovery_assist>` text the hook
 * injects — fetched from the worker (`discovery::hint-preview`) so the
 * preview can never drift from the shipped wording.
 */

import { useEffect, useRef, useState } from 'react'
import type { ConfigFormProps, Host, JsonValue } from '@iii-dev/console-ui'

type JsonObject = { [key: string]: JsonValue }

interface HintPreview {
  agent_trigger: string
  native: string
}

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? { ...value }
    : {}
}

function useHintPreview(host: Host): HintPreview | 'loading' | 'error' {
  const [state, setState] = useState<HintPreview | 'loading' | 'error'>(
    'loading',
  )
  useEffect(() => {
    let cancelled = false
    host.iii
      .trigger<HintPreview>('discovery::hint-preview', {})
      .then((preview) => {
        if (cancelled) return
        if (
          typeof preview?.agent_trigger === 'string' &&
          typeof preview?.native === 'string'
        ) {
          setState(preview)
        } else {
          setState('error')
        }
      })
      .catch(() => {
        if (!cancelled) setState('error')
      })
    return () => {
      cancelled = true
    }
  }, [host])
  return state
}

type ExposureMode = 'agent_trigger' | 'native'

export function DiscoveryConfigForm(props: ConfigFormProps & { host: Host }) {
  const value = asObject(props.value)
  const rootRef = useRef<HTMLDivElement>(null)
  const preview = useHintPreview(props.host)
  const [exposure, setExposure] = useState<ExposureMode>('agent_trigger')
  const injectHint = value.inject_hint !== false

  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const target = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${CSS.escape(field)}"]`,
    )
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  return (
    <div className="discovery-cfg" ref={rootRef}>
      <p className="discovery-cfg-intro">
        One-shot lexical function search. Both knobs hot-apply on save — no
        restart.
      </p>

      <section className="discovery-cfg-section" aria-labelledby="discovery-cfg-hint">
        <h3 id="discovery-cfg-hint">Search hint</h3>

        <label className="discovery-cfg-check" htmlFor="discovery-cfg-inject-hint">
          <input
            id="discovery-cfg-inject-hint"
            data-field="inject_hint"
            type="checkbox"
            checked={injectHint}
            onChange={(event) =>
              props.onChange({ ...value, inject_hint: event.target.checked })
            }
          />
          <span>
            <strong>Inject the search hint</strong>
            <small>
              Binds the <code>discovery::pre-generate</code> hook. Off unbinds
              it entirely — the hook never runs and the model only finds
              search_functions through normal discovery. Default: enabled.
            </small>
          </span>
        </label>

        <div className="discovery-cfg-field">
          <label htmlFor="discovery-cfg-min-workers">
            Minimum surface width (workers)
          </label>
          <input
            id="discovery-cfg-min-workers"
            data-field="hint_min_workers"
            className="discovery-cfg-input"
            type="number"
            min={0}
            step={1}
            inputMode="numeric"
            value={
              typeof value.hint_min_workers === 'number'
                ? value.hint_min_workers
                : ''
            }
            placeholder="2"
            aria-describedby="discovery-cfg-min-workers-hint"
            onChange={(event) => {
              const raw = event.target.value
              const next = { ...value }
              if (raw === '') {
                delete next.hint_min_workers
              } else {
                const parsed = Number(raw)
                if (!Number.isSafeInteger(parsed) || parsed < 0) return
                next.hint_min_workers = parsed
              }
              props.onChange(next)
            }}
          />
          <span className="discovery-cfg-hint" id="discovery-cfg-min-workers-hint">
            The hint only fires when the session exposes at least this many
            distinct non-engine workers; narrower surfaces resolve faster
            through normal discovery. 0 hints on every surface. Default: 2.
          </span>
        </div>

        <div
          className="discovery-cfg-preview"
          data-disabled={injectHint ? undefined : ''}
        >
          <div className="discovery-cfg-preview-head">
            <h4>Injected text</h4>
            <div
              aria-label="Function exposure mode"
              className="discovery-cfg-modes"
              role="group"
            >
              {(['agent_trigger', 'native'] as const).map((mode) => (
                <button
                  key={mode}
                  aria-pressed={exposure === mode}
                  className="discovery-cfg-mode"
                  type="button"
                  onClick={() => setExposure(mode)}
                >
                  {mode === 'agent_trigger' ? 'agent_trigger' : 'native'}
                  {mode === 'agent_trigger' ? (
                    <span className="discovery-cfg-mode-badge">default</span>
                  ) : null}
                </button>
              ))}
            </div>
          </div>
          <p className="discovery-cfg-hint">
            The exact block appended once per session to the system prompt.
            The two exposure modes differ only in the closing call
            instruction; <code>functions_generation</code> varies per
            generation.
          </p>
          <div aria-live="polite" className="discovery-cfg-preview-slot">
            {preview === 'loading' ? (
              <p className="discovery-cfg-hint">Loading the live hint text…</p>
            ) : preview === 'error' ? (
              <p className="discovery-cfg-hint" role="alert">
                Could not load the hint text from the discovery worker — is it
                connected?
              </p>
            ) : (
              <pre className="discovery-cfg-preview-text" tabIndex={0}>
                <code>{preview[exposure]}</code>
              </pre>
            )}
          </div>
        </div>
      </section>

      {props.errors && props.errors.size > 0 ? (
        <div className="discovery-cfg-errors" role="alert">
          {[...props.errors.entries()].map(([pointer, message]) => (
            <div key={pointer || message}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
