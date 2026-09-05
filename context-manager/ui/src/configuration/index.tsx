import {
  type ConfigFormProps,
  Input,
  type JsonValue,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
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
    description: 'Newest function-output tokens kept during normal pruning. Set to 0 to disable this window.',
  },
  {
    key: 'decay_user_turns',
    label: 'Function result decay (user turns)',
    defaultValue: 0,
    description: 'Prune eligible outputs after this many subsequent user turns. Set to 0 to turn decay off.',
  },
  {
    key: 'protected_user_turns',
    label: 'Protected recent turns',
    defaultValue: 2,
    description: 'Keep outputs in this many most recent user turns. Set to 0 to disable this protection.',
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
    description: 'Longer outputs are immediately eligible outside protection; shorter outputs can still decay.',
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
    description: 'How long another worker must honor an active compaction lease.',
  },
  {
    key: 'summarizer_timeout_ms',
    label: 'Summarizer timeout (milliseconds)',
    defaultValue: 320_000,
    description: 'Maximum duration of one summarizer request.',
  },
]

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
  return `context-manager-cfg-error-${suffix}`
}

function errorIdFor(errors: ConfigFormProps['errors'], pointer: string): string | undefined {
  return errors?.has(pointer) ? configurationErrorId(pointer) : undefined
}

function NumericSetting(props: {
  field: NumericField
  value: JsonValue | undefined
  errorId?: string
  onChange(raw: string): void
}) {
  const id = `context-manager-cfg-${props.field.key}`
  const descriptionId = `${id}-description`
  const defaultId = `${id}-default`
  return (
    <SettingsRow
      data-field={props.field.key}
      label={<label htmlFor={id}>{props.field.label}</label>}
      description={<span id={descriptionId}>{props.field.description}</span>}
      meta={<span id={defaultId}>Default: {props.field.defaultValue.toLocaleString()}.</span>}
      control={
        <Input
          id={id}
          className="ctx-cfg-control ctx-cfg-number"
          type="number"
          min={0}
          step={1}
          inputMode="numeric"
          value={typeof props.value === 'number' ? String(props.value) : ''}
          placeholder={String(props.field.defaultValue)}
          aria-label={props.field.label}
          aria-invalid={props.errorId ? true : undefined}
          aria-describedby={describedBy(descriptionId, defaultId, props.errorId)}
          onChange={props.onChange}
        />
      }
    />
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
    const target = rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(field)}"]`)
    target?.scrollIntoView({ block: 'center' })
    const focusable = target?.matches('input, button, [tabindex]')
      ? target
      : target?.querySelector<HTMLElement>('input, button, [tabindex]')
    focusable?.focus()
  }, [props.focusField])

  const fields = (items: NumericField[]) =>
    items.map((field) => (
      <NumericSetting
        key={field.key}
        field={field}
        value={value[field.key]}
        errorId={errorIdFor(props.errors, `/${field.key}`)}
        onChange={(raw) => commitNumber(field.key, raw)}
      />
    ))

  return (
    <div className="ctx-cfg" ref={rootRef}>
      <SettingsSection
        title="Context budget"
        description="Defaults for context assembly. Request-level options can override matching settings."
      >
        <SettingsList>
          {fields(BUDGET_FIELDS)}
          <SettingsRow
            data-field="allow_fallback_limits"
            label={<label htmlFor="context-manager-cfg-fallback">Fallback model limits</label>}
            description={
              <span id="context-manager-cfg-fallback-description">
                Use conservative 8,192 input and 1,024 output limits when model limits cannot be resolved.
              </span>
            }
            meta={<span id="context-manager-cfg-fallback-default">Default: on.</span>}
            control={
              <Switch
                id="context-manager-cfg-fallback"
                checked={value.allow_fallback_limits !== false}
                aria-label="Fallback model limits"
                aria-invalid={errorIdFor(props.errors, '/allow_fallback_limits') ? true : undefined}
                aria-describedby={describedBy(
                  'context-manager-cfg-fallback-description',
                  'context-manager-cfg-fallback-default',
                  errorIdFor(props.errors, '/allow_fallback_limits'),
                )}
                onChange={(event) =>
                  props.onChange({
                    ...value,
                    allow_fallback_limits: event.currentTarget.checked,
                  })
                }
              />
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Pruning and compaction"
        description="Control which function output is retained verbatim and when compaction is worthwhile."
      >
        <SettingsList>{fields(PRUNING_FIELDS)}</SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Runtime"
        description="Set compaction coordination and summarizer request limits. Changes apply to the next call."
      >
        <SettingsList>
          {fields(RUNTIME_FIELDS)}
          <SettingsRow
            data-field="lease_dir"
            label={<label htmlFor="context-manager-cfg-lease-dir">Lease directory</label>}
            description={
              <span id="context-manager-cfg-lease-dir-description">
                Stores compaction lease files. A leading ~/ expands to the home directory.
              </span>
            }
            meta={
              <span id="context-manager-cfg-lease-dir-default">
                Default: data/context-manager under III_COMPOSE_DIR.
              </span>
            }
            control={
              <Input
                id="context-manager-cfg-lease-dir"
                className="ctx-cfg-control ctx-cfg-path"
                type="text"
                value={asString(value.lease_dir)}
                placeholder="data/context-manager"
                spellCheck={false}
                autoComplete="off"
                aria-label="Lease directory"
                aria-invalid={errorIdFor(props.errors, '/lease_dir') ? true : undefined}
                aria-describedby={describedBy(
                  'context-manager-cfg-lease-dir-description',
                  'context-manager-cfg-lease-dir-default',
                  errorIdFor(props.errors, '/lease_dir'),
                )}
                onChange={(next) => commitString('lease_dir', next)}
              />
            }
          />
        </SettingsList>
      </SettingsSection>

      {props.errors && props.errors.size > 0 ? (
        <div className="ctx-cfg-errors" role="alert">
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
