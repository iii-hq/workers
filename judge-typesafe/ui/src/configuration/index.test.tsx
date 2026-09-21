// @vitest-environment jsdom

import type { ConfigFormProps, ExtensionIii, SelectProps, SettingsFieldProps } from '@iii-dev/console-ui'
import { act, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { JevConfigForm, listModels } from './index'

const changes = vi.hoisted(() => new Map<string, (value: string) => void>())
const limits = [
  ['max_request_bytes', 8388608],
  ['max_response_bytes', 8388608],
  ['max_timeout_ms', 300000],
] as const

// The console provides these components via its import map. Mirror the public
// contract so the form can run without a live console or configuration store.
vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
  Chip: ({ tone, children }: { tone?: string; children: ReactNode }) => <span data-chip={tone}>{children}</span>,
  Input: ({
    onChange,
    ...props
  }: Omit<InputHTMLAttributes<HTMLInputElement>, 'onChange'> & {
    onChange: (value: string) => void
  }) => {
    changes.set(props.name!, onChange)
    return <input {...props} onChange={(event) => onChange(event.currentTarget.value)} />
  },
  Select: ({ id, name, value, options, onChange, onClear, allowEmpty, emptyLabel, 'aria-busy': busy, ...props }: SelectProps) => {
    changes.set(name!, (next) => (next === '' ? onClear?.() : onChange(next)))
    return (
      <select
        id={id}
        name={name}
        value={value ?? ''}
        aria-busy={busy}
        aria-invalid={props['aria-invalid']}
        aria-describedby={props['aria-describedby']}
        onChange={(event) => (event.currentTarget.value === '' ? onClear?.() : onChange(event.currentTarget.value))}
      >
        {allowEmpty ? <option value="">{emptyLabel}</option> : null}
        {options?.map((option) => (
          <option key={option.value} value={option.value} data-description={option.description}>
            {option.label}
          </option>
        ))}
      </select>
    )
  },
  SettingsField: ({ id, field, label, description, error, meta, renderControl }: SettingsFieldProps) => (
    <div data-field={field}>
      <label htmlFor={id}>{label}</label>
      <span id={`${id}-description`}>{description}</span>
      {meta ? <div data-meta>{meta}</div> : null}
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
  SettingsSection: ({ title, description, action, children }: { title: string; description: string; action?: ReactNode; children: ReactNode }) => (
    <section>
      <h2>{title}</h2>
      <p>{description}</p>
      {action}
      {children}
    </section>
  ),
  StatusPanel: ({ variant, headline, detail, action }: { variant?: string; headline: ReactNode; detail?: ReactNode; action?: ReactNode }) => (
    <div role="alert" data-variant={variant}>
      {headline}
      {detail}
      {action}
    </div>
  ),
}))

const catalog = {
  status: 'ok',
  models: [
    { name: 'jev-latest', description: 'Latest JEV', release_date: '2026-09-10T18:38:01Z' },
    { name: 'jev-preview', description: 'Preview', release_date: '2026-09-10T18:39:06Z' },
  ],
}
type Engine = Pick<ExtensionIii, 'trigger'>
const engine = (reply: unknown = catalog) => {
  const trigger = vi.fn(async (_id: string, _payload?: Record<string, unknown>, _options?: { timeoutMs?: number }) => {
    if (reply instanceof Error) throw reply
    return reply
  })
  return { trigger } as unknown as Engine & { trigger: typeof trigger }
}

// Effects never run here, so the catalog stays in its "checking" state.
function render(value: ConfigFormProps['value'] = {}, errors = new Map<string, string>()) {
  changes.clear()
  const onChange = vi.fn()
  const html = renderToStaticMarkup(
    <JevConfigForm id="judge-typesafe" schema={{}} value={value} errors={errors} onChange={onChange} iii={engine()} />,
  )
  return { html, onChange }
}

let root: Root | undefined
async function mount(value: ConfigFormProps['value'], iii = engine(), extra: Partial<ConfigFormProps> = {}) {
  changes.clear()
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  const onChange = vi.fn()
  await act(async () =>
    root!.render(<JevConfigForm id="judge-typesafe" schema={{}} value={value} onChange={onChange} iii={iii} {...extra} />),
  )
  return { container, onChange, iii }
}
afterEach(async () => {
  if (root) await act(async () => root!.unmount())
  root = undefined
  document.body.innerHTML = ''
})

describe('listModels', () => {
  it('returns the catalog and turns typed refusals into ProviderError codes', async () => {
    await expect(listModels(engine())).resolves.toEqual(catalog.models)
    await expect(listModels(engine({ status: 'error', code: 'missing_key' }))).rejects.toMatchObject({ code: 'missing_key' })
    await expect(listModels(engine({ bogus: true }))).rejects.toMatchObject({ code: 'invalid_response' })
  })
})

describe('JevConfigForm', () => {
  it('masks the key and explains worker environment fallback and the default model', () => {
    const { html } = render({ api_key: 'fixture-only' })
    const key = html.match(/<input[^>]*name="api_key"[^>]*>/)?.[0]
    expect(key).toContain('type="password"')
    expect(key).toContain('autoComplete="new-password"')
    expect(key).toContain('spellCheck="false"')
    expect(html).toContain('TYPESAFE_API_KEY')
    expect(html).toContain('restart judge-typesafe')
    expect(html).toContain('Built-in default (jev-latest)')
    expect(html).toContain('Checking the worker…')
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
    for (const field of ['api_key', ...limits.map(([field]) => field)]) {
      const input = html.match(new RegExp(`<input[^>]*name="${field}"[^>]*>`))?.[0]
      expect(input).toContain('aria-invalid="true"')
      expect(input).toContain(`jev-cfg-${field}-error`)
    }
    const select = html.match(/<select[^>]*name="model"[^>]*>/)?.[0]
    expect(select).toContain('aria-invalid="true"')
    expect(select).toContain('jev-cfg-model-error')
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

describe('JevConfigForm catalog', () => {
  it('lists the catalog the worker answers, keeps the stored model selectable, and reports the key as accepted', async () => {
    const { container, iii } = await mount({ model: 'jev-1.13.0' })
    expect(iii.trigger).toHaveBeenCalledWith('judge-typesafe::models::list', { timeout_ms: 15_000 }, { timeoutMs: 20_000 })
    const select = container.querySelector<HTMLSelectElement>('select[name="model"]')!
    expect([...select.options].map((option) => option.value)).toEqual(['', 'jev-latest', 'jev-preview', 'jev-1.13.0'])
    expect(select.options[1].dataset.description).toBe('Latest JEV · 2026-09-10')
    expect(select.options[3].dataset.description).toBe('Not in the current catalog')
    expect(select.value).toBe('jev-1.13.0')
    expect(container.querySelector('[data-chip="success"]')?.textContent).toBe('Key accepted · 2 models')
    expect(container.querySelector('[role="alert"]')).toBeNull()
  })

  it('selects a catalog model and clears back to the built-in default', async () => {
    const value = Object.freeze({ model: 'jev-1.13.0', future: { keep: true } })
    const { container, onChange } = await mount(value)
    const select = container.querySelector<HTMLSelectElement>('select[name="model"]')!
    await act(async () => {
      select.value = 'jev-latest'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...value, model: 'jev-latest' })
    onChange.mockClear()
    await act(async () => {
      select.value = ''
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ future: { keep: true } })
  })

  it('explains a missing key on the credentials field and in a panel', async () => {
    const { container } = await mount({}, engine({ status: 'error', code: 'missing_key' }))
    expect(container.querySelector('[data-chip="warning"]')?.textContent).toBe('No key reaches the worker')
    const alert = container.querySelector('[role="alert"][data-variant="warn"]')!
    expect(alert.textContent).toContain('No API key reaches the worker')
    expect(alert.textContent).toContain('TYPESAFE_API_KEY')
    expect(alert.querySelector('button')).toBeNull()
    expect([...container.querySelector<HTMLSelectElement>('select[name="model"]')!.options].map((o) => o.value)).toEqual([''])
  })

  it('surfaces other listing failures with a retry that asks the worker again', async () => {
    const iii = engine(new Error('invocation timed out'))
    const { container } = await mount({ model: 'jev-1.13.0' }, iii)
    expect(container.querySelector('[data-chip="warning"]')?.textContent).toBe('Listing failed · invocation timed out')
    const alert = container.querySelector('[role="alert"][data-variant="warn"]')!
    expect(alert.textContent).toContain('Could not list models')
    expect(container.querySelector<HTMLSelectElement>('select[name="model"]')!.value).toBe('jev-1.13.0')
    await act(async () => alert.querySelector('button')!.click())
    expect(iii.trigger).toHaveBeenCalledTimes(2)
  })
})

describe('JEV configuration deep links', () => {
  it.each(['api_key', 'model', ...limits.map(([field]) => field)])('focuses %s from global Settings', async (field) => {
    const { container } = await mount({}, engine(), { focusField: [field] })
    const control = container.querySelector(`#jev-cfg-${field}`)
    expect(control).not.toBeNull()
    expect(document.activeElement).toBe(control)
  })
})
