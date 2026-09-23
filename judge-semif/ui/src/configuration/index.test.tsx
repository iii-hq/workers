// @vitest-environment jsdom

import type { ConfigFormProps, ExtensionIii, SelectProps, SettingsFieldProps } from '@iii-dev/console-ui'
import { act, type InputHTMLAttributes, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { deviceOf, loadedModel, SemifConfigForm } from './index'

const changes = vi.hoisted(() => new Map<string, (value: string) => void>())

vi.mock('@iii-dev/console-ui', () => ({
  Chip: ({ tone, children }: { tone?: string; children: ReactNode }) => <span data-chip={tone}>{children}</span>,
  Input: ({ onChange, ...props }: Omit<InputHTMLAttributes<HTMLInputElement>, 'onChange'> & { onChange: (value: string) => void }) => {
    changes.set(props.name!, onChange)
    return <input {...props} onChange={(event) => onChange(event.currentTarget.value)} />
  },
  Select: ({ id, name, value, options, onChange, onClear, allowEmpty, emptyLabel }: SelectProps) => {
    changes.set(name!, (next) => (next === '' ? onClear?.() : onChange(next)))
    return (
      <select id={id} name={name} value={value ?? ''} onChange={() => {}}>
        {allowEmpty ? <option value="">{emptyLabel}</option> : null}
        {options?.map((option) => (
          <option key={option.value} value={option.value}>
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
      {renderControl({ id: id!, name: field, 'aria-describedby': `${id}-description` })}
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
  StatusPanel: ({ variant, headline, detail }: { variant?: string; headline: ReactNode; detail?: ReactNode }) => (
    <div role="alert" data-variant={variant}>
      {headline}
      {detail}
    </div>
  ),
}))

const card = {
  status: 'ok',
  models: [
    {
      name: 'qwen3.5-4b',
      description: 'Qwen3.5-4B Q4_K_M; running in-process on AMD Radeon RX 6900 XT (Vulkan) with a 16384-token window',
      release_date: '4168f45',
    },
  ],
}
type Engine = Pick<ExtensionIii, 'trigger'>
const engine = (reply: unknown = card) => {
  const trigger = vi.fn(async (_id: string, _payload?: Record<string, unknown>, _options?: { timeoutMs?: number }) => {
    if (reply instanceof Error) throw reply
    return reply
  })
  return { trigger } as unknown as Engine & { trigger: typeof trigger }
}

let root: Root | undefined
async function mount(value: ConfigFormProps['value'], iii = engine(), extra: Partial<ConfigFormProps> = {}) {
  changes.clear()
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  const onChange = vi.fn()
  await act(async () => root!.render(<SemifConfigForm id="judge-semif" schema={{}} value={value} onChange={onChange} iii={iii} {...extra} />))
  return { container, onChange, iii }
}
afterEach(async () => {
  if (root) await act(async () => root!.unmount())
  root = undefined
  document.body.innerHTML = ''
})

describe('loadedModel', () => {
  it('returns the first card, its device, and throws typed codes', async () => {
    await expect(loadedModel(engine())).resolves.toEqual(card.models[0])
    expect(deviceOf(card.models[0])).toBe('AMD Radeon RX 6900 XT (Vulkan)')
    await expect(loadedModel(engine({ status: 'error', code: 'deadline' }))).rejects.toThrow('deadline')
  })
})

describe('SemifConfigForm', () => {
  it('offers the model and reports where it runs', async () => {
    const { container, iii } = await mount({})
    expect(iii.trigger).toHaveBeenCalledWith('judge-semif::models::list', { timeout_ms: 15_000 }, { timeoutMs: 20_000 })
    const select = container.querySelector<HTMLSelectElement>('select[name="model"]')!
    expect([...select.options].map((o) => o.value)).toEqual(['', 'qwen3.5-4b'])
    expect(container.querySelector('[data-chip="success"]')?.textContent).toBe('Running qwen3.5-4b on AMD Radeon RX 6900 XT (Vulkan)')
    expect(container.textContent).toContain('no API key')
  })

  it('warns when the worker does not answer', async () => {
    const { container } = await mount({}, engine(new Error('invocation timed out')))
    expect(container.querySelector('[data-chip="warning"]')?.textContent).toContain('invocation timed out')
  })

  it('edits placement and limits, allows 0 GPU layers, preserves unknown fields', async () => {
    const value = Object.freeze({ threads: 8, future: { keep: true } })
    const { onChange } = await mount(value)
    await act(async () => changes.get('gpu_layers')?.('0'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, gpu_layers: 0 })
    await act(async () => changes.get('context_tokens')?.('8192'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, context_tokens: 8192 })
    await act(async () => changes.get('parallel_questions')?.('16'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, parallel_questions: 16 })
    await act(async () => changes.get('threads')?.(''))
    expect(onChange).toHaveBeenLastCalledWith({ future: { keep: true } })
    onChange.mockClear()
    await act(async () => changes.get('threads')?.('0'))
    await act(async () => changes.get('max_timeout_ms')?.('-1'))
    expect(onChange).not.toHaveBeenCalled()
    await act(async () => changes.get('model')?.('qwen3.5-4b'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, model: 'qwen3.5-4b' })
  })

  it('maps field errors, shows the rest in a panel, and keeps opaque roots', async () => {
    const { container } = await mount({}, engine(), { errors: new Map([['/context_tokens', 'too small'], ['', 'root failure']]) })
    expect(container.querySelector('#semif-cfg-context_tokens-error')?.textContent).toBe('too small')
    expect(container.querySelector('[role="alert"][data-variant="alert"]')?.textContent).toContain('root failure')
    const { container: opaque, onChange } = await mount('${SEMIF}')
    expect(opaque.textContent).toContain('single value')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('focuses the deep-linked control', async () => {
    const { container } = await mount({}, engine(), { focusField: ['gpu_layers'] })
    expect(document.activeElement).toBe(container.querySelector('#semif-cfg-gpu_layers'))
  })
})
