import {
  Button,
  Chip,
  type ConfigFormProps,
  Input,
  type JsonValue,
  RawValueInput,
  Select,
  SettingsField,
  SettingsList,
  SettingsSection,
  StatusPanel,
} from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useRef } from 'react'
import { readCronConfiguration, withAdapterConfigValue, withAdapterName, withAdapterValue, withRedisUrl } from './model'

function firstError(errors: ConfigFormProps['errors'], pointers: readonly string[]) {
  for (const pointer of pointers) {
    const message = errors?.get(pointer)
    if (message) return message
  }
  return undefined
}

function OpaqueCronSetting({
  id,
  field,
  label,
  description,
  value,
  error,
  replacementLabel,
  onChange,
  onUseLiteral,
}: {
  id: string
  field: string
  label: string
  description: string
  value: JsonValue
  error?: ReactNode
  replacementLabel: string
  onChange(next: string): void
  onUseLiteral(): void
}) {
  if (typeof value === 'string') {
    return (
      <SettingsField
        id={id}
        field={field}
        data-path={field}
        label={label}
        description={description}
        error={error}
        layout="stacked"
        controlSize="full"
        renderControl={(controlProps) => (
          <RawValueInput
            {...controlProps}
            label={label}
            kind={value.trim().startsWith('${') ? 'environment' : 'custom'}
            value={value}
            replacementLabel={replacementLabel}
            onChange={onChange}
            onUseLiteral={onUseLiteral}
          />
        )}
      />
    )
  }

  return (
    <SettingsField
      id={id}
      field={field}
      data-path={field}
      label={label}
      description={description}
      error={error}
      controlSize="fit"
      renderControl={(controlProps) => (
        <div className="cron-ui-config-opaque">
          <Chip tone="warning">Custom value preserved</Chip>
          <Button {...controlProps} type="button" variant="ghost" size="sm" onClick={onUseLiteral}>
            Use {replacementLabel}
          </Button>
        </div>
      )}
    />
  )
}

export function CronConfigForm(props: ConfigFormProps) {
  const model = readCronConfiguration(props.value)
  const adapterError = firstError(props.errors, ['/adapter/name', '/adapter'])
  const configError = firstError(props.errors, ['/adapter/config'])
  const redisUrlError = firstError(props.errors, ['/adapter/config/redis_url'])
  const rootError = firstError(props.errors, ['', '/'])
  const adapterOptions = [
    { value: 'local', label: 'Local · Process-local locks' },
    { value: 'redis', label: 'Redis · Shared distributed locks' },
  ]
  if (!adapterOptions.some((option) => option.value === model.adapterName)) {
    adapterOptions.push({
      value: model.adapterName,
      label: `${model.adapterName} · Unsupported adapter`,
    })
  }

  const rootRef = useRef<HTMLDivElement | null>(null)
  const handledFocusKeyRef = useRef('')
  const focusKey = props.focusField?.length ? JSON.stringify(props.focusField.map(String)) : ''
  useEffect(() => {
    if (!focusKey) {
      handledFocusKeyRef.current = ''
      return
    }
    if (handledFocusKeyRef.current === focusKey || !rootRef.current) return
    const path = JSON.parse(focusKey) as string[]
    const exact = rootRef.current.querySelector<HTMLElement>(`[data-path="${CSS.escape(path.join('.'))}"]`)
    const leaf = rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(path[path.length - 1])}"]`)
    const topLevel = rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(path[0])}"]`)
    const target = exact ?? leaf ?? topLevel
    target?.scrollIntoView({ block: 'center' })
    const focusable = target?.matches('input, button, [tabindex]')
      ? target
      : target?.querySelector<HTMLElement>('input, button, [tabindex]')
    if (!focusable) return
    handledFocusKeyRef.current = focusKey
    focusable?.focus()
  }, [focusKey])

  return (
    <div className="cron-ui-form" ref={rootRef}>
      <SettingsSection
        title="Scheduler coordination"
        description="Choose how Cron workers coordinate a job before it runs."
      >
        <SettingsList>
          {model.hasOpaqueRoot ? (
            <OpaqueCronSetting
              id="cron-cfg-root-value"
              field="configuration"
              label="Cron configuration"
              description="This worker configuration is provided as one opaque value. It remains unchanged until you edit it or explicitly replace it."
              value={props.value}
              error={rootError}
              replacementLabel="Local"
              onChange={props.onChange}
              onUseLiteral={() => props.onChange({})}
            />
          ) : model.hasOpaqueAdapter ? (
            <OpaqueCronSetting
              id="cron-cfg-adapter-value"
              field="adapter"
              label="Lock adapter"
              description="This configuration supplies the entire adapter as an opaque value."
              value={model.value.adapter as JsonValue}
              error={adapterError}
              replacementLabel="Local"
              onChange={(next) => props.onChange(withAdapterValue(props.value, next))}
              onUseLiteral={() => props.onChange(withAdapterValue(props.value, { name: 'local' }))}
            />
          ) : (
            <SettingsField
              id="cron-cfg-adapter"
              field="adapter.name"
              data-field="adapter"
              data-path="adapter.name"
              label="Lock adapter"
              description="Local coordinates one process. Redis prevents duplicate execution across workers."
              error={adapterError}
              renderControl={(controlProps) => (
                <div className="cron-ui-config-select">
                  <Select
                    {...controlProps}
                    value={model.adapterName}
                    options={adapterOptions}
                    onChange={(next) => props.onChange(withAdapterName(props.value, next))}
                    aria-label="Lock adapter"
                  />
                </div>
              )}
            />
          )}

          {!model.hasOpaqueRoot && model.adapterName === 'redis' ? (
            model.hasOpaqueAdapterConfig ? (
              <OpaqueCronSetting
                id="cron-cfg-adapter-config"
                field="adapter.config"
                label="Adapter configuration"
                description="This configuration supplies the Redis adapter payload as an opaque value."
                value={model.adapter.config as JsonValue}
                error={configError}
                replacementLabel="Redis defaults"
                onChange={(next) => props.onChange(withAdapterConfigValue(props.value, next))}
                onUseLiteral={() => props.onChange(withAdapterConfigValue(props.value, undefined))}
              />
            ) : (
              <SettingsField
                id="cron-cfg-redis-url"
                field="adapter.config.redis_url"
                data-field="redis_url"
                data-path="adapter.config.redis_url"
                label="Redis URL"
                description="Connection used for distributed locks. Empty uses redis://localhost:6379."
                error={redisUrlError}
                renderControl={(controlProps) => (
                  <Input
                    {...controlProps}
                    className="cron-ui-config-control"
                    type="text"
                    inputMode="url"
                    value={model.redisUrl}
                    placeholder="redis://localhost:6379"
                    autoComplete="off"
                    spellCheck={false}
                    aria-label="Redis URL"
                    onChange={(next) => props.onChange(withRedisUrl(props.value, next))}
                  />
                )}
              />
            )
          ) : null}
        </SettingsList>
      </SettingsSection>

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
