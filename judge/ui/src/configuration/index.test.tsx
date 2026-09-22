// @vitest-environment jsdom

import type { ConfigFormProps, ExtensionIii, SelectProps, SettingsFieldProps } from '@iii-dev/console-ui'
import { act, type ButtonHTMLAttributes, type ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { JudgeRoutingForm, listProviders } from './index'

// The console provides these components via its import map. Mirror the public
// contract so the form can run without a live console or configuration store.
vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
  Chip: ({ tone, children }: { tone?: string; children: ReactNode }) => <span data-chip={tone}>{children}</span>,
  Select: ({ id, name, value, options, onChange, onClear, allowEmpty, emptyLabel, 'aria-busy': busy }: SelectProps) => (
    <select
      id={id}
      name={name}
      value={value ?? ''}
      aria-busy={busy}
      onChange={(event) => (event.currentTarget.value === '' ? onClear?.() : onChange(event.currentTarget.value))}
    >
      {allowEmpty ? <option value="">{emptyLabel}</option> : null}
      {options?.map((option) => (
        <option key={option.value} value={option.value} data-description={option.description}>
          {option.label}
        </option>
      ))}
    </select>
  ),
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

const registry = {
  functions: [
    { function_id: 'judge-typesafe::evaluate', namespace: 'my-project', worker_name: 'judge-typesafe' },
    { function_id: 'judge-typesafe::cancel', namespace: 'my-project', worker_name: 'judge-typesafe' },
    { function_id: 'judge-local-llm::evaluate', namespace: 'my-project', worker_name: 'judge-local-llm' },
    { function_id: 'judge::evaluate', namespace: 'my-project', worker_name: 'judge' },
    { function_id: 'judgemental::evaluate', namespace: 'my-project', worker_name: 'judgemental' },
  ],
}
type Engine = Pick<ExtensionIii, 'trigger'>
const engine = (reply: unknown = registry) => {
  const trigger = vi.fn(async (_id: string, _payload?: Record<string, unknown>, _options?: { timeoutMs?: number }) => {
    if (reply instanceof Error) throw reply
    return reply
  })
  return { trigger } as unknown as Engine & { trigger: typeof trigger }
}

let root: Root | undefined
async function mount(
  value: ConfigFormProps['value'],
  iii = engine(),
  extra: Partial<ConfigFormProps> = {},
) {
  const container = document.body.appendChild(document.createElement('div'))
  root = createRoot(container)
  const onChange = vi.fn()
  await act(async () =>
    root!.render(<JudgeRoutingForm id="judge" schema={{}} value={value} onChange={onChange} iii={iii} {...extra} />),
  )
  return { container, onChange, iii }
}
afterEach(async () => {
  if (root) await act(async () => root!.unmount())
  root = undefined
  document.body.innerHTML = ''
})

describe('listProviders', () => {
  it('keeps one row per judge-<provider> worker answering evaluate', async () => {
    await expect(listProviders(engine())).resolves.toEqual([
      { provider: 'local-llm', worker: 'judge-local-llm', namespace: 'my-project' },
      { provider: 'typesafe', worker: 'judge-typesafe', namespace: 'my-project' },
    ])
  })
})

describe('JudgeRoutingForm', () => {
  it('offers the registered providers, the built-in default, and marks the selection as registered', async () => {
    const { container, iii } = await mount({ provider: 'typesafe' })
    expect(iii.trigger).toHaveBeenCalledWith(
      'engine::functions::list',
      { include_internal: true },
      { timeoutMs: 10_000 },
    )
    const select = container.querySelector<HTMLSelectElement>('select[name="provider"]')!
    expect([...select.options].map((option) => option.value)).toEqual(['', 'local-llm', 'typesafe'])
    expect(select.options[0].textContent).toBe('Built-in default (typesafe)')
    expect(select.options[2].dataset.description).toBe('judge-typesafe · my-project')
    expect(select.value).toBe('typesafe')
    expect(container.querySelector('[data-chip="success"]')?.textContent).toBe('judge-typesafe · my-project')
    expect(container.querySelector('[role="alert"]')).toBeNull()
  })

  it('emits the chosen provider, preserves unknown fields, and clears back to the built-in default', async () => {
    const value = Object.freeze({ provider: 'typesafe', future: { keep: true } })
    const { container, onChange } = await mount(value)
    const select = container.querySelector<HTMLSelectElement>('select[name="provider"]')!
    await act(async () => {
      select.value = 'local-llm'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ ...value, provider: 'local-llm' })
    onChange.mockClear()
    await act(async () => {
      select.value = ''
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    expect(onChange).toHaveBeenCalledExactlyOnceWith({ future: { keep: true } })
  })

  it('keeps an unregistered stored provider selectable and warns that calls will fail', async () => {
    const { container } = await mount({ provider: 'missing' })
    const select = container.querySelector<HTMLSelectElement>('select[name="provider"]')!
    expect(select.value).toBe('missing')
    expect(select.options[select.selectedIndex].dataset.description).toBe('Not registered on the engine')
    expect(container.querySelector('[data-chip="warning"]')?.textContent).toBe('judge-missing not registered')
    const alert = container.querySelector('[role="alert"][data-variant="warn"]')
    expect(alert?.textContent).toContain('judge-missing is not running')
    expect(alert?.textContent).toContain('provider_unavailable')
  })

  it('treats an empty entry as the built-in default and reports it against the registry', async () => {
    const { container } = await mount({}, engine({ functions: [] }))
    expect(container.querySelector('[data-chip="warning"]')?.textContent).toBe('judge-typesafe not registered')
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('judge-typesafe is not running')
  })

  it('surfaces a listing failure with a retry that asks the engine again', async () => {
    const iii = engine(new Error('engine offline'))
    const { container } = await mount({ provider: 'typesafe' }, iii)
    const alert = container.querySelector('[role="alert"][data-variant="warn"]')!
    expect(alert.textContent).toContain('engine offline')
    expect(container.querySelector<HTMLSelectElement>('select[name="provider"]')!.value).toBe('typesafe')
    await act(async () => alert.querySelector('button')!.click())
    expect(iii.trigger).toHaveBeenCalledTimes(2)
  })

  it('shows validation errors on the field and the rest in a panel', async () => {
    const { container } = await mount(
      {},
      engine(),
      { errors: new Map([['/provider', 'must match the pattern'], ['', 'top-level failure']]) },
    )
    expect(container.querySelector('#judge-cfg-provider-error')?.textContent).toBe('must match the pattern')
    expect(container.querySelector('[role="alert"][data-variant="alert"]')?.textContent).toContain('top-level failure')
  })

  it('keeps a non-object value untouched behind an explanation', async () => {
    const { container, onChange, iii } = await mount('${JUDGE_CONFIGURATION}')
    expect(container.textContent).toContain('single value')
    expect(onChange).not.toHaveBeenCalled()
    expect(iii.trigger).toHaveBeenCalledTimes(1)
  })

  it('focuses the provider control on a deep link', async () => {
    const { container } = await mount({}, engine(), { focusField: ['provider'] })
    expect(document.activeElement).toBe(container.querySelector('#judge-cfg-provider'))
  })
})
