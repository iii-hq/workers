import {
  type ConfigFormProps,
  Input,
  type JsonValue,
  Select,
  type SelectOption,
  SettingsField,
  SettingsList,
  SettingsSection,
} from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import type { Strategy } from '../lib/resize'

type Draft = Record<string, JsonValue>

const STRATEGIES: SelectOption<Strategy>[] = [
  { value: 'scale-to-fit', label: 'Scale to fit', description: 'Fits the whole image inside the box.' },
  {
    value: 'crop-to-fit',
    label: 'Crop to fit',
    description: "Centre-crops to the box's aspect ratio, then shrinks.",
  },
]

function draftOf(value: JsonValue | undefined): Draft {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Draft) : {}
}

/**
 * Set the key at `path`, or delete it (restoring the default) when `next` is
 * undefined. A nested object left empty goes too, so `quality: {}` never stays.
 */
export function withPath(value: JsonValue | undefined, path: readonly string[], next: JsonValue | undefined): Draft {
  const [key, ...rest] = path
  const copy = { ...draftOf(value) }
  let leaf = next
  if (rest.length > 0) {
    const nested = withPath(copy[key], rest, next)
    leaf = Object.keys(nested).length > 0 ? nested : undefined
  }
  if (leaf === undefined) delete copy[key]
  else copy[key] = leaf
  return copy
}

function read(value: JsonValue, path: readonly string[]): JsonValue | undefined {
  return path.reduce<JsonValue | undefined>((node, key) => draftOf(node)[key], value)
}

export function ImageResizeConfigForm({ value, onChange, errors, focusField }: ConfigFormProps) {
  const rootRef = useRef<HTMLDivElement>(null)

  const focusKey = focusField?.map(String).join('.') ?? ''
  useEffect(() => {
    if (!focusKey) return
    const target = rootRef.current?.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusKey)}"]`)
    target?.scrollIntoView({ block: 'center' })
    target?.focus()
  }, [focusKey])

  const number = (path: string[], label: string, fallback: number, max?: number, description?: string) => {
    const current = read(value, path)
    return (
      <SettingsField
        field={path.join('.')}
        label={label}
        description={description}
        error={errors?.get(`/${path.join('/')}`)}
        controlSize="compact"
        renderControl={(props) => (
          <Input
            {...props}
            type="number"
            inputMode="numeric"
            min={1}
            max={max}
            step={1}
            value={typeof current === 'number' ? String(current) : ''}
            placeholder={String(fallback)}
            onChange={(next) => {
              const n = Number.parseInt(next, 10)
              onChange(withPath(value, path, Number.isNaN(n) ? undefined : n))
            }}
          />
        )}
      />
    )
  }

  const strategy = read(value, ['strategy'])

  return (
    <div className="imgr-config" ref={rootRef}>
      <SettingsSection
        title="Default box"
        description="Used by calls that leave the box out. A change applies to the next call and moves the output of every caller that relies on the defaults."
      >
        <SettingsList>
          {number(['width'], 'Width', 200, undefined, 'Pixels.')}
          {number(['height'], 'Height', 200, undefined, 'Pixels.')}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Default output"
        description="Used by calls that leave the strategy or quality out. PNG is lossless and has no quality."
      >
        <SettingsList>
          <SettingsField
            field="strategy"
            label="Strategy"
            error={errors?.get('/strategy')}
            renderControl={(props) => (
              <Select
                {...props}
                value={typeof strategy === 'string' ? (strategy as Strategy) : undefined}
                options={STRATEGIES}
                placeholder="Scale to fit"
                allowEmpty
                emptyLabel="Built-in default (Scale to fit)"
                onClear={() => onChange(withPath(value, ['strategy'], undefined))}
                onChange={(next) => onChange(withPath(value, ['strategy'], next))}
                sheetTitle="Strategy"
              />
            )}
          />
          {number(['quality', 'jpeg'], 'JPEG quality', 85, 100, '1 to 100.')}
          {number(['quality', 'webp'], 'WebP quality', 80, 100, '1 to 100.')}
        </SettingsList>
      </SettingsSection>
    </div>
  )
}
