// @vitest-environment jsdom

import type {
  ConfigFormProps,
  ExtensionIii,
  SecretKeyFieldProps,
  SelectProps,
  SettingsFieldProps,
} from '@iii-dev/console-ui'
import { act, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { listModels, OpenAiConfigForm } from './index'

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
  Select: ({
    id,
    name,
    value,
    options,
    onChange,
    onClear,
    allowEmpty,
    emptyLabel,
    'aria-busy': busy,
    ...props
  }: SelectProps) => {
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
  SettingsSection: ({
    title,
    description,
    action,
    children,
  }: {
    title: string
    description: string
    action?: ReactNode
    children: ReactNode
  }) => (
    <section>
      <h2>{title}</h2>
      <p>{description}</p>
      {action}
      {children}
    </section>
  ),
  StatusPanel: ({
    variant,
    headline,
    detail,
    action,
  }: {
    variant?: string
    headline: ReactNode
    detail?: ReactNode
    action?: ReactNode
  }) => (
    <div role="alert" data-variant={variant}>
      {headline}
      {detail}
      {action}
    </div>
  ),
}))

const catalog = {
  status: 'ok',
  models: [{ name: 'gpt-6-luna', description: 'OpenAI Decisions (beta)', release_date: '2026-09-14' }],
}
const emptyCatalog = { status: 'ok', models: [] }
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
    <OpenAiConfigForm id="judge-openai" schema={{}} value={value} errors={errors} onChange={onChange} iii={engine()} />,
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
    root!.render(
      <OpenAiConfigForm id="judge-openai" schema={{}} value={value} onChange={onChange} iii={iii} {...extra} />,
    ),
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
    await expect(listModels(engine({ status: 'error', code: 'missing_key' }))).rejects.toThrow('missing_key')
    await expect(listModels(engine({ bogus: true }))).rejects.toThrow('invalid_response')
  })
})

describe('OpenAiConfigForm', () => {
  it('never renders the stored key, offers a blank replacement and a clear action', () => {
    const { html } = render({ api_key: 'fixture-only' })
    const key = html.match(/<input[^>]*name="api_key"[^>]*>/)?.[0]
    expect(key).toContain('type="password"')
    expect(key).toContain('autoComplete="new-password"')
    expect(key).toContain('spellCheck="false"')
    expect(key).toContain('value=""')
    expect(html).not.toContain('fixture-only')
    expect(key).toContain('type a new key to replace it')
    expect(html).toContain('Clear key')
    expect(html).toContain('restart judge-openai')
    expect(html).toContain('Built-in default (gpt-6-luna)')
    expect(html).toContain('Checking the worker…')
    const { html: empty } = render({})
    expect(empty).toContain('placeholder="Use OPENAI_API_KEY"')
    expect(empty).not.toContain('Clear key')
  })

  it.each([
    ['api_key', `\${OPENAI_API_KEY}`],
    ['model', 'gpt-6-luna-custom'],
  ])('edits %s while retaining unknown fields and host errors', (field, next) => {
    const value = Object.freeze({ api_key: 'fixture-only', model: 'gpt-6-luna-2026-10-01', future: { keep: true } })
    const errors = new Map([['/future', 'Check future setting']])
    const { onChange } = render(value, errors)
    expect(changes.has(field)).toBe(true)
    changes.get(field)?.(next)
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...value, [field]: next })
    expect(errors.get('/future')).toBe('Check future setting')
  })

  it('clears model to restore its default', () => {
    const { onChange } = render({ model: 'fixture-only', future: true })
    changes.get('model')?.('')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ future: true })
  })

  it('emptying the replacement field keeps the stored key; the clear action removes it', async () => {
    const value = Object.freeze({ api_key: 'fixture-only', future: true })
    const { container, onChange } = await mount(value)
    expect(container.querySelector<HTMLInputElement>('input[name="api_key"]')!.value).toBe('')
    expect(container.innerHTML).not.toContain('fixture-only')
    await act(async () => changes.get('api_key')?.('replacement'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, api_key: 'replacement' })
    await act(async () => changes.get('api_key')?.(''))
    expect(onChange).toHaveBeenLastCalledWith({ ...value })
    const clear = [...container.querySelectorAll('button')].find((b) => b.textContent === 'Clear key')!
    await act(async () => clear.click())
    expect(onChange).toHaveBeenLastCalledWith({ future: true })
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
      api_key: `\${OPENAI_API_KEY}`,
      model: 'gpt-6-luna-custom',
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
    const value = Object.freeze({ max_timeout_ms: `\${OPENAI_MAX_TIMEOUT_MS}`, future: [1, 2] })
    const { onChange } = render(value)
    changes.get('model')?.('gpt-6-luna-custom')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...value, model: 'gpt-6-luna-custom' })
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
      expect(input).toContain(`judge-openai-cfg-${field}-error`)
    }
    const select = html.match(/<select[^>]*name="model"[^>]*>/)?.[0]
    expect(select).toContain('aria-invalid="true"')
    expect(select).toContain('judge-openai-cfg-model-error')
    for (const [field] of limits) {
      expect(html.split(`Invalid ${field}`)).toHaveLength(2)
    }
    expect(html).toContain('Invalid configuration')
    expect(html).toContain('Unknown setting')
  })

  it('keeps an opaque root untouched until it can be edited in the host', () => {
    const { html, onChange } = render(`\${OPENAI_CONFIGURATION}`)
    expect(html).toContain('configuration is supplied as a single value')
    expect(html).not.toContain('<input')
    expect(onChange).not.toHaveBeenCalled()
  })
})

describe('OpenAiConfigForm catalog', () => {
  it('lists the catalog the worker answers, keeps the stored model selectable, and reports the key as accepted', async () => {
    const { container, iii } = await mount({ model: 'gpt-6-luna-2026-10-01' })
    expect(iii.trigger).toHaveBeenCalledWith(
      'judge-openai::models::list',
      { timeout_ms: 15_000 },
      { timeoutMs: 20_000 },
    )
    const select = container.querySelector<HTMLSelectElement>('select[name="model"]')!
    expect([...select.options].map((option) => option.value)).toEqual(['', 'gpt-6-luna', 'gpt-6-luna-2026-10-01'])
    expect(select.options[1].dataset.description).toBe('OpenAI Decisions (beta) · 2026-09-14')
    expect(select.options[2].dataset.description).toBe('Not in the current catalog')
    expect(select.value).toBe('gpt-6-luna-2026-10-01')
    expect(container.querySelector('[data-chip="success"]')?.textContent).toBe('Key accepted · 1 models')
    expect(container.querySelector('[role="alert"]')).toBeNull()
  })

  it('selects a catalog model and clears back to the built-in default', async () => {
    const value = Object.freeze({ model: 'gpt-6-luna-2026-10-01', future: { keep: true } })
    const { container, onChange } = await mount(value)
    const select = container.querySelector<HTMLSelectElement>('select[name="model"]')!
    await act(async () => {
      select.value = 'gpt-6-luna'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...value, model: 'gpt-6-luna' })
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
    expect(alert.textContent).toContain('OPENAI_API_KEY')
    expect(alert.querySelector('button')).toBeNull()
    expect(
      [...container.querySelector<HTMLSelectElement>('select[name="model"]')!.options].map((o) => o.value),
    ).toEqual([''])
  })

  it('warns when the key lists no supported model and keeps the stored model selectable', async () => {
    const { container } = await mount({ model: 'gpt-6-luna' }, engine(emptyCatalog))
    expect(container.querySelector('[data-chip="success"]')).toBeNull()
    expect(container.querySelector('[data-chip="warning"]')?.textContent).toBe('gpt-6-luna is not visible to this key')
    const select = container.querySelector<HTMLSelectElement>('select[name="model"]')!
    expect([...select.options].map((option) => option.value)).toEqual(['', 'gpt-6-luna'])
    expect(select.options[1].dataset.description).toBe('Not in the current catalog')
    expect(container.querySelector('[role="alert"]')).toBeNull()
  })

  it('surfaces other listing failures with a retry that asks the worker again', async () => {
    const iii = engine(new Error('invocation timed out'))
    const { container } = await mount({ model: 'gpt-6-luna-2026-10-01' }, iii)
    expect(container.querySelector('[data-chip="warning"]')?.textContent).toBe('Listing failed · invocation timed out')
    const alert = container.querySelector('[role="alert"][data-variant="warn"]')!
    expect(alert.textContent).toContain('Could not list models')
    expect(container.querySelector<HTMLSelectElement>('select[name="model"]')!.value).toBe('gpt-6-luna-2026-10-01')
    await act(async () => alert.querySelector('button')!.click())
    expect(iii.trigger).toHaveBeenCalledTimes(2)
  })

  it('checks again on retry instead of reporting the model as not visible', async () => {
    const iii = engine({ status: 'error', code: 'http' })
    const { container } = await mount({}, iii)
    iii.trigger.mockImplementation(() => new Promise<never>(() => {}))
    const alert = container.querySelector('[role="alert"][data-variant="warn"]')!
    await act(async () => alert.querySelector('button')!.click())
    expect(container.querySelector('[data-chip="warning"]')).toBeNull()
    expect(container.querySelector('[data-chip="neutral"]')?.textContent).toBe('Checking the worker…')
  })
})

describe('OpenAI configuration deep links', () => {
  it.each(['api_key', 'model', ...limits.map(([field]) => field)])('focuses %s from global Settings', async (field) => {
    const { container } = await mount({}, engine(), { focusField: [field] })
    const control = container.querySelector(`#judge-openai-cfg-${field}`)
    expect(control).not.toBeNull()
    expect(document.activeElement).toBe(control)
  })

  it('survives the host swapping an opaque root for an object while mounted', async () => {
    const { container, iii } = await mount(`\${OPENAI_CONFIGURATION}`)
    expect(container.innerHTML).toContain('configuration is supplied as a single value')
    await act(async () =>
      root!.render(
        <OpenAiConfigForm id="judge-openai" schema={{}} value={{ api_key: 'k' }} onChange={vi.fn()} iii={iii} />,
      ),
    )
    expect(container.querySelector('input[name="api_key"]')).not.toBeNull()
  })
})

describe('OpenAiConfigForm with the Console secret field', () => {
  it('hands the key to the shared field and stores only its reference', async () => {
    const seen: SecretKeyFieldProps[] = []
    function StubSecretField(props: SecretKeyFieldProps) {
      seen.push(props)
      return <output data-name={props.name} />
    }
    const { container, onChange } = await mount(
      { api_key: 'secret://OPENAI_API_KEY', model: 'gpt-6-luna-2026-10-01' },
      engine(),
      { secretField: StubSecretField } as Partial<ConfigFormProps>,
    )
    expect(container.querySelector('input[name="api_key"]')).toBeNull()
    const last = seen[seen.length - 1]
    expect(last).toMatchObject({
      name: 'OPENAI_API_KEY',
      value: 'secret://OPENAI_API_KEY',
      consumers: ['judge-openai'],
      keysUrl: 'https://platform.openai.com/api-keys',
    })
    // judge-openai resolves only `secret://`, so the field must not offer `env://`.
    expect(last).not.toHaveProperty('environment')
    expect(last.status).toMatchObject({ connected: true, detail: `${catalog.models.length} models` })
    await act(async () => last.onChange('secret://OPENAI_API_KEY_2'))
    expect(onChange).toHaveBeenLastCalledWith({ api_key: 'secret://OPENAI_API_KEY_2', model: 'gpt-6-luna-2026-10-01' })
  })

  it.each([
    ['an empty catalog', engine(emptyCatalog), 'gpt-6-luna is not visible to this key'],
    ['a missing key', engine({ status: 'error', code: 'missing_key' }), 'No key reaches the worker.'],
  ])('reports %s as not connected', async (_case, iii, error) => {
    const seen: SecretKeyFieldProps[] = []
    function StubSecretField(props: SecretKeyFieldProps) {
      seen.push(props)
      return <output data-name={props.name} />
    }
    await mount({}, iii, { secretField: StubSecretField } as Partial<ConfigFormProps>)
    expect(seen[seen.length - 1].status).toEqual({ checking: false, connected: false, error, detail: undefined })
  })
})
