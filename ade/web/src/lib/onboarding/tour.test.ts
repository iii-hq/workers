import { beforeEach, describe, expect, it, vi } from 'vitest'

const harness = vi.hoisted(() => ({
  registered: false,
  commands: [] as {
    key: string
    source: 'worker' | 'page'
    command: { run: () => void }
  }[],
  listeners: new Set<() => void>(),
  add: vi.fn(async () => ({ note: 'onboarding running' })),
  wait: vi.fn(async () => true),
  panel: vi.fn(),
}))

vi.mock('@/lib/ui-slots', () => ({
  getExtPage: (id: string) =>
    harness.registered && id === 'onboarding' ? { id } : undefined,
  whenExtPage: harness.wait,
}))
vi.mock('@/lib/page-commands', () => ({
  getPageCommands: () => harness.commands,
  subscribePageCommands: (listener: () => void) => {
    harness.listeners.add(listener)
    return () => harness.listeners.delete(listener)
  },
}))
vi.mock('@/lib/panel-context', () => ({ requestPanelOpen: harness.panel }))
vi.mock('./api', () => ({ addWorkersWithProgress: harness.add }))

import { openTour, prepareTour, TOUR_PAGE_TIMEOUT_MS } from './tour'

function registerCommand(run = vi.fn()) {
  harness.commands = [
    { key: 'onboarding.open', source: 'worker', command: { run } },
  ]
  for (const listener of [...harness.listeners]) listener()
  return run
}

beforeEach(() => {
  harness.registered = false
  harness.commands = []
  harness.listeners.clear()
  harness.add.mockClear()
  harness.wait.mockClear()
  harness.panel.mockClear()
})

describe('prepareTour', () => {
  it('adds the onboarding worker and waits for its command or page', async () => {
    harness.wait.mockImplementationOnce(() => new Promise(() => undefined))
    const ready = prepareTour()
    await vi.waitFor(() => expect(harness.add).toHaveBeenCalled())
    expect(harness.add).toHaveBeenCalledWith(
      ['onboarding'],
      expect.any(Function),
    )
    expect(harness.wait).toHaveBeenCalledWith(
      'onboarding',
      TOUR_PAGE_TIMEOUT_MS,
    )
    registerCommand()
    await expect(ready).resolves.toBeUndefined()
  })

  it('adds nothing when the tour command is already there', async () => {
    registerCommand()
    await prepareTour()
    expect(harness.add).not.toHaveBeenCalled()
  })

  it('adds nothing when an older worker’s page is already there', async () => {
    harness.registered = true
    await prepareTour()
    expect(harness.add).not.toHaveBeenCalled()
    expect(harness.wait).not.toHaveBeenCalled()
  })

  it('passes a failed add on, for the step to show', async () => {
    harness.add.mockRejectedValueOnce(new Error('compose is not running'))
    await expect(prepareTour()).rejects.toThrow('compose is not running')
    expect(harness.wait).not.toHaveBeenCalled()
  })
})

describe('openTour', () => {
  it('runs the worker’s own open command', () => {
    const run = registerCommand()
    openTour()
    expect(run).toHaveBeenCalledOnce()
    expect(harness.panel).not.toHaveBeenCalled()
  })

  it('falls back to opening the page by id', () => {
    openTour()
    expect(harness.panel).toHaveBeenCalledWith({ pageId: 'onboarding' })
  })
})
