// Shared by every page suite. Each suite still declares its two mocks itself (vi.mock is per file):
//   vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
//   vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))
// then calls `usePage()` once.
import type { JsonValue, PageRenderProps } from '@iii-dev/console-ui'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, vi } from 'vitest'
import { clearAsked } from '../asked'
import { CloudPage } from '../CloudPage'
import { type Board, boards } from '../fixtures'
import type { Status } from '../types'
import { ui } from './hooks-mock'
import { fakeHost } from './host'

export { ui }
export const onClose = vi.fn()
/** The mounted page's container and root, fresh for every test (live bindings: read them inside tests). */
export let el: HTMLDivElement
export let root: Root

export function usePage() {
  beforeEach(() => {
    clearAsked()
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
    ui.narrow = false
    ui.panes = {}
    el = document.createElement('div')
    document.body.append(el)
    root = createRoot(el)
  })
  afterEach(() => {
    act(() => root.unmount())
    el.remove()
  })
}

export const lastCall = (calls: { id: string }[]) =>
  calls.filter((c) => c.id !== 'iii-cloud::status').at(-1) as { id: string; payload?: Record<string, unknown> }
export const flush = () => act(async () => void (await new Promise((r) => setTimeout(r, 0))))
export const text = () => el.textContent ?? ''
export const click = (node: Element | null | undefined) => act(async () => void (node as HTMLElement).click())
export const byText = (sel: string, t: string) => [...el.querySelectorAll(sel)].find((n) => n.textContent?.trim() === t)
export const btn = (t: string) => byText('button', t) ?? el.querySelector(`button[aria-label="${t}"]`)
/** A row of a list (the mock renders ListItem as `[role=row]`) whose text starts with `t`. */
export const row = (t: string) => [...el.querySelectorAll('[role=row]')].find((r) => r.textContent?.startsWith(t))

/** Types into a controlled input the way React sees it. */
export async function type(input: Element | null, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, value)
    input?.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

/** `panelContext` for a mount: a route or an old `{tab}` context. */
export const ctx = (context: unknown, id = 1): Partial<PageRenderProps> => ({
  panelContext: { id, pageId: 'iii-cloud', context: context as JsonValue },
})

/**
 * Answers `iii-cloud::api` (and `::write`) from `map`, keyed by the full path (with `?query` when the call
 * has one, else the bare path); a path not in the map answers 404 the way iii cloud does.
 */
export function cloudAnswers(map: Record<string, unknown>) {
  const answer = (p: Record<string, unknown>) => {
    const path = String(p.path)
    const q = p.query ? `?${new URLSearchParams(p.query as Record<string, string>)}` : ''
    const key = `${path}${q}` in map ? `${path}${q}` : path
    return key in map
      ? { status: 200, body: map[key] }
      : { status: 404, body: { code: 'NOT_FOUND', message: 'No such environment, deployment or resource.' } }
  }
  return { 'iii-cloud::api': answer, 'iii-cloud::api::write': answer }
}

export async function mount(
  status: Status,
  extra: Partial<PageRenderProps> = {},
  answers: Parameters<typeof fakeHost>[1] = {},
) {
  const h = fakeHost(status, answers)
  await act(async () =>
    root.render(<CloudPage host={h.host} panelSide="right" tabId="t" onRequestClose={onClose} {...extra} />),
  )
  await flush()
  return h
}

/** Mounts a board with its context and its cloud answers (`answers` adds or overrides). */
export const board = (id: keyof typeof boards, extra: Partial<PageRenderProps> = {}, answers = {}) => {
  // Typed as a Board: an area module keyed with `satisfies` keeps each board's narrow type.
  const b: Board = boards[id]
  // A board's `values` is Setup's Values step, which the route no longer names: the pane remembers it.
  if (b.values && b.context?.at === 'setup') ui.panes['iii-cloud:setup-step:t'] = 'values'
  return mount(
    b.status,
    { ...(b.context ? ctx(b.context) : {}), ...extra },
    {
      ...cloudAnswers(b.cloud ?? {}),
      ...(b.values ? { 'iii-cloud::values': b.values } : {}),
      ...(b.configImport ? { 'iii-cloud::config::import': b.configImport } : {}),
      ...answers,
    },
  )
}
