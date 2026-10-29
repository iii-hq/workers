// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  ContextTrackingBody,
  type ContextTrackingItem,
  safeTrackingUrl,
  useContextTracking,
} from './ContextTracking'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true
const mocks = vi.hoisted(() => ({ trigger: vi.fn() }))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger: mocks.trigger }),
}))
const item: ContextTrackingItem = {
  id: 'release-1',
  scope: 'project',
  owner: '/project',
  title: 'Release v1.8',
  summary: '14 of 20 delivered',
  progress: 70,
  items: [
    { label: 'Tests', done: true },
    { label: 'Ship', done: false },
  ],
  source: { label: 'Release board', url: 'https://example.com/release' },
  updated_at: 1700000000000,
}
let container: HTMLDivElement
let root: Root
beforeEach(() => {
  mocks.trigger.mockReset()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
})
async function render(node: React.ReactNode) {
  await act(async () => {
    root.render(node)
  })
}

describe('Context tracking body', () => {
  it('shows progress, scope, checklist, source and snapshot time without executing content', async () => {
    await render(
      <ContextTrackingBody
        items={[{ ...item, summary: '<script>unsafe()</script>' }]}
        loading={false}
        error={false}
        retry={() => {}}
      />,
    )
    const progress = container.querySelector('[role="progressbar"]')
    expect(progress?.getAttribute('aria-valuenow')).toBe('70')
    expect(container.textContent).toContain('Project')
    expect(container.textContent).toContain('Completed:')
    expect(container.textContent).toContain('Pending:')
    expect(container.textContent).toContain('<script>unsafe()</script>')
    expect(container.querySelector('script')).toBeNull()
    expect(container.querySelector('a')?.getAttribute('rel')).toBe(
      'noopener noreferrer',
    )
    expect(container.querySelector('time')?.getAttribute('datetime')).toBe(
      new Date(item.updated_at).toISOString(),
    )
    expect(container.textContent).toContain('not monitored live')
  })
  it('prepares update and remove drafts without sending or changing data', async () => {
    const ask = vi.fn()
    await render(
      <ContextTrackingBody
        items={[item]}
        loading={false}
        error={false}
        retry={() => {}}
        onAskAgent={ask}
      />,
    )
    const buttons = [...container.querySelectorAll('button')]
    await act(async () => {
      buttons[0].click()
      buttons[1].click()
    })
    expect(ask).toHaveBeenNthCalledWith(
      1,
      expect.stringContaining('release-1'),
      false,
    )
    expect(ask).toHaveBeenNthCalledWith(
      2,
      expect.stringContaining('Remove Context tracking'),
      false,
    )
    expect(mocks.trigger).not.toHaveBeenCalled()
  })
  it('supports loading, empty and retry while retaining previous snapshots', async () => {
    const retry = vi.fn()
    await render(
      <ContextTrackingBody items={[]} loading error={false} retry={retry} />,
    )
    expect(
      container.querySelector('[aria-label="Loading tracking"]'),
    ).not.toBeNull()
    await render(
      <ContextTrackingBody
        items={[]}
        loading={false}
        error={false}
        retry={retry}
      />,
    )
    expect(container.textContent).toContain('Ask in chat')
    await render(
      <ContextTrackingBody
        items={[item]}
        loading={false}
        error
        retry={retry}
      />,
    )
    expect(container.textContent).toContain('Release v1.8')
    expect(container.textContent).toContain('out of date')
    await act(async () => {
      container.querySelector('button')?.click()
    })
    expect(retry).toHaveBeenCalledOnce()
  })
  it('rejects unsafe and credential-bearing URLs', () => {
    for (const url of [
      'javascript:alert(1)',
      'file:///tmp/file',
      '/relative',
      'https://u:p@example.com',
    ])
      expect(safeTrackingUrl(url)).toBeNull()
    expect(safeTrackingUrl('https://example.com')).toBe('https://example.com/')
  })
})

function Probe({
  sessionId,
  active = true,
  status = 'idle',
}: {
  sessionId: string
  active?: boolean
  status?: string
}) {
  const state = useContextTracking({
    sessionId,
    workingDir: '/project',
    status,
    active,
    connectionState: 'connected',
  })
  return (
    <div>
      {state.items.map((i) => i.title).join(',')}
      {state.error ? 'error' : ''}
    </div>
  )
}
it('does not read while hidden, reloads at turn completion and ignores old-session responses', async () => {
  mocks.trigger.mockResolvedValue({ items: [item] })
  await render(<Probe sessionId="a" active={false} />)
  expect(mocks.trigger).not.toHaveBeenCalled()
  await render(<Probe sessionId="a" status="working" />)
  expect(container.textContent).toContain(item.title)
  await render(<Probe sessionId="a" status="idle" />)
  expect(mocks.trigger).toHaveBeenCalledTimes(2)
  let resolveOld!: (value: unknown) => void
  mocks.trigger.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        resolveOld = resolve
      }),
  )
  await render(<Probe sessionId="old" />)
  mocks.trigger.mockResolvedValueOnce({
    items: [{ ...item, title: 'Current session' }],
  })
  await render(<Probe sessionId="new" />)
  await act(async () => {
    resolveOld({ items: [{ ...item, title: 'Old secret' }] })
  })
  expect(container.textContent).toBe('Current session')
})
