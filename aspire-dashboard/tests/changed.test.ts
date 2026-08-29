import { describe, expect, it } from 'vitest'
import { type ChangedBinding, type ChangedEvent, createChangedFeed } from '../src/changed.js'

function feedWith(state: { value: unknown }) {
  const sent: Array<{ binding: ChangedBinding; event: ChangedEvent }> = []
  const feed = createChangedFeed(
    () => state.value,
    (binding, event) => sent.push({ binding, event }),
  )
  return { feed, sent }
}

describe('changed feed', () => {
  it('sends one event per binding, carrying the namespace the page registered with', () => {
    const { feed, sent } = feedWith({ value: { state: 'running' } })
    feed.bind('a', { function_id: 'page-a' })
    feed.bind('b', { function_id: 'page-b', namespace: 'tenant-1' })

    feed.emit('dashboard')

    expect(sent.map((item) => item.binding.function_id)).toEqual(['page-a', 'page-b'])
    expect(sent[1].binding.namespace).toBe('tenant-1')
    expect(sent[0].event).toEqual({ reason: 'dashboard', dashboard: { state: 'running' } })
  })

  it('drops a dashboard event whose snapshot is unchanged', () => {
    const state = { value: { state: 'starting' } }
    const { feed, sent } = feedWith(state)
    feed.bind('a', { function_id: 'page-a' })

    feed.emit('dashboard')
    feed.emit('dashboard')
    expect(sent).toHaveLength(1)

    state.value = { state: 'running' }
    feed.emit('dashboard')
    expect(sent).toHaveLength(2)
    expect(sent[1].event.dashboard).toEqual({ state: 'running' })
  })

  it('sends an observability event even when the dashboard snapshot is unchanged', () => {
    // The snapshot says nothing about iii-observability, so the dashboard
    // dedupe must not swallow these.
    const { feed, sent } = feedWith({ value: { state: 'running' } })
    feed.bind('a', { function_id: 'page-a' })

    feed.emit('dashboard')
    feed.emit('observability')
    feed.emit('observability')

    expect(sent.map((item) => item.event.reason)).toEqual(['dashboard', 'observability', 'observability'])
  })

  it('stops sending to a binding once the page unbinds', () => {
    const state = { value: { state: 'running' } }
    const { feed, sent } = feedWith(state)
    feed.bind('a', { function_id: 'page-a' })
    feed.unbind('a')

    feed.emit('dashboard')
    expect(sent).toHaveLength(0)
  })
})
