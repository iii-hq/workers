import { Input, SettingsField, SettingsList, type ConfigFormProps, type JsonValue } from '@iii-dev/console-ui'

function asObject(value: JsonValue | undefined): Record<string, JsonValue> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
}

export function ControlPlaneConfigForm({ value, onChange, errors }: ConfigFormProps) {
  const config = asObject(value)
  const summary = asObject(config.summary)
  return (
    <SettingsList>
      <SettingsField
        label="Summary model"
        field="summary.model"
        description="Use provider::model. Leave blank to turn off the summary."
        error={errors?.get('/summary/model')}
        renderControl={(props) => (
          <Input
            {...props}
            value={String(summary.model ?? '')}
            onChange={(model) => {
              const next = { ...summary }
              if (model.trim()) next.model = model.trim()
              else delete next.model
              onChange({ ...config, summary: next })
            }}
          />
        )}
      />
      <SettingsField
        label="Stall timeout"
        field="stall_after_min"
        description="Minutes without progress before a waiting run is flagged as stalled."
        error={errors?.get('/stall_after_min')}
        renderControl={(props) => (
          <Input
            {...props}
            type="number"
            min={0}
            step="any"
            placeholder="30"
            value={String(config.stall_after_min ?? '')}
            onChange={(minutes) => {
              const next = { ...config }
              if (minutes) next.stall_after_min = Number(minutes)
              else delete next.stall_after_min
              onChange(next)
            }}
          />
        )}
      />
    </SettingsList>
  )
}
