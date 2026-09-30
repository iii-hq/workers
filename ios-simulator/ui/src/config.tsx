import {
  type ConfigFormProps,
  Input,
  type JsonValue,
  SettingsField,
  SettingsList,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { formatDuration } from '@iii-dev/console-ui/format'
import { useEffect, useRef } from 'react'

type Draft = Record<string, JsonValue>

/** Defaults mirror `WorkerConfig::default()` in src/config.rs. */
const DEFAULTS = {
  data_dir: 'data/ios-simulator',
  max_booted: 3,
  max_booted_per_tenant: 2,
  max_devices_per_tenant: 10,
  stream_fps: 30,
  stream_max_dimension: 1000,
  stream_quality: 70,
  max_recording_seconds: 600,
} as const

function draftOf(value: JsonValue): Draft {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Draft) : {}
}

/** Set a key, or delete it (restoring the default) when `next` is undefined. */
function withKey(value: JsonValue, key: string, next: JsonValue | undefined): Draft {
  const copy = { ...draftOf(value) }
  if (next === undefined) delete copy[key]
  else copy[key] = next
  return copy
}

export function IosSimulatorConfigForm({ value, onChange, errors, focusField }: ConfigFormProps) {
  const draft = draftOf(value)
  const rootRef = useRef<HTMLDivElement>(null)

  const focusKey = focusField?.map(String).join('.') ?? ''
  useEffect(() => {
    if (!focusKey) return
    const target = rootRef.current?.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusKey)}"]`)
    target?.scrollIntoView({ block: 'center' })
    target?.focus()
  }, [focusKey])

  const text = (key: string, label: string, description: string, placeholder: string) => (
    <SettingsField
      field={key}
      label={label}
      description={description}
      error={errors?.get(`/${key}`)}
      layout="stacked"
      controlSize="full"
      renderControl={(props) => (
        <Input
          {...props}
          value={typeof draft[key] === 'string' ? (draft[key] as string) : ''}
          placeholder={placeholder}
          onChange={(next) => onChange(withKey(value, key, next.trim() ? next : undefined))}
          className="ios-ui-mono"
        />
      )}
    />
  )

  const toggle = (key: string, label: string, description: string, fallback: boolean) => (
    <SettingsField
      field={key}
      label={label}
      description={description}
      error={errors?.get(`/${key}`)}
      layout="inline"
      controlSize="fit"
      renderControl={(props) => (
        <Switch
          {...props}
          checked={typeof draft[key] === 'boolean' ? (draft[key] as boolean) : fallback}
          onChange={(e) => onChange(withKey(value, key, e.currentTarget.checked))}
        />
      )}
    />
  )

  const number = (
    key: keyof typeof DEFAULTS,
    label: string,
    description: string,
    [min, max]: [number, number],
    echo?: (n: number) => string,
  ) => {
    const current = typeof draft[key] === 'number' ? (draft[key] as number) : undefined
    const shown = current ?? (DEFAULTS[key] as number)
    return (
      <SettingsField
        field={key}
        label={label}
        description={echo ? `${description} ${echo(shown)}.` : description}
        error={errors?.get(`/${key}`)}
        controlSize="compact"
        renderControl={(props) => (
          <Input
            {...props}
            type="number"
            inputMode="numeric"
            min={min}
            max={max}
            value={current === undefined ? '' : String(current)}
            placeholder={String(DEFAULTS[key])}
            onChange={(next) => {
              const n = Number.parseInt(next, 10)
              onChange(withKey(value, key, next.trim() === '' || Number.isNaN(n) ? undefined : n))
            }}
          />
        )}
      />
    )
  }

  return (
    <div className="ios-ui-config" ref={rootRef}>
      <SettingsSection
        title="Storage"
        description="Where simulators, screenshots and recordings live. Changes apply to the next operation."
      >
        <SettingsList>
          {text(
            'data_dir',
            'Data folder',
            'Each tenant gets tenants/<tenant>/devices and tenants/<tenant>/media here. Relative to the project.',
            DEFAULTS.data_dir,
          )}
          {text(
            'developer_dir',
            'Xcode developer directory',
            'Leave empty to use the Xcode selected with xcode-select.',
            'xcode-select -p',
          )}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Tenancy"
        description="One Mac, several consumers: each tenant only ever sees its own simulators and media."
      >
        <SettingsList>
          {toggle(
            'share_system_devices',
            'Default tenant uses this Mac’s simulators',
            'The default tenant manages the simulators Xcode and Simulator.app show. Turn off on a shared Mac.',
            true,
          )}
          {toggle(
            'require_tenant',
            'Require a tenant on every call',
            'Refuse calls without a tenant instead of treating them as default. Turn on when serving an API.',
            false,
          )}
        </SettingsList>
      </SettingsSection>

      <SettingsSection title="Capacity" description="Booted simulators cost memory; cap them to what the Mac holds.">
        <SettingsList>
          {number('max_booted', 'Booted at once, whole Mac', 'Across every tenant.', [1, 64])}
          {number('max_booted_per_tenant', 'Booted at once, per tenant', 'Keeps one tenant from taking every slot.', [
            1, 64,
          ])}
          {number('max_devices_per_tenant', 'Simulators per tenant', 'Created through this worker.', [1, 500])}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Live view"
        description="Only while someone watches. Applies the next time a live view starts."
      >
        <SettingsList>
          {number('stream_fps', 'Frame rate', 'Frames per second at most; a still screen sends none.', [1, 60])}
          {number('stream_max_dimension', 'Frame size', 'Longest edge in pixels. Touch keeps full resolution.', [
            240, 4096,
          ])}
          {number('stream_quality', 'JPEG quality', '1–100.', [1, 100])}
        </SettingsList>
      </SettingsSection>

      <SettingsSection title="Recording">
        <SettingsList>
          {number(
            'max_recording_seconds',
            'Longest recording',
            'Recordings stop by themselves after',
            [5, 7200],
            (s) => formatDuration(s * 1000),
          )}
        </SettingsList>
      </SettingsSection>
    </div>
  )
}
