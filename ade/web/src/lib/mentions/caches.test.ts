import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getMentionProvidersSnapshot, loadMentionProviders } from './providers'
import {
  iiiMentionRuntime,
  type MentionRuntime,
  setMentionRuntime,
} from './runtime'
import { searchMentionProvider } from './search'
import type { MentionProvider, MentionView } from './types'
import {
  getMentionViewState,
  primeMentionView,
  requestMentionView,
} from './views'

const kanban: MentionProvider = {
  v: 1,
  name: 'kanban',
  label: 'Tickets',
  search: 'kanban::mention::search',
  getFunctionId: 'kanban::mention::get',
}

function fakeRuntime(views: Record<string, MentionView | null | Error>) {
  const runtime = {
    listProviders: vi.fn(async () => [kanban]),
    search: vi.fn(async () => [{ id: 'u1', label: 'Fix login' }]),
    get: vi.fn(async (_provider: MentionProvider, id: string) => {
      const view = views[id]
      if (view instanceof Error) throw view
      return view ?? null
    }),
  } satisfies MentionRuntime
  setMentionRuntime(runtime)
  return runtime
}

beforeEach(() => {
  vi.useRealTimers()
})

afterEach(() => {
  setMentionRuntime(iiiMentionRuntime)
})

describe('mention providers', () => {
  it('load once while fresh, and reload when forced', async () => {
    const runtime = fakeRuntime({})
    await Promise.all([loadMentionProviders(), loadMentionProviders()])
    await loadMentionProviders()
    expect(runtime.listProviders).toHaveBeenCalledTimes(1)
    expect(getMentionProvidersSnapshot().byName.get('kanban')).toBe(kanban)
    await loadMentionProviders({ force: true })
    expect(runtime.listProviders).toHaveBeenCalledTimes(2)
  })

  it('keep the last good list when a reload fails', async () => {
    const runtime = fakeRuntime({})
    await loadMentionProviders()
    runtime.listProviders.mockRejectedValueOnce(new Error('down'))
    await loadMentionProviders({ force: true })
    expect(getMentionProvidersSnapshot()).toMatchObject({ status: 'ready' })
    expect(getMentionProvidersSnapshot().providers).toEqual([kanban])
  })
})

describe('mention views', () => {
  it('resolve, report missing ids and unknown providers', async () => {
    fakeRuntime({ u1: { id: 'u1', label: 'Fix login' } })
    await requestMentionView('kanban', 'u1')
    expect(getMentionViewState('kanban', 'u1')).toEqual({
      status: 'ready',
      view: { id: 'u1', label: 'Fix login' },
    })
    await requestMentionView('kanban', 'gone')
    expect(getMentionViewState('kanban', 'gone')).toEqual({ status: 'missing' })
    await requestMentionView('calendar', 'x')
    expect(getMentionViewState('calendar', 'x')).toEqual({
      status: 'unknown-provider',
    })
  })

  it('share one call between concurrent readers and cache the answer', async () => {
    const runtime = fakeRuntime({ u1: { id: 'u1', label: 'Fix login' } })
    await Promise.all([
      requestMentionView('kanban', 'u1'),
      requestMentionView('kanban', 'u1'),
    ])
    await requestMentionView('kanban', 'u1')
    expect(runtime.get).toHaveBeenCalledTimes(1)
  })

  it('show a primed search row at once, then refresh it', async () => {
    const runtime = fakeRuntime({
      u1: { id: 'u1', label: 'Fix login (fresh)' },
    })
    primeMentionView('kanban', { id: 'u1', label: 'Fix login', hint: 'KAN-1' })
    expect(getMentionViewState('kanban', 'u1')).toMatchObject({
      status: 'ready',
      view: { label: 'Fix login', hint: 'KAN-1' },
    })
    await requestMentionView('kanban', 'u1')
    expect(runtime.get).toHaveBeenCalledTimes(1)
    expect(getMentionViewState('kanban', 'u1')).toMatchObject({
      view: { label: 'Fix login (fresh)' },
    })
  })

  it('keep a shown view when its refresh fails', async () => {
    fakeRuntime({ u1: new Error('worker restarting') })
    primeMentionView('kanban', { id: 'u1', label: 'Fix login' })
    await requestMentionView('kanban', 'u1')
    expect(getMentionViewState('kanban', 'u1')).toMatchObject({
      status: 'ready',
      view: { label: 'Fix login' },
    })
  })
})

describe('mention search', () => {
  it('shares identical asks and does not cache failures', async () => {
    const runtime = fakeRuntime({})
    const ask = () => searchMentionProvider(kanban, ' fix ', { limit: 4 })
    await Promise.all([ask(), ask()])
    expect(runtime.search).toHaveBeenCalledTimes(1)
    expect(runtime.search).toHaveBeenCalledWith(kanban, {
      query: 'fix',
      limit: 4,
      context: undefined,
    })

    runtime.search.mockRejectedValueOnce(new Error('boom'))
    await expect(
      searchMentionProvider(kanban, 'other', { limit: 4 }),
    ).rejects.toThrow('boom')
    await searchMentionProvider(kanban, 'other', { limit: 4 })
    expect(runtime.search).toHaveBeenCalledTimes(3)
  })

  it('gives up on a provider that does not answer', async () => {
    vi.useFakeTimers()
    const runtime = fakeRuntime({})
    runtime.search.mockImplementationOnce(() => new Promise(() => {}))
    const pending = searchMentionProvider(kanban, 'slow', { limit: 4 })
    const settled = expect(pending).rejects.toThrow('timed out')
    await vi.advanceTimersByTimeAsync(3_000)
    await settled
  })
})
