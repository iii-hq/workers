import {
  type ConfigFormProps,
  Input,
  SettingsField,
  SettingsList,
  SettingsSection,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'

const limitFields = [
  {
    field: 'max_request_bytes',
    label: 'Maximum request bytes',
    fallback: 8388608,
    description: 'Maximum encoded JSON bytes per upstream evaluation. Clear to use 8388608 (8 MiB).',
  },
  {
    field: 'max_response_bytes',
    label: 'Maximum response bytes',
    fallback: 8388608,
    description: 'Maximum response bytes per evaluation or model listing. Clear to use 8388608 (8 MiB).',
  },
  {
    field: 'max_timeout_ms',
    label: 'Maximum timeout (ms)',
    fallback: 300000,
    description: 'Maximum caller timeout, including queue waits and response reading. Clear to use 300000 (5 minutes).',
  },
]
const knownFields = ['api_key', 'model', ...limitFields.map(({ field }) => field)]

export function JevConfigForm(props: ConfigFormProps) {
  const rootRef = useRef<HTMLDivElement>(null)
  const focusField = props.focusField?.[0]
  useEffect(() => {
    if (!focusField || !knownFields.includes(focusField)) return
    const input = rootRef.current?.querySelector<HTMLInputElement>(`input[name="${focusField}"]`)
    input?.scrollIntoView?.({ block: 'center' })
    input?.focus()
  }, [focusField])

  if (props.value !== null && (typeof props.value !== 'object' || Array.isArray(props.value))) {
    return (
      <StatusPanel
        variant="info"
        headline="JEV configuration is supplied as a single value"
        detail="Edit this value in the configuration source. It is preserved until you replace it with an object."
      />
    )
  }
  const value = props.value ?? {}
  const setString = (field: string, raw: string) => {
    const next = { ...value }
    if (raw === '') delete next[field]
    else next[field] = raw
    props.onChange(next)
  }
  const setNumber = (field: string, raw: string) => {
    const next = { ...value }
    if (raw === '') delete next[field]
    else {
      const number = Number(raw)
      if (!Number.isSafeInteger(number) || number <= 0) return
      next[field] = number
    }
    props.onChange(next)
  }
  const unassociatedErrors = [...(props.errors?.entries() ?? [])].filter(
    ([pointer]) => !knownFields.some((field) => pointer === `/${field}`),
  )

  return (
    <div className="jev-ui-form" ref={rootRef}>
      <SettingsSection
        title="JEV evaluation"
        description="Configure TypeSafe evaluation and model listing for calling workers. Changes apply to new calls without restarting."
      >
        <SettingsList>
          <SettingsField
            id="jev-cfg-api_key"
            field="api_key"
            label="API key"
            description="A configured key takes precedence over TYPESAFE_API_KEY in the JEV worker process environment. Clear it to use the environment key. Restart JEV after changing its environment."
            error={props.errors?.get('/api_key')}
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                type="password"
                autoComplete="new-password"
                spellCheck={false}
                aria-label="API key"
                placeholder="Use TYPESAFE_API_KEY"
                value={typeof value.api_key === 'string' ? value.api_key : ''}
                onChange={(next) => setString('api_key', next)}
              />
            )}
          />
          <SettingsField
            id="jev-cfg-model"
            field="model"
            label="Default model"
            description="Used when an evaluation omits its model. Callers can supply their own model override. Clear to use jev-1.13.0."
            error={props.errors?.get('/model')}
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                type="text"
                autoComplete="off"
                spellCheck={false}
                aria-label="Default model"
                placeholder="jev-1.13.0"
                value={typeof value.model === 'string' ? value.model : ''}
                onChange={(next) => setString('model', next)}
              />
            )}
          />
        </SettingsList>
      </SettingsSection>
      <SettingsSection
        title="Execution limits"
        description="Positive integers only. Byte limits are local transport safeguards; provider token limits still apply. Callers can supply a shorter timeout."
      >
        <SettingsList>
          {limitFields.map(({ field, label, fallback, description }) => (
            <SettingsField
              key={field}
              id={`jev-cfg-${field}`}
              field={field}
              label={label}
              description={description}
              error={props.errors?.get(`/${field}`)}
              renderControl={(controlProps) => (
                <Input
                  {...controlProps}
                  type="number"
                  min={1}
                  step={1}
                  aria-label={label}
                  placeholder={String(fallback)}
                  value={typeof value[field] === 'number' ? String(value[field]) : ''}
                  onChange={(next) => setNumber(field, next)}
                />
              )}
            />
          ))}
        </SettingsList>
      </SettingsSection>
      {unassociatedErrors.length > 0 ? (
        <StatusPanel
          variant="alert"
          headline="Review the configuration errors"
          detail={unassociatedErrors.map(([pointer, message]) => (
            <div key={pointer}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        />
      ) : null}
    </div>
  )
}
