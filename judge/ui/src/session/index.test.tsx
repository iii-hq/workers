// @vitest-environment jsdom

import type { ExtensionIii } from '@iii-dev/console-ui'
import { act, forwardRef, type ReactNode, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { JudgeSessionPicker, SESSION_PROVIDER_KEY } from './index'

// The console provides these via its import map; the menu opens on mount.
vi.mock('@iii-dev/console-ui', () => ({
  DropdownMenu: ({ children, onOpenChange }: { children: ReactNode; onOpenChange?(open: boolean): void }) => {
    // Open once, on mount (the real menu opens on a click).
    // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only open
    useEffect(() => onOpenChange?.(true), [])
    return <div>{children}</div>
  },
  DropdownMenuTrigger: ({ children, ...props }: { children: ReactNode }) => <button {...props}>{children}</button>,
  DropdownMenuContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SearchField: ({ value, onChange, onKeyDown, ...props }: {
    value: string
    onChange(next: string): void
    onKeyDown?(event: unknown): void
  }) => <input {...props} value={value} onKeyDown={onKeyDown} onChange={(event) => onChange(event.target.value)} />,
  List: forwardRef<HTMLDivElement, { children: ReactNode }>(({ children }, ref) => <div ref={ref}>{children}</div>),
  ListGroupLabel: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  ListItem: ({ label, description, selected, trailing, leading: _l, as, ...props }: {
    label: ReactNode
    description?: ReactNode
    selected?: boolean
    trailing?: ReactNode
    leading?: ReactNode
    as?: 'button' | 'div'
  }) => {
    const Row = as === 'div' ? 'div' : 'button'
    return (
      <Row data-list-item="" data-selected={selected || undefined} {...props}>
        <span data-label>{label}</span>
        {description ? <small>{description}</small> : null}
        {trailing}
      </Row>
    )
  },
  Button: ({ children, variant: _v, size: _s, ...props }: { children: ReactNode; variant?: string; size?: string }) => (
    <button type="button" {...props}>
      {children}
    </button>
  ),
  IconButton: ({ children, label, tooltip, tooltipSide: _s, variant: _v, ...props }: {
    children: ReactNode
    label: string
    tooltip?: ReactNode | false
    tooltipSide?: string
    variant?: string
  }) => (
    <>
      <button type="button" aria-label={label} {...props}>
        {children}
      </button>
      {tooltip ? <div role="tooltip">{tooltip}</div> : null}
    </>
  ),
  uiClasses: { spin: 'spin', motionPanel: 'motion-panel' },
  StatusBar: ({ children }: { children: ReactNode }) => <footer>{children}</footer>,
  WorkerConfigurationDialog: ({ configurationId }: { configurationId: string | null }) =>
    configurationId ? <output data-configuring={configurationId} /> : null,
}))

let root: Root | null = null
let host: HTMLElement | null = null
afterEach(() => {
  vi.unstubAllGlobals()
  act(() => root?.unmount())
  host?.remove()
  root = null
  host = null
})

function engine(registered = ['typesafe', 'semif']) {
  const state = { registered: [...registered], addReply: { status: 'accepted' } as unknown }
  const trigger = vi.fn(async (id: string) => {
    if (id === 'engine::functions::list') {
      return {
        functions: state.registered.map((p) => ({ function_id: `judge-${p}::evaluate`, worker_name: `judge-${p}` })),
      }
    }
    if (id === 'judge::configuration-id') return { id: 'judge' }
    if (id === 'configuration::get') return { value: { provider: 'typesafe' } }
    if (id === 'engine::workers::list') return { workers: [] }
    if (id === 'compose::add') {
      if (state.addReply instanceof Error) throw state.addReply
      return state.addReply
    }
    return {}
  })
  return Object.assign({ trigger }, { state })
}

/** The registry's `tag=evaluation` page: judges plus unrelated workers. */
function registry(ok = true) {
  const page = {
    workers: [
      { name: 'judge-typesafe', version: '0.2.1', description: 'TypeSafe evaluation' },
      { name: 'judge', version: '0.2.1', description: 'Provider-neutral evaluation hub' },
      { name: 'judge-decider', version: '0.1.0', description: 'decider provider for the judge hub' },
      { name: 'judge-semif', version: '0.1.0', description: 'SemIf provider for the judge hub' },
      { name: 'eval', version: '0.2.14', description: 'benchmarks' },
    ],
    pagination: { has_more: false },
  }
  return vi.fn(async (url: string) => {
    expect(url).toBe('https://api.workers.iii.dev/w?tag=evaluation')
    return ok ? new Response(JSON.stringify(page), { status: 200 }) : new Response('down', { status: 503 })
  })
}

async function mount(metadata: Record<string, unknown>, setMetadata = vi.fn(), iii = engine()) {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => {
    root!.render(
      <JudgeSessionPicker
        iii={iii as unknown as Pick<ExtensionIii, 'trigger'>}
        sessionId="s1"
        isStreaming={false}
        metadata={metadata}
        setMetadata={setMetadata}
      />,
    )
  })
  const view = host
  const row = (label: string) =>
    [...view.querySelectorAll<HTMLElement>('[data-list-item]')].find(
      (item) => item.querySelector('[data-label]')?.textContent === label,
    )
  const labels = () => [...view.querySelectorAll('[data-list-item] [data-label]')].map((item) => item.textContent)
  return { view, iii, setMetadata, row, labels }
}

async function type(input: HTMLInputElement, text: string) {
  await act(async () => {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!
    setValue.call(input, text)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('per-session judge provider', () => {
  it("lists Default and the running judges, and writes only this session's metadata key", async () => {
    const { view, iii, setMetadata, row, labels } = await mount({ [SESSION_PROVIDER_KEY]: 'typesafe', model: 'm' })
    expect(view.querySelector('button[aria-label]')?.textContent).toBe('judge · typesafe')
    expect(labels()).toEqual(['Default', 'semif', 'typesafe'])
    expect(row('Default')?.textContent).toContain('typesafe, from judge settings')
    expect(row('typesafe')?.dataset.selected).toBe('true')
    await act(async () => row('semif')!.click())
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: 'semif' })
    // Picking a provider starts its model load before the session's next turn.
    expect(iii.trigger).toHaveBeenCalledWith(
      'judge::models::list',
      { provider: 'semif', timeout_ms: 1_000 },
      { timeoutMs: 10_000 },
    )
    const calls = iii.trigger.mock.calls.length
    await act(async () => row('Default')!.click())
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: undefined })
    expect(iii.trigger.mock.calls.length).toBe(calls)
  })

  it('keeps a stored provider that is not running selectable', async () => {
    const { row } = await mount({ [SESSION_PROVIDER_KEY]: 'laya' })
    expect(row('laya')?.textContent).toContain('Not running')
    expect(row('laya')?.dataset.selected).toBe('true')
  })

  it('filters the judges and picks the first match on Enter', async () => {
    const { view, setMetadata, labels } = await mount({})
    const filter = view.querySelector<HTMLInputElement>('input[aria-label="Filter judges"]')!
    await type(filter, 'sem')
    expect(labels()).toEqual(['semif'])
    await act(async () => {
      filter.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    })
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: 'semif' })
    await type(filter, 'nothing')
    expect(labels()).toEqual([])
    expect(view.textContent).toContain('No judge matches “nothing”.')
  })

  it("opens the judge settings from Configure", async () => {
    const { view } = await mount({})
    const configure = [...view.querySelectorAll('button')].find((button) => button.textContent === 'Configure')!
    await act(async () => configure.click())
    expect(view.querySelector('[data-configuring]')?.getAttribute('data-configuring')).toBe('judge')
  })

  it('adds a registry judge that is not installed and follows it until it registers', async () => {
    vi.stubGlobal('fetch', registry())
    const iii = engine()
    const { view } = await mount({}, vi.fn(), iii)
    await act(async () => view.querySelector<HTMLElement>('button[aria-label="Add a judge"]')!.click())
    await act(async () => {})
    expect(view.textContent).toContain('Add a judge')
    // Installed judges and the hub itself stay off the page.
    const rows = () => [...view.querySelectorAll('[data-list-item] [data-label]')].map((row) => row.textContent)
    expect(rows()).toEqual(['deciderjudge-decider@0.1.0'])
    await act(async () => view.querySelector<HTMLElement>('button[aria-label="Add decider"]')!.click())
    expect(iii.trigger).toHaveBeenCalledWith('compose::add', { workers: ['judge-decider'] }, { timeoutMs: 600_000 })
    expect(view.textContent).toContain('Adding…')
    iii.state.registered.push('decider')
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 3_100))
    })
    expect(view.textContent).toContain('Added')
    await act(async () => view.querySelector<HTMLElement>('button[aria-label="Back to judges"]')!.click())
    expect(rows()).toEqual(['Default', 'decider', 'semif', 'typesafe'])
  }, 10_000)

  it('shows why an add failed and offers a retry', async () => {
    vi.stubGlobal('fetch', registry())
    const iii = engine()
    iii.state.addReply = { status: 'failed', error: { message: 'worker judge-decider not found' } }
    const { view } = await mount({}, vi.fn(), iii)
    await act(async () => view.querySelector<HTMLElement>('button[aria-label="Add a judge"]')!.click())
    await act(async () => {})
    await act(async () => view.querySelector<HTMLElement>('button[aria-label="Add decider"]')!.click())
    await act(async () => {})
    expect(view.querySelector('[role="alert"]')?.textContent).toBe('worker judge-decider not found')
    expect(view.querySelector('button[aria-label="Retry adding decider"]')).not.toBeNull()
  })

  it('says so when the registry is unreachable', async () => {
    vi.stubGlobal('fetch', registry(false))
    const { view } = await mount({})
    await act(async () => view.querySelector<HTMLElement>('button[aria-label="Add a judge"]')!.click())
    await act(async () => {})
    expect(view.textContent).toContain('The workers registry is unreachable right now.')
  })

  it('explains the judge and where this session uses it beside the picker', async () => {
    const { view } = await mount({})
    expect(view.querySelector('button[aria-label="What is the judge?"]')).not.toBeNull()
    const help = view.querySelector('[role="tooltip"]')?.textContent ?? ''
    expect(help).toContain('A fast evaluation model for typed questions')
    expect(help).toContain('ranks functions and skills when the agent searches the catalog')
    expect(help).toContain('checks a function call’s arguments')
    expect(help).toContain('browser::run')
  })
})
