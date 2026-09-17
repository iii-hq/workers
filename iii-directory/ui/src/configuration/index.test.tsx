// @vitest-environment jsdom

import type { ConfigFormProps } from '@iii-dev/console-ui'
import { act, useState, type InputHTMLAttributes, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DirectoryConfigForm } from './index'

const controlChanges = vi.hoisted(() => new Map<string, (value: string) => void>())

// Injected worker UIs receive this package from the Console import map. The
// package deliberately throws when loaded directly, so the test mirrors only
// the documented rendering contract needed by this form.
vi.mock('@iii-dev/console-ui', () => ({
  Chip: ({ children }: { children?: ReactNode }) => <span>{children}</span>,
  Input: ({ onChange, ...props }: Omit<InputHTMLAttributes<HTMLInputElement>, 'onChange'> & {
    name: string
    onChange: (value: string) => void
  }) => {
    controlChanges.set(props.name, onChange)
    return <input {...props} onChange={(event) => onChange(event.currentTarget.value)} />
  },
  Select: ({
    onChange,
    options,
    value,
    sheetTitle: _sheetTitle,
    sheetDescription: _sheetDescription,
    ...props
  }: {
    name: string
    onChange: (value: string) => void
    options: Array<{ label: string; value: string }>
    value?: string
    sheetTitle?: string
    sheetDescription?: string
  }) => {
    controlChanges.set(props.name, onChange)
    return (
      <select {...props} defaultValue={value}>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    )
  },
  SettingsList: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  SettingsRow: ({
    label,
    description,
    meta,
    control,
    ...props
  }: {
    label?: ReactNode
    description?: ReactNode
    meta?: ReactNode
    control?: ReactNode
  }) => (
    <div {...props}>
      {label}
      {description}
      {meta}
      {control}
    </div>
  ),
  SettingsSection: ({
    title,
    description,
    children,
  }: {
    title?: ReactNode
    description?: ReactNode
    children?: ReactNode
  }) => (
    <section>
      <h2>{title}</h2>
      <p>{description}</p>
      {children}
    </section>
  ),
  Switch: ({ onChange: _onChange, ...props }: { onChange?: unknown }) => <input type="checkbox" {...props} readOnly />,
}))

function renderConfiguration(
  value: ConfigFormProps['value'],
  errors: ConfigFormProps['errors'] = new Map(),
  onChange: ConfigFormProps['onChange'] = () => {},
) {
  return renderToStaticMarkup(
    <DirectoryConfigForm id="iii-directory" schema={{}} value={value} onChange={onChange} errors={errors} />,
  )
}

describe('DirectoryConfigForm function search settings', () => {
  beforeEach(() => controlChanges.clear())

  it('renders hybrid as the default without a model notice', () => {
    const html = renderConfiguration({})

    expect(html).toContain('Function search mode')
    expect(html).toContain('<option value="hybrid" selected="">')
    expect(html).not.toContain('requires a local semantic model')
  })

  it('shows how to recover when the model directory is cleared', () => {
    const html = renderConfiguration({ function_search_mode: 'hybrid', function_search_model_path: null })

    expect(html).toContain('Hybrid')
    expect(html).toContain('requires a local semantic model')
    expect(html).toContain('function_search_model_path')
    expect(html).toContain('Restart required')
  })

  it('renders Jev without a MiniLM notice and explains key precedence and lexical fallback', () => {
    const html = renderConfiguration({ function_search_mode: 'jev', function_search_model_path: null })

    expect(html).toContain('<option value="jev" selected="">')
    expect(html).not.toContain('requires a local semantic model')
    expect(html).toContain('TypeSafe')
    expect(html).toContain('TYPESAFE_API_KEY')
    expect(html).toContain('worker process')
    expect(html).toContain('takes precedence')
    expect(html).toContain('lexical fallback')
    expect(html).toContain('valid empty result')
    const input = html.match(/<input[^>]*name="function_search_jev_api_key"[^>]*>/)?.[0]
    expect(input).toContain('type="password"')
    expect(input).toContain('autoComplete="new-password"')
    expect(input).toContain('spellCheck="false"')
  })

  it('renders Jev controls with defaults, numeric bounds and hot-reload guidance', () => {
    const html = renderConfiguration({})

    expect(html).toContain('Jev options apply without restarting')
    for (const [field, placeholder, bounds] of [
      ['function_search_jev_model', 'jev-1.13.0', []],
      ['function_search_jev_timeout_ms', '3000', ['min="1"', 'max="30000"', 'step="1"']],
      ['function_search_jev_min_relevance', '0.5', ['min="0"', 'max="1"', 'step="any"', 'inputMode="decimal"']],
    ] as const) {
      const input = html.match(new RegExp(`<input[^>]*name="${field}"[^>]*>`))?.[0]
      expect(input).toBeDefined()
      expect(input).toContain(`placeholder="${placeholder}"`)
      for (const bound of bounds) expect(input).toContain(bound)
      const label = html.match(new RegExp(`<label[^>]*for="dir-cfg-${field}"[^>]*>(.*?)</label>`))?.[1]
      expect(label).toBeDefined()
      expect(label).not.toContain('Restart required')
    }
  })

  it.each([
    ['function_search_mode', 'jev', 'jev'],
    ['function_search_jev_api_key', 'configured-test-key', 'configured-test-key'],
    ['function_search_jev_model', 'jev-custom', 'jev-custom'],
  ])('edits %s while preserving the rest of the draft and host errors', (field, raw, expected) => {
    const draft = Object.freeze({
      function_search_mode: 'hybrid',
      function_search_model_path: null,
      function_search_jev_model: 'jev-1.13.0',
      function_search_jev_timeout_ms: 3000,
      function_search_jev_min_relevance: 0.5,
      registry_search: false,
      future_setting: { keep: true },
    })
    const errors = new Map([['/future_setting', 'Keep this host error']])
    const onChange = vi.fn()
    renderConfiguration(draft, errors, onChange)

    expect(controlChanges.has(field)).toBe(true)
    controlChanges.get(field)?.(raw)

    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...draft, [field]: expected })
    expect(errors).toEqual(new Map([['/future_setting', 'Keep this host error']]))
  })

  it('clears the Jev model to restore its worker default', () => {
    const field = 'function_search_jev_model'
    const onChange = vi.fn()
    renderConfiguration({ [field]: 'jev-custom', registry_search: false }, new Map(), onChange)

    expect(controlChanges.has(field)).toBe(true)
    controlChanges.get(field)?.('')

    expect(onChange).toHaveBeenCalledExactlyOnceWith({ registry_search: false })
  })

  it('masks a configured key and clearing it removes only the override', () => {
    const field = 'function_search_jev_api_key'
    const onChange = vi.fn()
    const html = renderConfiguration({ [field]: 'configured-test-key', registry_search: false }, new Map(), onChange)
    const input = html.match(new RegExp(`<input[^>]*name="${field}"[^>]*>`))?.[0]
    expect(input).toContain('type="password"')
    expect(input).toContain('value="configured-test-key"')
    const label = html.match(new RegExp(`<label[^>]*for="dir-cfg-${field}"[^>]*>(.*?)</label>`))?.[1]
    expect(label).toBeDefined()
    expect(label).not.toContain('Restart required')
    expect(controlChanges.has(field)).toBe(true)
    controlChanges.get(field)?.('')
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ registry_search: false })
  })

  it('associates Jev errors with their controls while retaining unknown host errors', () => {
    const errors = new Map([
      ['/function_search_jev_api_key', 'Enter a string or clear the override'],
      ['/function_search_jev_model', 'Enter a non-empty model'],
      ['/function_search_jev_timeout_ms', 'Use 1 to 30000 ms'],
      ['/function_search_jev_min_relevance', 'Use a finite value from 0 to 1'],
      ['/future_setting', 'Unknown setting error'],
    ])
    const html = renderConfiguration({ function_search_mode: 'lexical' }, errors)

    for (const [pointer, message] of [...errors].slice(0, 4)) {
      const field = pointer.slice(1)
      const input = html.match(new RegExp(`<input[^>]*name="${field}"[^>]*>`))?.[0]
      expect(input).toContain('aria-invalid="true"')
      expect(input).toContain(`aria-describedby="dir-cfg-${field}-description dir-cfg-${field}-error"`)
      expect(html).toContain(`id="dir-cfg-${field}-error">${message}</span>`)
      expect(html).not.toContain(`${pointer}: ${message}`)
    }
    expect(html).toContain('/future_setting: Unknown setting error')
    expect(html).toContain('There are 5 configuration errors')
  })
})

describe('DirectoryConfigForm numeric editing', () => {
  let root: ReturnType<typeof createRoot> | undefined
  let container: HTMLDivElement
  beforeEach(() => vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true))
  afterEach(async () => {
    await act(async () => root?.unmount())
    container?.remove()
    vi.unstubAllGlobals()
  })

  async function mountConfiguration(initial: ConfigFormProps['value'], errors = new Map<string, string>()) {
    const saved = vi.fn()
    let replaceValue: (value: ConfigFormProps['value']) => void = () => {}
    function Host() {
      const [value, setValue] = useState<ConfigFormProps['value']>(initial)
      replaceValue = setValue
      return (
        <DirectoryConfigForm id="iii-directory" schema={{}} value={value} errors={errors}
          onChange={(next) => { saved(next); setValue(next) }} />
      )
    }
    container = document.createElement('div')
    document.body.append(container)
    root = createRoot(container)
    await act(async () => root?.render(<Host />))
    return {
      saved,
      input: (field: string) => container.querySelector<HTMLInputElement>(`[name="${field}"]`)!,
      replaceValue: async (value: ConfigFormProps['value']) => act(async () => replaceValue(value)),
    }
  }

  async function type(input: HTMLInputElement, raw: string) {
    // Use the native setter so React sees a browser input event, including
    // the controlled-value round trip before the next character is typed.
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, raw)
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
  }

  it('keeps the zero while typing 0.105 and saves the exact number on blur', async () => {
    const initial = { function_search_jev_min_relevance: 0.5, registry_search: false }
    const form = await mountConfiguration(initial)
    const input = form.input('function_search_jev_min_relevance')
    await act(async () => input.focus())
    await type(input, '0.1')
    await type(input, `${input.value}0`)
    expect(input.value).toBe('0.10')
    await type(input, `${input.value}5`)
    expect(input.value).toBe('0.105')
    expect(form.saved).not.toHaveBeenCalled()
    await act(async () => input.blur())
    expect(form.saved).toHaveBeenCalledExactlyOnceWith({ ...initial, function_search_jev_min_relevance: 0.105 })
  })

  it.each([
    ['function_search_jev_timeout_ms', '4500', 4500],
    ['function_search_jev_min_relevance', '0', 0],
    ['function_search_jev_min_relevance', '0.725', 0.725],
  ])('commits %s on Enter, preserving other fields and host errors', async (field, raw, expected) => {
    const initial = { registry_search: false, future_setting: { keep: true } }
    const errors = new Map([['/future_setting', 'Keep this host error']])
    const form = await mountConfiguration(initial, errors)
    const input = form.input(field)
    await act(async () => input.focus())
    await type(input, raw)
    expect(form.saved).not.toHaveBeenCalled()
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    })
    expect(form.saved).toHaveBeenCalledExactlyOnceWith({ ...initial, [field]: expected })
    expect(errors).toEqual(new Map([['/future_setting', 'Keep this host error']]))
    expect(container.textContent).toContain('Keep this host error')
  })

  it.each(['function_search_jev_timeout_ms', 'function_search_jev_min_relevance'])(
    'clears %s on blur to restore its worker default', async (field) => {
      const form = await mountConfiguration({ [field]: 1, registry_search: false })
      const input = form.input(field)
      await act(async () => input.focus())
      await type(input, '')
      await act(async () => input.blur())
      expect(form.saved).toHaveBeenCalledExactlyOnceWith({ registry_search: false })
    },
  )

  it('discards pending input when the host replaces the configured value', async () => {
    const form = await mountConfiguration({ function_search_jev_min_relevance: 0.5 })
    const input = form.input('function_search_jev_min_relevance')
    await act(async () => input.focus())
    await type(input, '0.10')
    await form.replaceValue({ function_search_jev_min_relevance: 0.75 })
    expect(input.value).toBe('0.75')
    await act(async () => input.blur())
    expect(form.saved).not.toHaveBeenCalled()
  })
})
