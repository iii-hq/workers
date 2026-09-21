// @vitest-environment jsdom

import type { ConfigFormProps, SettingsFieldProps } from '@iii-dev/console-ui'
import { act, type InputHTMLAttributes, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { JevConfigForm } from './index'

const changes = vi.hoisted(() => new Map<string, (value: string) => void>())
const limits = [
  ['max_request_bytes', 8388608],
  ['max_response_bytes', 8388608],
  ['max_timeout_ms', 300000],
] as const

// The console provides these components via its import map. Mirror the public
// contract so the form can run without a live console or configuration store.
vi.mock('@iii-dev/console-ui', () => ({
  Input: ({
    onChange,
    ...props
  }: Omit<InputHTMLAttributes<HTMLInputElement>, 'onChange'> & {
    onChange: (value: string) => void
  }) => {
    changes.set(props.name!, onChange)
    return <input {...props} onChange={(event) => onChange(event.currentTarget.value)} />
  },
  SettingsField: ({ id, field, label, description, error, renderControl }: SettingsFieldProps) => (
    <div data-field={field}>
      <label htmlFor={id}>{label}</label>
      <span id={`${id}-description`}>{description}</span>
      {error ? <span id={`${id}-error`}>{error}</span> : null}
      {renderControl({
        id: id!,
        name: field,
        'aria-invalid': error ? true : undefined,
        'aria-describedby': `${id}-description${error ? ` ${id}-error` : ''}`,
      })}
    </div>
  ),
  SettingsList: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SettingsSection: ({ title, description, children }: { title: string; description: string; children: ReactNode }) => (
    <section>
      <h2>{title}</h2>
      <p>{description}</p>
      {children}
    </section>
  ),
  StatusPanel: ({ headline, detail }: { headline: string; detail: ReactNode }) => (
    <div role="alert">
      {headline}
      {detail}
    </div>
  ),
}))

function render(value: ConfigFormProps['value'] = {}, errors = new Map<string, string>()) {
  changes.clear()
  const onChange = vi.fn()
  const html = renderToStaticMarkup(
    <JevConfigForm id="judge-typesafe" schema={{}} value={value} errors={errors} onChange={onChange} />,
  )
  return { html, onChange }
}

describe('JevConfigForm', () => {
  it('masks the key and explains worker environment fallback and the default model', () => {
    const { html } = render({ api_key: 'fixture-only' })
    const key = html.match(/<input[^>]*name="api_key"[^>]*>/)?.[0]
    expect(key).toContain('type="password"')
    expect(key).toContain('autoComplete="new-password"')
    expect(key).toContain('spellCheck="false"')
    expect(html).toContain('TYPESAFE_API_KEY')
    expect(html).toContain('JEV worker process')
    expect(html).toContain('takes precedence')
    expect(html).toContain('placeholder="jev-1.13.0"')
  })

  it.each([
    ['api_key', `\${TYPESAFE_API_KEY}`],
    ['model', 'jev-custom'],
  ])('edits %s while retaining unknown fields and host errors', (field, next) => {
    const value = Object.freeze({ api_key: 'fixture-only', model: 'jev-1.13.0', future: { keep: true } })
    const errors = new Map([['/future', 'Check future setting']])
    const { onChange } = render(value, errors)
    expect(changes.has(field)).toBe(true)
    changes.get(field)?.(next)
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...value, [field]: next })
    expect(errors.get('/future')).toBe('Check future setting')
  })

  it.each(['api_key', 'model'])('clears %s to restore its default', (field) => {
    const { onChange } = render({ [field]: 'fixture-only', future: true })
    expect(changes.has(field)).toBe(true)
    changes.get(field)?.('')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ future: true })
  })

  it.each(limits)('shows the default for %s without writing configuration', (field, fallback) => {
    const { html, onChange } = render()
    const input = html.match(new RegExp(`<input[^>]*name="${field}"[^>]*>`))?.[0]
    expect(input).toBeDefined()
    expect(input).toContain('type="number"')
    expect(input).toContain('min="1"')
    expect(input).toContain('step="1"')
    expect(input).toContain(`placeholder="${fallback}"`)
    expect(onChange).not.toHaveBeenCalled()
  })

  it.each(limits)('edits %s as a number and preserves seeded and unknown values', (field) => {
    const value = Object.freeze({
      api_key: `\${TYPESAFE_API_KEY}`,
      model: 'jev-custom',
      max_request_bytes: 16777216,
      max_response_bytes: 4194304,
      max_timeout_ms: 60000,
      future: { keep: true },
    })
    const { html, onChange } = render(value)
    expect(html.match(new RegExp(`<input[^>]*name="${field}"[^>]*>`))?.[0]).toContain(`value="${value[field]}"`)
    expect(changes.has(field)).toBe(true)
    changes.get(field)?.('1')
    expect(onChange).toHaveBeenLastCalledWith({ ...value, [field]: 1 })
    changes.get(field)?.('16777216')
    expect(onChange).toHaveBeenLastCalledWith({ ...value, [field]: 16777216 })
  })

  it.each(limits)('clears %s to restore the worker default', (field) => {
    const { onChange } = render({ [field]: 1, future: true })
    expect(changes.has(field)).toBe(true)
    changes.get(field)?.('')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ future: true })
  })

  it.each(limits)('keeps invalid %s edits out of configuration', (field) => {
    const value = Object.freeze({ [field]: 60000, future: true })
    const { onChange } = render(value)
    expect(changes.has(field)).toBe(true)
    for (const invalid of ['0', '-1', '1.5', 'NaN', 'Infinity', '1e999', '9007199254740993', ' ']) {
      changes.get(field)?.(invalid)
    }
    expect(onChange).not.toHaveBeenCalled()
  })

  it('preserves unresolved limit placeholders when editing another field', () => {
    const value = Object.freeze({ max_timeout_ms: `\${JEV_MAX_TIMEOUT_MS}`, future: [1, 2] })
    const { onChange } = render(value)
    changes.get('model')?.('jev-custom')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...value, model: 'jev-custom' })
  })

  it('associates host errors with controls and shows root and unknown errors', () => {
    const { html } = render(
      {},
      new Map([
        ['/api_key', 'Use a string'],
        ['/model', 'Choose a model'],
        ...limits.map(([field]) => [`/${field}`, `Invalid ${field}`] as [string, string]),
        ['', 'Invalid configuration'],
        ['/future', 'Unknown setting'],
      ]),
    )
    for (const field of ['api_key', 'model', ...limits.map(([field]) => field)]) {
      const input = html.match(new RegExp(`<input[^>]*name="${field}"[^>]*>`))?.[0]
      expect(input).toContain('aria-invalid="true"')
      expect(input).toContain(`jev-cfg-${field}-error`)
    }
    for (const [field] of limits) {
      expect(html.split(`Invalid ${field}`)).toHaveLength(2)
    }
    expect(html).toContain('Invalid configuration')
    expect(html).toContain('Unknown setting')
  })

  it('keeps an opaque root untouched until it can be edited in the host', () => {
    const { html, onChange } = render(`\${JEV_CONFIGURATION}`)
    expect(html).toContain('configuration is supplied as a single value')
    expect(html).not.toContain('<input')
    expect(onChange).not.toHaveBeenCalled()
  })
})

describe('JEV configuration deep links', () => {
  afterEach(() => vi.unstubAllGlobals())

  it.each(['api_key', 'model', ...limits.map(([field]) => field)])('focuses %s from global Settings', async (field) => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)
    try {
      await act(async () =>
        root.render(<JevConfigForm id="judge-typesafe" schema={{}} value={{}} onChange={() => {}} focusField={[field]} />),
      )
      const input = container.querySelector(`input[name="${field}"]`)
      expect(input).not.toBeNull()
      expect(document.activeElement).toBe(input)
    } finally {
      await act(async () => root.unmount())
      container.remove()
    }
  })
})
