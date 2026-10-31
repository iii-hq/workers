import { isValidElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { RegisteredObserverActivityRenderer } from '@/lib/ui-slots'
import type {
  ObserverActivityMessage,
  ObserverActivityRenderer,
} from '@/types/injectable-ui'
import {
  firstRenderedObserverActivity,
  firstRenderedObserverActivitySlot,
  OBSERVER_RAW_REDACTION_FAILED,
  observerActivityRawRedactor,
  observerActivityRenderers,
} from './renderer-registry'

function activity(
  extra: Partial<ObserverActivityMessage> = {},
): ObserverActivityMessage {
  return {
    id: 'activity-1',
    kind: 'fired',
    functionId: 'cron::on-tick',
    delivery: { kind: 'notify' },
    lifecycle: { state: 'active', once: false, fires: 1 },
    ...extra,
  }
}

function registration(
  renderer: ObserverActivityRenderer,
  path = 'cron/page.js',
): RegisteredObserverActivityRenderer {
  return {
    renderer,
    path,
    scope: path.split('/')[0],
  }
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('observer activity renderer registry', () => {
  it('preserves registration order and falls through a null renderer', () => {
    const calls: string[] = []
    const renderers = observerActivityRenderers([
      registration({
        id: 'first',
        isMatch: () => true,
        tryRender: () => {
          calls.push('first')
          return null
        },
      }),
      registration(
        {
          id: 'second',
          isMatch: () => true,
          tryRender: () => {
            calls.push('second')
            return <span>every weekday at 09:00</span>
          },
        },
        'scheduler/page.js',
      ),
      registration({
        id: 'third',
        isMatch: () => true,
        tryRender: () => {
          calls.push('third')
          return <span>too late</span>
        },
      }),
    ])

    expect(renderers.map((renderer) => renderer.id)).toEqual([
      'first',
      'second',
      'third',
    ])
    const rendered = firstRenderedObserverActivity(renderers, activity())

    expect(rendered?.renderer.id).toBe('second')
    expect(calls).toEqual(['first', 'second'])
    const html = renderToStaticMarkup(rendered?.node)
    expect(html).toContain('data-iii-ui="scheduler"')
    expect(html).toContain('every weekday at 09:00')
  })

  it('returns null when every matching renderer falls through', () => {
    const renderers = observerActivityRenderers([
      registration({
        id: 'cron-null',
        isMatch: (functionId) => functionId === 'cron::on-tick',
        tryRender: () => null,
      }),
      registration({
        id: 'other-source',
        isMatch: (functionId) => functionId === 'state::on-changed',
        tryRender: () => <span>state source</span>,
      }),
    ])

    expect(firstRenderedObserverActivity(renderers, activity())).toBeNull()
  })

  it('skips a renderer whose matcher throws', () => {
    const brokenRender = vi.fn(() => <span>should not run</span>)
    const renderers = observerActivityRenderers([
      registration({
        id: 'broken-match',
        isMatch: () => {
          throw new Error('bad matcher')
        },
        tryRender: brokenRender,
      }),
      registration({
        id: 'healthy',
        isMatch: () => true,
        tryRender: () => <span>generic schedule</span>,
      }),
    ])

    const rendered = firstRenderedObserverActivity(renderers, activity())
    expect(rendered?.renderer.id).toBe('healthy')
    expect(brokenRender).not.toHaveBeenCalled()
  })

  it('contains a renderer throw inside the extension error chip', () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const afterBroken = vi.fn(() => <span>must not take over</span>)
    const renderers = observerActivityRenderers([
      registration(
        {
          id: 'broken-render',
          isMatch: () => true,
          tryRender: () => {
            throw new Error('bad schedule')
          },
        },
        'broken/page.js',
      ),
      registration({
        id: 'after-broken',
        isMatch: () => true,
        tryRender: afterBroken,
      }),
    ])

    const rendered = firstRenderedObserverActivity(renderers, activity())
    expect(rendered?.renderer.id).toBe('broken-render')
    expect(afterBroken).not.toHaveBeenCalled()
    const html = renderToStaticMarkup(rendered?.node)
    expect(html).toContain('extension crashed · broken/page.js')
    expect(html).toContain('title="bad schedule"')
  })

  it('remounts the render boundary when a worker renderer reloads', () => {
    const renderer: ObserverActivityRenderer = {
      id: 'cron-source',
      isMatch: () => true,
      tryRender: () => <span>schedule</span>,
    }
    const first = firstRenderedObserverActivity(
      observerActivityRenderers([registration(renderer)]),
      activity(),
    )
    const reloaded = firstRenderedObserverActivity(
      observerActivityRenderers([registration(renderer)]),
      activity(),
    )
    expect(isValidElement(first?.node)).toBe(true)
    expect(isValidElement(reloaded?.node)).toBe(true)
    if (!isValidElement(first?.node) || !isValidElement(reloaded?.node)) return
    expect(first.node.key).not.toBe(reloaded.node.key)
  })

  it('exposes worker-owned compact display and full detail slots', () => {
    const renderers = observerActivityRenderers([
      registration({
        id: 'complete-cron',
        isMatch: (functionId) => functionId === 'cron::on-tick',
        tryRender: () => <span>source</span>,
        tryRenderDisplay: () => <span>report became due</span>,
        tryRenderDetails: () => <section>complete cron details</section>,
      }),
    ])
    const display = firstRenderedObserverActivitySlot(
      renderers,
      activity(),
      (renderer) => renderer.tryRenderDisplay?.(activity()) ?? null,
    )
    const details = firstRenderedObserverActivitySlot(
      renderers,
      activity(),
      (renderer) => renderer.tryRenderDetails?.(activity()) ?? null,
    )
    expect(renderToStaticMarkup(display?.node)).toContain('report became due')
    expect(renderToStaticMarkup(details?.node)).toContain(
      'complete cron details',
    )
  })

  it('fails closed when an observer raw-data redactor throws', () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const renderers = observerActivityRenderers([
      registration({
        id: 'secret-cron',
        isMatch: () => true,
        tryRender: () => null,
        redactRaw: () => {
          throw new Error('broken redactor')
        },
      }),
    ])
    const redact = observerActivityRawRedactor(renderers, 'cron::on-tick')
    expect(redact?.({ token: 'secret' })).toBe(OBSERVER_RAW_REDACTION_FAILED)
  })
})
