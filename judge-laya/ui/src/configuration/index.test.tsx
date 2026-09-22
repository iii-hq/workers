// @vitest-environment jsdom

import type { ConfigFormProps, ExtensionIii, SelectProps, SettingsFieldProps } from '@iii-dev/console-ui'
import { act, type InputHTMLAttributes, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { LayaConfigForm, loadedModel } from './index'

const changes = vi.hoisted(() => new Map<string, (value: string) => void>())

vi.mock('@iii-dev/console-ui', () => ({
  Checkbox: ({ id, name, label, checked, onChange }: { id?: string; name?: string; label?: ReactNode; checked?: boolean; onChange?: (event: { currentTarget: { checked: boolean } }) => void }) => {
    changes.set(name!, (next) => onChange?.({ currentTarget: { checked: next === 'true' } }))
    return (
      <label>
        <input id={id} name={name} type="checkbox" checked={!!checked} onChange={() => {}} />
        {label}
      </label>
    )
  },
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

const card = { status: 'ok', models: [{ name: 'laya', description: 'laya checkpoint', release_date: '1c5edc1' }] }
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
  await act(async () => root!.render(<LayaConfigForm id="judge-laya" schema={{}} value={value} onChange={onChange} iii={iii} {...extra} />))
  return { container, onChange, iii }
}
afterEach(async () => {
  if (root) await act(async () => root!.unmount())
  root = undefined
  document.body.innerHTML = ''
})

describe('loadedModel', () => {
  it('returns the first card and throws typed codes', async () => {
    await expect(loadedModel(engine())).resolves.toEqual(card.models[0])
    await expect(loadedModel(engine({ status: 'error', code: 'deadline' }))).rejects.toThrow('deadline')
  })
})

describe('LayaConfigForm', () => {
  it('offers both checkpoints and reports the running one', async () => {
    const { container, iii } = await mount({ model: 'laya' })
    expect(iii.trigger).toHaveBeenCalledWith('judge-laya::models::list', { timeout_ms: 15_000 }, { timeoutMs: 20_000 })
    const select = container.querySelector<HTMLSelectElement>('select[name="model"]')!
    expect([...select.options].map((o) => o.value)).toEqual(['', 'laya', 'laya-multilingual', 'laya-typed-decisions'])
    // The default checkpoint is never offered for preloading.
    expect([...container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].map((box) => box.name)).toEqual([
      'preload:laya-multilingual',
      'preload:laya-typed-decisions',
      'auto_route',
      'auto_task_detection',
    ])
    expect(container.querySelector('[data-chip="success"]')?.textContent).toBe('Running laya · 1c5edc1')
    expect(container.textContent).toContain('no API key')
  })

  it('warns when the selection differs from the running checkpoint, or the worker does not answer', async () => {
    const { container } = await mount({ model: 'laya-multilingual' })
    expect(container.querySelector('[data-chip="warning"]')?.textContent).toBe('Running laya · restart to load laya-multilingual')
    const { container: down } = await mount({}, engine(new Error('invocation timed out')))
    expect(down.querySelector('[data-chip="warning"]')?.textContent).toContain('invocation timed out')
  })

  it('edits model, device and limits, preserving unknown fields', async () => {
    const value = Object.freeze({ model: 'laya', batch_questions: 8, future: { keep: true } })
    const { onChange } = await mount(value)
    await act(async () => changes.get('model')?.('laya-multilingual'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, model: 'laya-multilingual' })
    await act(async () => changes.get('revision')?.('abc123'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, revision: 'abc123' })
    await act(async () => changes.get('threads')?.('6'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, threads: 6 })
    await act(async () => changes.get('batch_questions')?.('32'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, batch_questions: 32 })
    await act(async () => changes.get('batch_questions')?.(''))
    expect(onChange).toHaveBeenLastCalledWith({ model: 'laya', future: { keep: true } })
    onChange.mockClear()
    await act(async () => changes.get('max_timeout_ms')?.('-1'))
    expect(onChange).not.toHaveBeenCalled()
    await act(async () => changes.get('model')?.(''))
    expect(onChange).toHaveBeenLastCalledWith({ batch_questions: 8, future: { keep: true } })
  })

  it('edits preload, routing flags and the shortlist', async () => {
    const value = Object.freeze({ model: 'laya', preload: ['laya-typed-decisions'] })
    const { onChange } = await mount(value)
    await act(async () => changes.get('preload:laya-multilingual')?.('true'))
    expect(onChange).toHaveBeenLastCalledWith({ model: 'laya', preload: ['laya-typed-decisions', 'laya-multilingual'] })
    await act(async () => changes.get('preload:laya-typed-decisions')?.('false'))
    expect(onChange).toHaveBeenLastCalledWith({ model: 'laya' })
    await act(async () => changes.get('auto_route')?.('true'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, auto_route: true })
    await act(async () => changes.get('auto_task_detection')?.('false'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value })
    await act(async () => changes.get('shortlist_k')?.('20'))
    expect(onChange).toHaveBeenLastCalledWith({ ...value, shortlist_k: 20 })
    await act(async () => changes.get('shortlist_k')?.(''))
    expect(onChange).toHaveBeenLastCalledWith({ ...value })
  })

  it('maps field errors, shows the rest in a panel, and keeps opaque roots', async () => {
    const { container } = await mount({}, engine(), { errors: new Map([['/revision', 'bad revision'], ['', 'root failure']]) })
    expect(container.querySelector('#laya-cfg-revision-error')?.textContent).toBe('bad revision')
    expect(container.querySelector('[role="alert"][data-variant="alert"]')?.textContent).toContain('root failure')
    const { container: opaque, onChange } = await mount('${LAYA}')
    expect(opaque.textContent).toContain('single value')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('focuses the deep-linked control', async () => {
    const { container } = await mount({}, engine(), { focusField: ['revision'] })
    expect(document.activeElement).toBe(container.querySelector('#laya-cfg-revision'))
  })
})
