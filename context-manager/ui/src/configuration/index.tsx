import type { ConfigFormProps, JsonValue } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'

type JsonObject = { [key: string]: JsonValue }

type NumericField = {
  key: string
  label: string
  defaultValue: number
  description: string
}

const BUDGET_FIELDS: NumericField[] = [
  {
    key: 'reserved_tokens_cap',
    label: 'Reserve cap (tokens)',
    defaultValue: 20_000,
    description: 'Maximum model-input reserve after the percentage is applied.',
  },
  {
    key: 'reserved_pct',
    label: 'Context reserve (%)',
    defaultValue: 10,
    description: 'Share of the context window reserved for model output.',
  },
]

const PRUNING_FIELDS: NumericField[] = [
  {
    key: 'protect_recent_tokens',
    label: 'Protected recent output (tokens)',
    defaultValue: 40_000,
    description:
      'Newest function-output tokens protected from age-based pruning.',
  },
  {
    key: 'min_free_tokens',
    label: 'Minimum useful reduction (tokens)',
    defaultValue: 20_000,
    description: 'Skip normal pruning when it would release fewer tokens.',
  },
  {
    key: 'max_output_chars',
    label: 'Verbose output threshold (characters)',
    defaultValue: 2_000,
    description:
      'Shorter function outputs are not considered for normal pruning.',
  },
  {
    key: 'max_result_tokens',
    label: 'Per-result cap (tokens)',
    defaultValue: 20_000,
    description: 'Hard ceiling for each function result. Set to 0 to disable.',
  },
  {
    key: 'tail_turns',
    label: 'Verbatim tail (turns)',
    defaultValue: 2,
    description: 'Recent user and assistant turns retained during compaction.',
  },
]

const RUNTIME_FIELDS: NumericField[] = [
  {
    key: 'lease_ttl_secs',
    label: 'Compaction lease TTL (seconds)',
    defaultValue: 300,
    description:
      'How long another worker must honor an active compaction lease.',
  },
  {
    key: 'summarizer_timeout_ms',
    label: 'Summarizer timeout (milliseconds)',
    defaultValue: 320_000,
    description: 'Maximum duration of one summarizer request.',
  },
]

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? { ...value }
    : {}
}

function asString(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

function NumericInput(props: {
  field: NumericField
  value: JsonValue | undefined
  onChange(raw: string): void
}) {
  const id = `context-manager-cfg-${props.field.key}`
  const hintId = `${id}-hint`
  return (
    <div className="ctx-cfg-field">
      <label htmlFor={id}>{props.field.label}</label>
      <input
        id={id}
        data-field={props.field.key}
        className="ctx-cfg-input"
        type="number"
        min={0}
        step={1}
        inputMode="numeric"
        value={typeof props.value === 'number' ? props.value : ''}
        placeholder={String(props.field.defaultValue)}
        aria-describedby={hintId}
        onChange={(event) => props.onChange(event.target.value)}
      />
      <span className="ctx-cfg-hint" id={hintId}>
        {props.field.description} Default:{' '}
        {props.field.defaultValue.toLocaleString()}.
      </span>
    </div>
  )
}

export function ContextManagerConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const rootRef = useRef<HTMLDivElement>(null)

  const commitNumber = (field: string, raw: string) => {
    const next = { ...value }
    if (raw === '') {
      delete next[field]
    } else {
      const parsed = Number(raw)
      if (!Number.isSafeInteger(parsed) || parsed < 0) return
      next[field] = parsed
    }
    props.onChange(next)
  }

  const commitString = (field: string, raw: string) => {
    const next = { ...value }
    if (raw === '') delete next[field]
    else next[field] = raw
    props.onChange(next)
  }

  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const target = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${CSS.escape(field)}"]`,
    )
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  const fields = (items: NumericField[]) =>
    items.map((field) => (
      <NumericInput
        key={field.key}
        field={field}
        value={value[field.key]}
        onChange={(raw) => commitNumber(field.key, raw)}
      />
    ))

  return (
    <div className="ctx-cfg" ref={rootRef}>
      <p className="ctx-cfg-intro">
        Defaults for context assembly. Request-level options can override the
        matching setting, and saved changes apply to the next call.
      </p>

      <section className="ctx-cfg-section" aria-labelledby="ctx-cfg-budget">
        <h3 id="ctx-cfg-budget">Budget</h3>
        <div className="ctx-cfg-grid">{fields(BUDGET_FIELDS)}</div>
        <label className="ctx-cfg-check" htmlFor="context-manager-cfg-fallback">
          <input
            id="context-manager-cfg-fallback"
            data-field="allow_fallback_limits"
            type="checkbox"
            checked={value.allow_fallback_limits !== false}
            onChange={(event) =>
              props.onChange({
                ...value,
                allow_fallback_limits: event.target.checked,
              })
            }
          />
          <span>
            <strong>Allow fallback model limits</strong>
            <small>
              Use conservative 8,192 / 1,024 limits when model limits cannot be
              resolved. Default: enabled.
            </small>
          </span>
        </label>
      </section>

      <section className="ctx-cfg-section" aria-labelledby="ctx-cfg-pruning">
        <h3 id="ctx-cfg-pruning">Pruning &amp; compaction</h3>
        <div className="ctx-cfg-grid">{fields(PRUNING_FIELDS)}</div>
      </section>

      <section className="ctx-cfg-section" aria-labelledby="ctx-cfg-runtime">
        <h3 id="ctx-cfg-runtime">Runtime</h3>
        <div className="ctx-cfg-grid">{fields(RUNTIME_FIELDS)}</div>
        <div className="ctx-cfg-field ctx-cfg-field-wide">
          <label htmlFor="context-manager-cfg-lease-dir">Lease directory</label>
          <input
            id="context-manager-cfg-lease-dir"
            data-field="lease_dir"
            className="ctx-cfg-input"
            type="text"
            value={asString(value.lease_dir)}
            placeholder="data/context-manager"
            aria-describedby="context-manager-cfg-lease-dir-hint"
            onChange={(event) => commitString('lease_dir', event.target.value)}
          />
          <span
            className="ctx-cfg-hint"
            id="context-manager-cfg-lease-dir-hint"
          >
            Compaction lease files live here; a leading ~/ expands to the home
            directory. Default: data/context-manager under III_COMPOSE_DIR.
          </span>
        </div>
      </section>

      {props.errors && props.errors.size > 0 ? (
        <div className="ctx-cfg-errors" role="alert">
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
