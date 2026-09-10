import {
  Button,
  IconButton,
  Input,
  type JsonValue,
  Select,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { Trash2 } from 'lucide-react'
import { type ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import {
  emptyStructuredValue,
  renameStructuredKey,
  type StructuredValueKind,
  structuredValueKind,
} from './structured-value'
import type {
  ConfigPath,
  DynamicMapFieldSpec,
  FilterListFieldSpec,
  FormFieldSpec,
  ObjectCollectionFieldSpec,
  ObjectMapFieldSpec,
  PermissionRulesFieldSpec,
  StringListFieldSpec,
  StructuredValueFieldSpec,
  VariantFieldSpec,
} from './types'
import {
  asObject,
  atPath,
  booleanLiteralForRawValue,
  cloneJson,
  deleteAtPath,
  displayValue,
  isEnvironmentValue,
  isObject,
  isRawTypedValue,
  type JsonObject,
  joinPath,
  numberLiteralForRawValue,
  pointerFor,
  selectLiteralForRawValue,
  selectVariantValue,
  setAtPath,
} from './value'

export interface FieldRenderContext {
  root: JsonValue
  onChange(next: JsonValue): void
  errors?: ReadonlyMap<string, string>
}

interface FieldProps extends FieldRenderContext {
  field: FormFieldSpec
  basePath?: ConfigPath
}

function errorFor(errors: ReadonlyMap<string, string> | undefined, path: ConfigPath): string | undefined {
  if (!errors) return undefined
  const pointer = pointerFor(path)
  const exact = errors.get(pointer)
  if (exact) return exact
  for (const [candidate, message] of errors) {
    if (candidate.startsWith(`${pointer}/`)) return message
  }
  return undefined
}

function fieldId(path: ConfigPath): string {
  return `console-worker-config-${path.join('-').replace(/[^a-zA-Z0-9_-]/g, '-')}`
}

function setFieldValue(context: FieldRenderContext, path: ConfigPath, value: JsonValue, optional = false) {
  if (optional && value === '') {
    context.onChange(deleteAtPath(context.root, path))
  } else {
    context.onChange(setAtPath(context.root, path, value))
  }
}

function RawScalarControl({
  id,
  label,
  value,
  replacementLabel,
  error,
  errorId,
  onChange,
  onUseLiteral,
}: {
  id: string
  label: string
  value: string
  replacementLabel: string
  error?: string
  errorId?: string
  onChange(next: string): void
  onUseLiteral(): void
}) {
  const environmentBacked = isEnvironmentValue(value) || /^\$\{[^}]*$/.test(value)
  return (
    <div
      className="console-worker-config-template-control"
      data-environment-template={environmentBacked ? 'true' : 'false'}
    >
      <span className="console-worker-config-template-kind">{environmentBacked ? 'Environment' : 'Custom value'}</span>
      <Input
        id={id}
        type="text"
        value={value}
        onChange={onChange}
        spellCheck={false}
        autoComplete="off"
        aria-label={`${label} raw value`}
        aria-invalid={Boolean(error)}
        aria-describedby={errorId}
        className="console-worker-config-control console-worker-config-template-input"
      />
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={onUseLiteral}
        aria-label={`Replace ${label} environment value with ${replacementLabel}`}
      >
        Use {replacementLabel}
      </Button>
    </div>
  )
}

function ScalarField({ field, basePath = [], ...context }: FieldProps) {
  if (
    field.kind !== 'text' &&
    field.kind !== 'password' &&
    field.kind !== 'number' &&
    field.kind !== 'select' &&
    field.kind !== 'switch'
  ) {
    return null
  }

  const path = joinPath(basePath, field.path)
  const value = atPath(context.root, path)
  const error = errorFor(context.errors, path)
  const id = fieldId(path)
  const errorId = error ? `${id}-error` : undefined
  let control: ReactNode

  if (field.kind === 'switch') {
    if (isRawTypedValue(value)) {
      const replacement = booleanLiteralForRawValue(value, field.defaultValue ?? false)
      control = (
        <RawScalarControl
          id={id}
          label={field.label}
          value={value}
          replacementLabel={replacement ? 'on' : 'off'}
          error={error}
          errorId={errorId}
          onChange={(next) => setFieldValue(context, path, next)}
          onUseLiteral={() => setFieldValue(context, path, replacement)}
        />
      )
    } else {
      control = (
        <Switch
          name={path.join('.')}
          checked={typeof value === 'boolean' ? value : (field.defaultValue ?? false)}
          onChange={(event) => setFieldValue(context, path, event.currentTarget.checked)}
          aria-label={field.label}
          aria-invalid={Boolean(error)}
          aria-describedby={errorId}
        />
      )
    }
  } else if (field.kind === 'select') {
    const selected = typeof value === 'string' ? value : undefined
    const knownSelection = field.options.some((option) => option.value === selected)
    if (selected !== undefined && !knownSelection) {
      const fallback = field.options[0]?.value ?? ''
      const replacement = selectLiteralForRawValue(
        selected,
        field.options.map((option) => option.value),
        fallback,
      )
      const replacementLabel = field.options.find((option) => option.value === replacement)?.label ?? replacement
      control = (
        <RawScalarControl
          id={id}
          label={field.label}
          value={selected}
          replacementLabel={replacementLabel}
          error={error}
          errorId={errorId}
          onChange={(next) => setFieldValue(context, path, next)}
          onUseLiteral={() => setFieldValue(context, path, replacement)}
        />
      )
    } else {
      control = (
        <Select
          value={selected}
          options={field.options.map(({ value, label, description }) => ({
            value,
            label,
            title: description,
          }))}
          onChange={(next) => setFieldValue(context, path, next)}
          allowEmpty={field.optional}
          emptyLabel="Not set"
          onClear={field.optional ? () => context.onChange(deleteAtPath(context.root, path)) : undefined}
          placeholder={field.placeholder}
          aria-label={field.label}
          aria-invalid={Boolean(error)}
          aria-describedby={errorId}
          className="console-worker-config-select"
        />
      )
    }
  } else if (field.kind === 'number') {
    if (isRawTypedValue(value)) {
      const replacement = numberLiteralForRawValue(value, field.min ?? 0)
      control = (
        <RawScalarControl
          id={id}
          label={field.label}
          value={value}
          replacementLabel={String(replacement)}
          error={error}
          errorId={errorId}
          onChange={(next) => setFieldValue(context, path, next)}
          onUseLiteral={() => setFieldValue(context, path, replacement)}
        />
      )
    } else {
      control = (
        <Input
          id={id}
          type="number"
          inputMode="decimal"
          min={field.min}
          max={field.max}
          step={field.step ?? 1}
          value={displayValue(value)}
          onChange={(next) => {
            if (field.optional && next === '') {
              context.onChange(deleteAtPath(context.root, path))
              return
            }
            const parsed = Number(next)
            setFieldValue(context, path, next !== '' && Number.isFinite(parsed) ? parsed : next)
          }}
          aria-invalid={Boolean(error)}
          aria-describedby={errorId}
          aria-label={field.label}
          className="console-worker-config-control console-worker-config-number"
        />
      )
    }
  } else {
    control = (
      <Input
        id={id}
        type={field.kind === 'password' ? 'password' : 'text'}
        value={displayValue(value)}
        onChange={(next) => setFieldValue(context, path, next, field.optional)}
        placeholder={field.placeholder}
        autoComplete={field.kind === 'password' ? 'new-password' : undefined}
        aria-invalid={Boolean(error)}
        aria-describedby={errorId}
        aria-label={field.label}
        className="console-worker-config-control"
      />
    )
  }

  return (
    <SettingsRow
      data-field={path[0]}
      data-path={path.join('.')}
      label={field.label}
      description={field.description}
      meta={
        error ? (
          <span id={errorId} className="console-worker-config-error">
            {error}
          </span>
        ) : undefined
      }
      control={control}
      layout={field.kind === 'switch' ? 'inline' : 'auto'}
    />
  )
}

function StringListField({
  field,
  basePath = [],
  ...context
}: FieldRenderContext & {
  field: StringListFieldSpec
  basePath?: ConfigPath
}) {
  const path = joinPath(basePath, field.path)
  const raw = atPath(context.root, path)
  const values = Array.isArray(raw) ? raw : []
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined
  const listRef = useRef<HTMLDivElement | null>(null)
  const pendingFocusIndex = useRef<number | null>(null)

  const update = (next: JsonValue[]) => context.onChange(setAtPath(context.root, path, next))

  useEffect(() => {
    const index = pendingFocusIndex.current
    if (index === null || index >= values.length) return
    pendingFocusIndex.current = null
    const frame = window.requestAnimationFrame(() => {
      listRef.current
        ?.querySelector<HTMLElement>(
          `[data-string-list-index="${index}"] input, [data-string-list-index="${index}"] button`,
        )
        ?.focus()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [values.length])

  const add = () => {
    pendingFocusIndex.current = values.length
    update([...values, field.options?.[0]?.value ?? ''])
  }

  return (
    // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
    <div className="console-worker-config-composite" role="listitem" data-field={path[0]} data-path={path.join('.')}>
      <SettingsRow
        label={field.label}
        description={field.description}
        meta={
          error ? (
            <span id={errorId} className="console-worker-config-error">
              {error}
            </span>
          ) : undefined
        }
        action={
          <Button type="button" variant="pill" size="sm" aria-describedby={errorId} onClick={add}>
            {field.addLabel ?? `Add ${field.itemLabel ?? 'item'}`}
          </Button>
        }
        layout="inline"
      />
      {values.length > 0 ? (
        <SettingsList ref={listRef} className="console-worker-config-sublist console-worker-config-string-list">
          {values.map((entry, index) => {
            const label = `${field.itemLabel ?? field.label} ${index + 1}`
            const knownOption = typeof entry === 'string' && field.options?.some((option) => option.value === entry)
            return (
              <SettingsRow
                key={index}
                className="console-worker-config-string-list-row"
                data-string-list-index={index}
                data-path={[...path, String(index)].join('.')}
                label={<span className="console-worker-config-visually-hidden">{label}</span>}
                control={
                  typeof entry === 'string' ? (
                    knownOption && field.options ? (
                      <Select
                        value={entry}
                        options={field.options.map(({ value, label: optionLabel, description }) => ({
                          value,
                          label: optionLabel,
                          title: description,
                        }))}
                        onChange={(next) => {
                          const copy = [...values]
                          copy[index] = next
                          update(copy)
                        }}
                        aria-label={label}
                        aria-invalid={Boolean(error)}
                        aria-describedby={errorId}
                        className="console-worker-config-select console-worker-config-string-list-input"
                      />
                    ) : (
                      <Input
                        id={fieldId([...path, String(index)])}
                        name={[...path, String(index)].join('.')}
                        value={entry}
                        onChange={(next) => {
                          const copy = [...values]
                          copy[index] = next
                          update(copy)
                        }}
                        placeholder={field.placeholder}
                        spellCheck={false}
                        autoComplete="off"
                        aria-label={label}
                        aria-invalid={Boolean(error)}
                        aria-describedby={errorId}
                        className="console-worker-config-control console-worker-config-string-list-input"
                      />
                    )
                  ) : (
                    <span className="console-worker-config-preserved">Structured entry preserved</span>
                  )
                }
                action={
                  <IconButton
                    label={`Remove ${label}`}
                    tooltipSide="left"
                    onClick={() => update(values.filter((_, item) => item !== index))}
                    className="console-worker-config-string-list-remove"
                  >
                    <Trash2 aria-hidden />
                  </IconButton>
                }
                layout="stacked"
              />
            )
          })}
        </SettingsList>
      ) : null}
    </div>
  )
}

type DynamicScalarKind = 'string' | 'number' | 'boolean' | 'structured'

function dynamicKind(value: JsonValue): DynamicScalarKind {
  if (typeof value === 'number') return 'number'
  if (typeof value === 'boolean') return 'boolean'
  if (typeof value === 'string' || value === null) return 'string'
  return 'structured'
}

function DynamicMapField({
  field,
  basePath = [],
  ...context
}: FieldRenderContext & {
  field: DynamicMapFieldSpec
  basePath?: ConfigPath
}) {
  const path = joinPath(basePath, field.path)
  const rawValue = atPath(context.root, path)
  const value = asObject(rawValue)
  const entries = Object.entries(value)
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined
  const [kindOverrides, setKindOverrides] = useState<Record<string, DynamicScalarKind>>({})

  const update = (next: JsonObject) => context.onChange(setAtPath(context.root, path, next))
  const add = () => {
    let index = entries.length + 1
    let key = `key_${index}`
    while (Object.hasOwn(value, key)) key = `key_${++index}`
    update({ ...value, [key]: '' })
  }
  const rename = (oldKey: string, newKey: string) => {
    if (newKey !== oldKey && Object.hasOwn(value, newKey)) return
    const next: JsonObject = {}
    for (const [key, entry] of entries) {
      next[key === oldKey ? newKey : key] = entry
    }
    setKindOverrides((current) => {
      if (!(oldKey in current) || oldKey === newKey) return current
      const nextOverrides = { ...current, [newKey]: current[oldKey] }
      delete nextOverrides[oldKey]
      return nextOverrides
    })
    update(next)
  }

  if (rawValue !== undefined && rawValue !== null && !isObject(rawValue)) {
    const kind = Array.isArray(rawValue) ? 'list' : typeof rawValue
    return (
      // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
      <div className="console-worker-config-composite" role="listitem" data-field={path[0]} data-path={path.join('.')}>
        <SettingsRow
          label={field.label}
          description={field.description}
          meta={`Existing ${kind} value is preserved unchanged.`}
          action={
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => context.onChange(setAtPath(context.root, path, {}))}
            >
              Use key/value settings
            </Button>
          }
          layout="inline"
        />
      </div>
    )
  }

  return (
    // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
    <div className="console-worker-config-composite" role="listitem" data-field={path[0]} data-path={path.join('.')}>
      <SettingsRow
        label={field.label}
        description={field.description}
        meta={
          error ? (
            <span id={errorId} className="console-worker-config-error">
              {error}
            </span>
          ) : undefined
        }
        action={
          <Button type="button" variant="pill" size="sm" aria-describedby={errorId} onClick={add}>
            {field.addLabel ?? 'Add value'}
          </Button>
        }
        layout="inline"
      />
      {entries.length > 0 ? (
        <SettingsList className="console-worker-config-map-list">
          {entries.map(([key, entry], index) => {
            const kind = kindOverrides[key] ?? dynamicKind(entry)
            const secret = field.secretKeys?.includes(key) || /(?:token|secret|password|api[_-]?key)/i.test(key)
            return (
              <SettingsRow
                key={index}
                label={
                  <Input
                    value={key}
                    onChange={(next) => rename(key, next)}
                    aria-label={field.keyLabel ?? 'Key'}
                    aria-invalid={Boolean(error)}
                    aria-describedby={errorId}
                    className="console-worker-config-map-key"
                  />
                }
                meta={kind === 'structured' ? 'Structured value is preserved.' : undefined}
                control={
                  <div className="console-worker-config-map-control">
                    <Select
                      value={kind}
                      options={[
                        { value: 'string', label: 'Text' },
                        { value: 'number', label: 'Number' },
                        { value: 'boolean', label: 'Switch' },
                        ...(kind === 'structured' ? [{ value: 'structured' as const, label: 'Structured' }] : []),
                      ]}
                      onChange={(next) => {
                        setKindOverrides((current) => ({
                          ...current,
                          [key]: next,
                        }))
                        if (next === 'string') update({ ...value, [key]: '' })
                        if (next === 'number') update({ ...value, [key]: 0 })
                        if (next === 'boolean') update({ ...value, [key]: false })
                      }}
                      disabled={kind === 'structured'}
                      aria-label={`${key} value type`}
                      aria-invalid={Boolean(error)}
                      aria-describedby={errorId}
                      className="console-worker-config-map-type"
                    />
                    {kind === 'boolean' ? (
                      <Switch
                        checked={entry === true}
                        onChange={(event) => update({ ...value, [key]: event.currentTarget.checked })}
                        aria-label={`${key} value`}
                        aria-invalid={Boolean(error)}
                        aria-describedby={errorId}
                      />
                    ) : kind === 'structured' ? (
                      <span className="console-worker-config-preserved">Preserved</span>
                    ) : (
                      <Input
                        type={secret ? 'password' : kind === 'number' ? 'number' : 'text'}
                        value={displayValue(entry)}
                        onChange={(next) => {
                          if (kind === 'number') {
                            setKindOverrides((current) => ({
                              ...current,
                              [key]: 'number',
                            }))
                          }
                          update({
                            ...value,
                            [key]:
                              kind === 'number' && next !== '' && Number.isFinite(Number(next)) ? Number(next) : next,
                          })
                        }}
                        onBlur={() => {
                          if (kind !== 'number' || typeof entry !== 'number') return
                          setKindOverrides((current) => {
                            const next = { ...current }
                            delete next[key]
                            return next
                          })
                        }}
                        aria-label={`${key} value`}
                        aria-invalid={Boolean(error)}
                        aria-describedby={errorId}
                        className="console-worker-config-control"
                      />
                    )}
                  </div>
                }
                action={
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      const next = { ...value }
                      delete next[key]
                      setKindOverrides((current) => {
                        const nextOverrides = { ...current }
                        delete nextOverrides[key]
                        return nextOverrides
                      })
                      update(next)
                    }}
                    aria-label={`Remove ${key}`}
                  >
                    Remove
                  </Button>
                }
                layout="stacked"
              />
            )
          })}
        </SettingsList>
      ) : null}
    </div>
  )
}

function FilterListField({
  field,
  basePath = [],
  ...context
}: FieldRenderContext & {
  field: FilterListFieldSpec
  basePath?: ConfigPath
}) {
  const path = joinPath(basePath, field.path)
  const raw = atPath(context.root, path)
  const filters = Array.isArray(raw) ? raw : []
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined
  const update = (next: JsonValue[]) => context.onChange(setAtPath(context.root, path, next))

  return (
    // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
    <div className="console-worker-config-composite" role="listitem" data-field={path[0]} data-path={path.join('.')}>
      <SettingsRow
        label={field.label}
        description={field.description}
        meta={
          error ? (
            <span id={errorId} className="console-worker-config-error">
              {error}
            </span>
          ) : undefined
        }
        action={
          <Button
            type="button"
            variant="pill"
            size="sm"
            aria-describedby={errorId}
            onClick={() => update([...filters, 'match("*")'])}
          >
            Add filter
          </Button>
        }
        layout="inline"
      />
      {filters.length > 0 ? (
        <SettingsList className="console-worker-config-sublist">
          {filters.map((filter, index) => {
            const metadata = isObject(filter) && isObject(filter.metadata)
            const kind = metadata ? 'metadata' : 'match'
            return (
              <SettingsRow
                key={index}
                label={
                  <Select
                    value={kind}
                    options={[
                      { value: 'match', label: 'Function match' },
                      { value: 'metadata', label: 'Metadata' },
                    ]}
                    onChange={(next) => {
                      const copy = [...filters]
                      copy[index] = next === 'metadata' ? { metadata: {} } : 'match("*")'
                      update(copy)
                    }}
                    aria-label={`Filter ${index + 1} type`}
                    aria-invalid={Boolean(error)}
                    aria-describedby={errorId}
                    className="console-worker-config-map-type"
                  />
                }
                control={
                  kind === 'match' ? (
                    <Input
                      value={typeof filter === 'string' ? filter : 'match("*")'}
                      onChange={(next) => {
                        const copy = [...filters]
                        copy[index] = next
                        update(copy)
                      }}
                      placeholder={'match("api::*")'}
                      aria-label={`Filter ${index + 1} pattern`}
                      aria-invalid={Boolean(error)}
                      aria-describedby={errorId}
                      className="console-worker-config-control"
                    />
                  ) : (
                    <div className="console-worker-config-inline-map">
                      <DynamicMapField
                        field={{
                          kind: 'dynamic-map',
                          path: ['metadata'],
                          label: 'Metadata conditions',
                          addLabel: 'Add condition',
                        }}
                        basePath={[...path, String(index)]}
                        {...context}
                      />
                    </div>
                  )
                }
                action={
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => update(filters.filter((_, item) => item !== index))}
                  >
                    Remove
                  </Button>
                }
                layout="stacked"
              />
            )
          })}
        </SettingsList>
      ) : null}
    </div>
  )
}

const PERMISSION_MODES = ['manual', 'auto', 'full'] as const

const STRUCTURED_VALUE_TYPES: ReadonlyArray<{
  value: StructuredValueKind
  label: string
}> = [
  { value: 'object', label: 'Key/value group' },
  { value: 'list', label: 'List' },
  { value: 'string', label: 'Text' },
  { value: 'number', label: 'Number' },
  { value: 'boolean', label: 'Switch' },
  { value: 'null', label: 'Null' },
]

function StructuredValueField({
  field,
  basePath = [],
  ...context
}: FieldRenderContext & {
  field: StructuredValueFieldSpec
  basePath?: ConfigPath
}) {
  const path = joinPath(basePath, field.path)
  const value = atPath(context.root, path)
  const present = value !== undefined
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined

  return (
    // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
    <div
      className="console-worker-config-composite console-worker-config-structured"
      role="listitem"
      data-field={path[0]}
      data-path={path.join('.')}
    >
      <SettingsRow
        label={field.label}
        description={field.description}
        meta={
          error ? (
            <span id={errorId} className="console-worker-config-error">
              {error}
            </span>
          ) : undefined
        }
        action={
          present ? (
            field.optional ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => context.onChange(deleteAtPath(context.root, path))}
              >
                Remove settings
              </Button>
            ) : undefined
          ) : (
            <Button
              type="button"
              variant="pill"
              size="sm"
              aria-describedby={errorId}
              onClick={() => context.onChange(setAtPath(context.root, path, {}))}
            >
              {field.addLabel ?? 'Add settings'}
            </Button>
          )
        }
        layout="inline"
      />
      {present ? (
        <div className="console-worker-config-structured-root">
          <StructuredValueEditor
            value={value}
            label={field.label}
            secretKeys={field.secretKeys ?? []}
            error={error}
            errorId={errorId}
            onChange={(next) => context.onChange(setAtPath(context.root, path, next))}
          />
        </div>
      ) : (
        <div className="console-worker-config-empty">No adapter-specific settings.</div>
      )}
    </div>
  )
}

function StructuredValueEditor({
  value,
  label,
  propertyName,
  secretKeys,
  error,
  errorId,
  onChange,
}: {
  value: JsonValue
  label: string
  propertyName?: string
  secretKeys: readonly string[]
  error?: string
  errorId?: string
  onChange(next: JsonValue): void
}) {
  const kind = structuredValueKind(value)
  const [numberDraft, setNumberDraft] = useState(() => (typeof value === 'number' ? String(value) : ''))

  useEffect(() => {
    if (typeof value === 'number') setNumberDraft(String(value))
  }, [value])

  const secret =
    propertyName !== undefined &&
    (secretKeys.includes(propertyName) || /(?:token|secret|password|api[_-]?key)/i.test(propertyName))

  const scalarControl =
    kind === 'string' ? (
      <Input
        type={secret ? 'password' : 'text'}
        value={value as string}
        onChange={onChange}
        autoComplete={secret ? 'new-password' : undefined}
        aria-label={`${label} value`}
        aria-invalid={Boolean(error)}
        aria-describedby={errorId}
        className="console-worker-config-control"
      />
    ) : kind === 'number' ? (
      <Input
        type="number"
        value={numberDraft}
        onChange={(next) => {
          setNumberDraft(next)
          if (next.trim() === '' || !Number.isFinite(Number(next))) return
          onChange(Number(next))
        }}
        onBlur={() => {
          if (numberDraft.trim() !== '' && Number.isFinite(Number(numberDraft))) return
          setNumberDraft(String(value))
        }}
        aria-label={`${label} value`}
        aria-invalid={Boolean(error)}
        aria-describedby={errorId}
        className="console-worker-config-control console-worker-config-number"
      />
    ) : kind === 'boolean' ? (
      <Switch
        checked={value as boolean}
        onChange={(event) => onChange(event.currentTarget.checked)}
        aria-label={`${label} value`}
        aria-invalid={Boolean(error)}
        aria-describedby={errorId}
      />
    ) : kind === 'null' ? (
      <span className="console-worker-config-preserved">Explicit null value</span>
    ) : null

  return (
    <div className="console-worker-config-structured-value" data-kind={kind}>
      <div className="console-worker-config-structured-toolbar">
        <Select
          value={kind}
          options={[...STRUCTURED_VALUE_TYPES]}
          onChange={(next) => onChange(emptyStructuredValue(next))}
          aria-label={`${label} value type`}
          aria-invalid={Boolean(error)}
          aria-describedby={errorId}
          className="console-worker-config-map-type"
        />
        {scalarControl}
        {kind === 'object' ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              const object = asObject(value)
              let index = Object.keys(object).length + 1
              let key = `field_${index}`
              while (Object.hasOwn(object, key)) key = `field_${++index}`
              onChange({ ...object, [key]: '' })
            }}
          >
            Add field
          </Button>
        ) : null}
        {kind === 'list' ? (
          <Button type="button" variant="ghost" size="sm" onClick={() => onChange([...(value as JsonValue[]), ''])}>
            Add item
          </Button>
        ) : null}
      </div>
      {kind === 'object' ? (
        <StructuredObjectEditor value={asObject(value)} label={label} secretKeys={secretKeys} onChange={onChange} />
      ) : null}
      {kind === 'list' ? (
        <StructuredListEditor value={value as JsonValue[]} label={label} secretKeys={secretKeys} onChange={onChange} />
      ) : null}
    </div>
  )
}

function StructuredObjectEditor({
  value,
  label,
  secretKeys,
  onChange,
}: {
  value: JsonObject
  label: string
  secretKeys: readonly string[]
  onChange(next: JsonValue): void
}) {
  const entries = Object.entries(value)
  if (entries.length === 0) {
    return <div className="console-worker-config-empty">No fields.</div>
  }
  return (
    <SettingsList className="console-worker-config-sublist console-worker-config-structured-list">
      {entries.map(([key, entry], index) => (
        <SettingsRow
          key={index}
          label={
            <Input
              value={key}
              onChange={(next) => onChange(renameStructuredKey(value, key, next))}
              aria-label={`${label} field name`}
              className="console-worker-config-map-key"
            />
          }
          control={
            <StructuredValueEditor
              value={entry}
              label={key || `Field ${index + 1}`}
              propertyName={key}
              secretKeys={secretKeys}
              onChange={(next) => onChange({ ...value, [key]: next })}
            />
          }
          action={
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label={`Remove ${key || `field ${index + 1}`}`}
              onClick={() => {
                const next = { ...value }
                delete next[key]
                onChange(next)
              }}
            >
              Remove
            </Button>
          }
          layout="stacked"
        />
      ))}
    </SettingsList>
  )
}

function StructuredListEditor({
  value,
  label,
  secretKeys,
  onChange,
}: {
  value: JsonValue[]
  label: string
  secretKeys: readonly string[]
  onChange(next: JsonValue): void
}) {
  if (value.length === 0) {
    return <div className="console-worker-config-empty">No items.</div>
  }
  return (
    <SettingsList className="console-worker-config-sublist console-worker-config-structured-list">
      {value.map((entry, index) => (
        <SettingsRow
          key={index}
          label={`Item ${index + 1}`}
          control={
            <StructuredValueEditor
              value={entry}
              label={`${label} item ${index + 1}`}
              secretKeys={secretKeys}
              onChange={(next) => {
                const copy = [...value]
                copy[index] = next
                onChange(copy)
              }}
            />
          }
          action={
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => onChange(value.filter((_, candidate) => candidate !== index))}
            >
              Remove
            </Button>
          }
          layout="stacked"
        />
      ))}
    </SettingsList>
  )
}

function PermissionRulesField({
  field,
  basePath = [],
  ...context
}: FieldRenderContext & {
  field: PermissionRulesFieldSpec
  basePath?: ConfigPath
}) {
  const path = joinPath(basePath, field.path)
  const raw = atPath(context.root, path)
  const rules = Array.isArray(raw) ? raw : []
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined
  const update = (next: JsonValue[]) => context.onChange(setAtPath(context.root, path, next))

  return (
    // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
    <div
      className="console-worker-config-composite console-worker-config-rule-stack"
      role="listitem"
      data-field={path[0]}
      data-path={path.join('.')}
    >
      <SettingsRow
        label={field.label}
        description={field.description}
        meta={
          error ? (
            <span id={errorId} className="console-worker-config-error">
              {error}
            </span>
          ) : undefined
        }
        action={
          <Button
            type="button"
            variant="pill"
            size="sm"
            aria-describedby={errorId}
            onClick={() => update([...rules, 'worker::*'])}
          >
            {field.addLabel ?? 'Add rule'}
          </Button>
        }
        layout="inline"
      />
      {rules.length === 0 ? (
        <div className="console-worker-config-empty">No permission rules. Calls without a match require approval.</div>
      ) : (
        rules.map((rule, index) => (
          <PermissionRuleEditor
            key={index}
            index={index}
            rule={rule}
            onChange={(nextRule) => {
              const next = [...rules]
              next[index] = nextRule
              update(next)
            }}
            onRemove={() => update(rules.filter((_, candidate) => candidate !== index))}
          />
        ))
      )}
    </div>
  )
}

function PermissionRuleEditor({
  index,
  rule,
  onChange,
  onRemove,
}: {
  index: number
  rule: JsonValue
  onChange(next: JsonValue): void
  onRemove(): void
}) {
  const structured = isObject(rule)
  const object = asObject(rule)
  const modes = Array.isArray(object.modes)
    ? object.modes.filter((mode): mode is string => typeof mode === 'string')
    : []
  const functionPattern = typeof object.function === 'string' ? object.function : ''
  const action = object.action === 'deny' ? 'deny' : 'allow'
  const setObject = (key: string, nextValue: JsonValue | undefined) => {
    const next = { ...object }
    if (nextValue === undefined) delete next[key]
    else next[key] = nextValue
    onChange(next)
  }

  return (
    <SettingsSection
      className="console-worker-config-nested console-worker-config-rule"
      role="listitem"
      title={`Rule ${index + 1}`}
      description={structured ? functionPattern || 'Advanced permission rule' : String(rule || 'Function shorthand')}
      action={
        <Button type="button" variant="ghost" size="sm" onClick={onRemove} aria-label={`Remove rule ${index + 1}`}>
          Remove
        </Button>
      }
    >
      <SettingsList>
        <SettingsRow
          label="Rule type"
          control={
            <Select
              value={structured ? 'advanced' : 'shorthand'}
              options={[
                { value: 'shorthand', label: 'Shorthand' },
                { value: 'advanced', label: 'Advanced' },
              ]}
              onChange={(next) => {
                if (next === 'advanced' && !structured) {
                  const shorthand = typeof rule === 'string' ? rule : ''
                  onChange({
                    function: shorthand.startsWith('!') ? shorthand.slice(1) : shorthand || 'worker::*',
                    action: shorthand.startsWith('!') ? 'deny' : 'allow',
                  })
                } else if (next === 'shorthand' && structured) {
                  onChange(`${action === 'deny' ? '!' : ''}${functionPattern || 'worker::*'}`)
                }
              }}
              aria-label={`Rule ${index + 1} type`}
              className="console-worker-config-select"
            />
          }
        />
        {!structured ? (
          <SettingsRow
            label="Function pattern"
            description="Prefix with ! to deny. Globs such as shell::* are supported."
            control={
              <Input
                value={typeof rule === 'string' ? rule : ''}
                onChange={onChange}
                placeholder="worker::*"
                aria-label={`Rule ${index + 1} function pattern`}
                className="console-worker-config-control"
              />
            }
          />
        ) : (
          <>
            <SettingsRow
              label="Function pattern"
              control={
                <Input
                  value={functionPattern}
                  onChange={(next) => setObject('function', next)}
                  placeholder="worker::*"
                  aria-label={`Rule ${index + 1} function pattern`}
                  className="console-worker-config-control"
                />
              }
            />
            <SettingsRow
              label="Decision"
              control={
                <Select
                  value={action}
                  options={[
                    { value: 'allow', label: 'Allow' },
                    { value: 'deny', label: 'Deny' },
                  ]}
                  onChange={(next) => setObject('action', next)}
                  aria-label={`Rule ${index + 1} decision`}
                  className="console-worker-config-select"
                />
              }
            />
            <SettingsRow
              label="Rule ID"
              description="Optional identifier shown when this rule makes a decision."
              control={
                <Input
                  value={typeof object.rule_id === 'string' ? object.rule_id : ''}
                  onChange={(next) => setObject('rule_id', next || undefined)}
                  aria-label={`Rule ${index + 1} ID`}
                  className="console-worker-config-control"
                />
              }
            />
            <SettingsRow
              label="Permission modes"
              description="No selected modes means the rule applies in every mode."
              layout="stacked"
              control={
                <div className="console-worker-config-rule-modes">
                  {PERMISSION_MODES.map((mode) => (
                    <div key={mode} className="console-worker-config-rule-mode">
                      <Switch
                        checked={modes.includes(mode)}
                        onChange={(event) => {
                          const nextModes = event.currentTarget.checked
                            ? [...new Set([...modes, mode])]
                            : modes.filter((candidate) => candidate !== mode)
                          setObject('modes', nextModes.length > 0 ? nextModes : undefined)
                        }}
                        aria-label={`${mode} mode`}
                      />
                      <span>{mode}</span>
                    </div>
                  ))}
                </div>
              }
            />
            <PermissionArguments
              value={asObject(object.args)}
              onChange={(next) => setObject('args', Object.keys(next).length > 0 ? next : undefined)}
            />
          </>
        )}
      </SettingsList>
    </SettingsSection>
  )
}

function PermissionArguments({ value, onChange }: { value: JsonObject; onChange(next: JsonObject): void }) {
  const entries = Object.entries(value)
  const rename = (from: string, to: string) => {
    if (to !== from && Object.hasOwn(value, to)) return
    const next: JsonObject = {}
    for (const [key, constraint] of entries) {
      next[key === from ? to : key] = constraint
    }
    onChange(next)
  }

  return (
    // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
    <div className="console-worker-config-composite" role="listitem">
      <SettingsRow
        label="Argument constraints"
        description="Match a payload field by exact value or regular expression."
        action={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              let index = entries.length + 1
              let key = `field_${index}`
              while (Object.hasOwn(value, key)) key = `field_${++index}`
              onChange({ ...value, [key]: { equals: '' } })
            }}
          >
            Add constraint
          </Button>
        }
        layout="inline"
      />
      {entries.length > 0 ? (
        <SettingsList className="console-worker-config-sublist">
          {entries.map(([key, constraint], index) => (
            <PermissionConstraint
              key={index}
              fieldName={key}
              value={constraint}
              onRename={(next) => rename(key, next)}
              onChange={(nextConstraint) => onChange({ ...value, [key]: nextConstraint })}
              onRemove={() => {
                const next = { ...value }
                delete next[key]
                onChange(next)
              }}
            />
          ))}
        </SettingsList>
      ) : null}
    </div>
  )
}

function PermissionConstraint({
  fieldName,
  value,
  onRename,
  onChange,
  onRemove,
}: {
  fieldName: string
  value: JsonValue
  onRename(next: string): void
  onChange(next: JsonValue): void
  onRemove(): void
}) {
  const constraint = asObject(value)
  const type = typeof constraint.matches === 'string' ? 'matches' : 'equals'
  const equalsValue = constraint.equals
  const [equalsKind, setEqualsKind] = useState<DynamicScalarKind>(() => dynamicKind(equalsValue ?? ''))

  return (
    <SettingsRow
      label={
        <Input
          value={fieldName}
          onChange={onRename}
          aria-label="Argument field"
          className="console-worker-config-map-key"
        />
      }
      layout="stacked"
      control={
        <div className="console-worker-config-constraint-control">
          <Select
            value={type}
            options={[
              { value: 'equals', label: 'Equals' },
              { value: 'matches', label: 'Matches pattern' },
            ]}
            onChange={(next) => onChange(next === 'matches' ? { matches: '' } : { equals: '' })}
            aria-label={`${fieldName} constraint type`}
            className="console-worker-config-map-type"
          />
          {type === 'matches' ? (
            <Input
              value={String(constraint.matches ?? '')}
              onChange={(next) => onChange({ ...constraint, matches: next })}
              placeholder="^value$"
              aria-label={`${fieldName} pattern`}
              className="console-worker-config-control"
            />
          ) : equalsKind === 'boolean' ? (
            <Switch
              checked={equalsValue === true}
              onChange={(event) => onChange({ ...constraint, equals: event.currentTarget.checked })}
              aria-label={`${fieldName} expected value`}
            />
          ) : equalsKind === 'structured' ? (
            <span className="console-worker-config-preserved">Structured equality value preserved</span>
          ) : (
            <Input
              type={equalsKind === 'number' ? 'number' : 'text'}
              value={displayValue(equalsValue)}
              onChange={(next) => {
                const nextValue =
                  equalsKind === 'number' && next !== '' && Number.isFinite(Number(next)) ? Number(next) : next
                onChange({ ...constraint, equals: nextValue })
              }}
              aria-label={`${fieldName} expected value`}
              className="console-worker-config-control"
            />
          )}
          {type === 'equals' ? (
            <Select
              value={equalsKind}
              options={[
                { value: 'string', label: 'Text' },
                { value: 'number', label: 'Number' },
                { value: 'boolean', label: 'Switch' },
                ...(equalsKind === 'structured'
                  ? [
                      {
                        value: 'structured' as const,
                        label: 'Structured',
                      },
                    ]
                  : []),
              ]}
              onChange={(next) => {
                setEqualsKind(next)
                if (next === 'string') onChange({ ...constraint, equals: '' })
                if (next === 'number') onChange({ ...constraint, equals: 0 })
                if (next === 'boolean') onChange({ ...constraint, equals: false })
              }}
              disabled={equalsKind === 'structured'}
              aria-label={`${fieldName} expected value type`}
              className="console-worker-config-map-type"
            />
          ) : null}
        </div>
      }
      action={
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={onRemove}
          aria-label={`Remove ${fieldName} constraint`}
        >
          Remove
        </Button>
      }
    />
  )
}

function ObjectField({ field, basePath = [], ...context }: FieldProps) {
  if (field.kind !== 'object') return null
  const path = joinPath(basePath, field.path)
  const current = atPath(context.root, path)
  const enabled = isObject(current)
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined

  return (
    <SettingsSection
      className="console-worker-config-nested"
      role="listitem"
      data-field={path[0]}
      data-path={path.join('.')}
      title={field.label}
      description={field.description}
      action={
        field.optional ? (
          <Switch
            checked={enabled}
            onChange={(event) => {
              context.onChange(
                event.currentTarget.checked
                  ? setAtPath(context.root, path, cloneJson(field.defaultValue ?? {}))
                  : field.disabledValue !== undefined
                    ? setAtPath(context.root, path, cloneJson(field.disabledValue))
                    : deleteAtPath(context.root, path),
              )
            }}
            aria-label={`Configure ${field.label}`}
            aria-invalid={Boolean(error)}
            aria-describedby={errorId}
          />
        ) : undefined
      }
    >
      {error ? (
        <div id={errorId} className="console-worker-config-error">
          {error}
        </div>
      ) : null}
      {enabled || !field.optional ? (
        <SettingsList>
          {field.fields.map((child, index) => (
            <FieldRenderer
              key={`${child.kind}-${child.path.join('.')}-${index}`}
              field={child}
              basePath={path}
              {...context}
            />
          ))}
        </SettingsList>
      ) : null}
    </SettingsSection>
  )
}

function VariantField({
  field,
  basePath = [],
  ...context
}: FieldRenderContext & { field: VariantFieldSpec; basePath?: ConfigPath }) {
  const path = joinPath(basePath, field.path)
  const discriminator = field.discriminator ?? 'name'
  const contentKey = field.contentKey ?? 'config'
  const object = asObject(atPath(context.root, path))
  const hasDiscriminator = typeof object[discriminator] === 'string'
  const selected = hasDiscriminator ? (object[discriminator] as string) : field.options[0]?.value
  const option = field.options.find((candidate) => candidate.value === selected)
  const unknownOption = hasDiscriminator && option === undefined
  const hasContent = Object.hasOwn(object, contentKey)
  const content = object[contentKey]
  const usesStructuredFallback = unknownOption || (hasContent && !isObject(content))
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined
  const optionDefaults = asObject(cloneJson(option?.defaultValue ?? {}))
  // An absent optional variant still displays its effective default. Give its
  // child controls a materialized draft root so the first edit writes the
  // discriminator and defaults together instead of producing `{config: …}`
  // without the required `name`.
  const childRoot =
    option && !hasDiscriminator && !usesStructuredFallback
      ? setAtPath(context.root, path, {
          ...optionDefaults,
          ...object,
          [discriminator]: option.value,
          [contentKey]: {
            ...asObject(optionDefaults[contentKey]),
            ...asObject(object[contentKey]),
          },
        })
      : context.root
  const selectOptions = field.options.map(({ value, label, description }) => ({
    value,
    label,
    title: description,
  }))
  if (unknownOption) {
    selectOptions.unshift({
      value: selected,
      label: `Unrecognized: ${selected}`,
      title: 'This adapter is not recognized by this console version.',
    })
  }

  return (
    <SettingsSection
      className="console-worker-config-nested"
      role="listitem"
      data-field={path[0]}
      data-path={path.join('.')}
      title={field.label}
      description={field.description}
    >
      <SettingsList>
        <SettingsRow
          label="Type"
          description={
            option?.description ??
            (unknownOption
              ? `“${selected}” is not recognized by this console version. Its saved value is preserved.`
              : undefined)
          }
          meta={
            error ? (
              <span id={errorId} className="console-worker-config-error">
                {error}
              </span>
            ) : undefined
          }
          control={
            <Select
              value={selected}
              options={selectOptions}
              onChange={(next) => {
                const nextOption = field.options.find((candidate) => candidate.value === next)
                if (!nextOption) return
                context.onChange(
                  setAtPath(
                    context.root,
                    path,
                    selectVariantValue(
                      object,
                      discriminator,
                      contentKey,
                      next,
                      asObject(cloneJson(nextOption.defaultValue)),
                      field.options.flatMap((candidate) => candidate.fields.map((declared) => declared.path)),
                      hasDiscriminator && !unknownOption,
                    ),
                  ),
                )
              }}
              aria-label={`${field.label} type`}
              aria-invalid={Boolean(error)}
              aria-describedby={errorId}
              className="console-worker-config-select"
            />
          }
        />
        {usesStructuredFallback ? (
          // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
          <div className="console-worker-config-composite" role="listitem">
            <SettingsRow
              role="presentation"
              label="Adapter settings"
              description={
                unknownOption
                  ? 'These settings belong to an adapter this console does not recognize. You can edit them without changing their JSON shape.'
                  : `This saved value is not an object, so ${option?.label ?? 'the adapter'} fields cannot be shown. Edit it below or replace it explicitly.`
              }
              action={
                option ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      const defaultContent = Object.hasOwn(optionDefaults, contentKey)
                        ? optionDefaults[contentKey]
                        : {}
                      context.onChange(setAtPath(context.root, [...path, contentKey], defaultContent))
                    }}
                  >
                    Use {option.label} defaults
                  </Button>
                ) : !hasContent ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => context.onChange(setAtPath(context.root, [...path, contentKey], {}))}
                  >
                    Add settings
                  </Button>
                ) : undefined
              }
              layout="inline"
            />
            {hasContent ? (
              <div className="console-worker-config-structured-root">
                <StructuredValueEditor
                  value={content}
                  label={`${field.label} settings`}
                  secretKeys={[]}
                  error={error}
                  errorId={errorId}
                  onChange={(next) => context.onChange(setAtPath(context.root, [...path, contentKey], next))}
                />
              </div>
            ) : (
              <div className="console-worker-config-empty">No adapter-specific settings value is set.</div>
            )}
          </div>
        ) : (
          option?.fields.map((child, index) => (
            <FieldRenderer
              key={`${child.kind}-${child.path.join('.')}-${index}`}
              field={child}
              basePath={path}
              {...context}
              root={childRoot}
            />
          ))
        )}
      </SettingsList>
    </SettingsSection>
  )
}

function itemSummary(value: JsonValue, paths: readonly ConfigPath[] | undefined, fallback: string): string {
  for (const path of paths ?? []) {
    const candidate = atPath(value, path)
    if (typeof candidate === 'string' && candidate.trim()) return candidate
    if (typeof candidate === 'number') return String(candidate)
  }
  return fallback
}

function ObjectListField({
  field,
  basePath = [],
  ...context
}: FieldRenderContext & {
  field: ObjectCollectionFieldSpec
  basePath?: ConfigPath
}) {
  const path = joinPath(basePath, field.path)
  const raw = atPath(context.root, path)
  const items = Array.isArray(raw) ? raw : []
  const [selected, setSelected] = useState<number | null>(null)
  const active = selected !== null && selected < items.length ? selected : null
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined
  const update = (next: JsonValue[]) => context.onChange(setAtPath(context.root, path, next))

  return (
    // biome-ignore lint/a11y/useSemanticElements: SettingsList is an embeddable ARIA list rather than a semantic ul.
    <div
      className="console-worker-config-collection"
      role="listitem"
      data-selected={active === null ? 'false' : 'true'}
      data-field={path[0]}
      data-path={path.join('.')}
    >
      <div className="console-worker-config-collection-header">
        <div>
          <div className="console-worker-config-collection-title">{field.label}</div>
          {field.description ? (
            <div className="console-worker-config-collection-description">{field.description}</div>
          ) : null}
          {error ? (
            <div id={errorId} className="console-worker-config-error">
              {error}
            </div>
          ) : null}
        </div>
        <Button
          type="button"
          variant="pill"
          size="sm"
          aria-describedby={errorId}
          onClick={() => {
            const next = [...items, cloneJson(field.createItem)]
            update(next)
            setSelected(next.length - 1)
          }}
        >
          {field.addLabel ?? `Add ${field.itemLabel}`}
        </Button>
      </div>
      <div className="console-worker-config-collection-body">
        <div className="console-worker-config-collection-list">
          {items.length === 0 ? (
            <div className="console-worker-config-empty">
              {field.emptyLabel ?? `No ${field.itemLabel.toLowerCase()}s configured.`}
            </div>
          ) : (
            <SettingsList>
              {items.map((item, index) => (
                <SettingsRow
                  key={index}
                  label={itemSummary(item, field.summaryPaths, `${field.itemLabel} ${index + 1}`)}
                  meta={`${index + 1} of ${items.length}`}
                  action={
                    <Button
                      type="button"
                      variant={active === index ? 'pill' : 'ghost'}
                      size="sm"
                      onClick={() => setSelected(index)}
                    >
                      Edit
                    </Button>
                  }
                  layout="inline"
                />
              ))}
            </SettingsList>
          )}
        </div>
        {active !== null ? (
          <div className="console-worker-config-collection-detail">
            <div className="console-worker-config-detail-actions">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => setSelected(null)}
                className="console-worker-config-back"
              >
                Back
              </Button>
              <span>{itemSummary(items[active], field.summaryPaths, `${field.itemLabel} ${active + 1}`)}</span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  update(items.filter((_, index) => index !== active))
                  setSelected(null)
                }}
              >
                Remove
              </Button>
            </div>
            <SettingsList>
              {field.fields.map((child, index) => (
                <FieldRenderer
                  key={`${child.kind}-${child.path.join('.')}-${index}`}
                  field={child}
                  basePath={[...path, String(active)]}
                  {...context}
                />
              ))}
            </SettingsList>
          </div>
        ) : null}
      </div>
    </div>
  )
}

function ObjectMapField({
  field,
  basePath = [],
  ...context
}: FieldRenderContext & { field: ObjectMapFieldSpec; basePath?: ConfigPath }) {
  const path = joinPath(basePath, field.path)
  const map = asObject(atPath(context.root, path))
  const keys = useMemo(() => Object.keys(map).sort(), [map])
  const [selected, setSelected] = useState<string | null>(null)
  const active = selected && Object.hasOwn(map, selected) ? selected : null
  const error = errorFor(context.errors, path)
  const errorId = error ? `${fieldId(path)}-error` : undefined
  const update = (next: JsonObject) => context.onChange(setAtPath(context.root, path, next))
  const add = () => {
    let index = keys.length + 1
    let key = `${field.itemLabel.toLowerCase().replaceAll(' ', '_')}_${index}`
    while (Object.hasOwn(map, key)) {
      key = `${field.itemLabel.toLowerCase().replaceAll(' ', '_')}_${++index}`
    }
    update({ ...map, [key]: cloneJson(field.createValue) })
    setSelected(key)
  }
  const rename = (nextKey: string) => {
    if (!active || !nextKey || (nextKey !== active && Object.hasOwn(map, nextKey))) {
      return
    }
    const next: JsonObject = {}
    for (const key of keys) next[key === active ? nextKey : key] = map[key]
    update(next)
    setSelected(nextKey)
  }

  return (
    <SettingsSection
      className="console-worker-config-collection console-worker-config-map-collection"
      role="listitem"
      data-selected={active === null ? 'false' : 'true'}
      data-field={path[0]}
      data-path={path.join('.')}
      title={field.label}
      description={field.description}
      action={
        <Button type="button" variant="pill" size="sm" aria-describedby={errorId} onClick={add}>
          {field.addLabel ?? `Add ${field.itemLabel}`}
        </Button>
      }
    >
      {error ? (
        <div id={errorId} className="console-worker-config-error">
          {error}
        </div>
      ) : null}
      <div className="console-worker-config-collection-body">
        <div className="console-worker-config-collection-list">
          {keys.length === 0 ? (
            <div className="console-worker-config-empty">
              {field.emptyLabel ?? `No ${field.itemLabel.toLowerCase()}s configured.`}
            </div>
          ) : (
            <SettingsList>
              {keys.map((key) => (
                <SettingsRow
                  key={key}
                  label={key}
                  description={itemSummary(map[key], field.summaryPaths, '') || undefined}
                  action={
                    <Button
                      type="button"
                      variant={active === key ? 'pill' : 'ghost'}
                      size="sm"
                      onClick={() => setSelected(key)}
                    >
                      Edit
                    </Button>
                  }
                  layout="inline"
                />
              ))}
            </SettingsList>
          )}
        </div>
        {active ? (
          <div className="console-worker-config-collection-detail">
            <div className="console-worker-config-detail-actions">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => setSelected(null)}
                className="console-worker-config-back"
              >
                Back
              </Button>
              <Input
                value={active}
                onChange={rename}
                aria-label={field.keyLabel ?? `${field.itemLabel} name`}
                className="console-worker-config-map-key"
              />
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  const next = { ...map }
                  delete next[active]
                  update(next)
                  setSelected(null)
                }}
              >
                Remove
              </Button>
            </div>
            <SettingsList>
              {field.fields.map((child, index) => (
                <FieldRenderer
                  key={`${child.kind}-${child.path.join('.')}-${index}`}
                  field={child}
                  basePath={[...path, active]}
                  {...context}
                />
              ))}
            </SettingsList>
          </div>
        ) : null}
      </div>
    </SettingsSection>
  )
}

export function FieldRenderer(props: FieldProps) {
  const { field } = props
  if (
    field.kind === 'text' ||
    field.kind === 'password' ||
    field.kind === 'number' ||
    field.kind === 'select' ||
    field.kind === 'switch'
  ) {
    return <ScalarField {...props} />
  }
  if (field.kind === 'string-list') return <StringListField {...props} field={field} />
  if (field.kind === 'dynamic-map') return <DynamicMapField {...props} field={field} />
  if (field.kind === 'filter-list') return <FilterListField {...props} field={field} />
  if (field.kind === 'permission-rules') {
    return <PermissionRulesField {...props} field={field} />
  }
  if (field.kind === 'structured-value') {
    return <StructuredValueField {...props} field={field} />
  }
  if (field.kind === 'object') return <ObjectField {...props} />
  if (field.kind === 'variant') return <VariantField {...props} field={field} />
  if (field.kind === 'object-list') return <ObjectListField {...props} field={field} />
  if (field.kind === 'object-map') return <ObjectMapField {...props} field={field} />
  return null
}
