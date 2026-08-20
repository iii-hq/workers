import { isValidElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { RegisteredTriggerActivityRenderer } from '@/lib/ui-slots'
import type {
  TriggerActivityMessage,
  TriggerActivityRenderer,
} from '@/types/injectable-ui'
import {
  firstRenderedTriggerActivity,
  triggerActivityRenderers,
} from './renderer-registry'

function activity(
  extra: Partial<TriggerActivityMessage> = {},
): TriggerActivityMessage {
  return {
    id: 'activity-1',
    kind: 'fired',
    triggerType: 'cron',
    delivery: { kind: 'notify' },
    lifecycle: { state: 'active', once: false, fires: 1 },
    ...extra,
  }
}

function registration(
  renderer: TriggerActivityRenderer,
  path = 'cron/page.js',
): RegisteredTriggerActivityRenderer {
  return {
    renderer,
    path,
    scope: path.split('/')[0],
  }
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('trigger activity renderer registry', () => {
  it('preserves registration order and falls through a null renderer', () => {
    const calls: string[] = []
    const renderers = triggerActivityRenderers([
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
    const rendered = firstRenderedTriggerActivity(renderers, activity())

    expect(rendered?.renderer.id).toBe('second')
    expect(calls).toEqual(['first', 'second'])
    const html = renderToStaticMarkup(rendered?.node)
    expect(html).toContain('data-iii-ui="scheduler"')
    expect(html).toContain('every weekday at 09:00')
  })

  it('returns null when every matching renderer falls through', () => {
    const renderers = triggerActivityRenderers([
      registration({
        id: 'cron-null',
        isMatch: (triggerType) => triggerType === 'cron',
        tryRender: () => null,
      }),
      registration({
        id: 'other-source',
        isMatch: (triggerType) => triggerType === 'state',
        tryRender: () => <span>state source</span>,
      }),
    ])

    expect(firstRenderedTriggerActivity(renderers, activity())).toBeNull()
  })

  it('skips a renderer whose matcher throws', () => {
    const brokenRender = vi.fn(() => <span>should not run</span>)
    const renderers = triggerActivityRenderers([
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

    const rendered = firstRenderedTriggerActivity(renderers, activity())
    expect(rendered?.renderer.id).toBe('healthy')
    expect(brokenRender).not.toHaveBeenCalled()
  })

  it('contains a renderer throw inside the extension error chip', () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const afterBroken = vi.fn(() => <span>must not take over</span>)
    const renderers = triggerActivityRenderers([
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

    const rendered = firstRenderedTriggerActivity(renderers, activity())
    expect(rendered?.renderer.id).toBe('broken-render')
    expect(afterBroken).not.toHaveBeenCalled()
    const html = renderToStaticMarkup(rendered?.node)
    expect(html).toContain('extension crashed · broken/page.js')
    expect(html).toContain('title="bad schedule"')
  })

  it('remounts the render boundary when a worker renderer reloads', () => {
    const renderer: TriggerActivityRenderer = {
      id: 'cron-source',
      isMatch: () => true,
      tryRender: () => <span>schedule</span>,
    }
    const first = firstRenderedTriggerActivity(
      triggerActivityRenderers([registration(renderer)]),
      activity(),
    )
    const reloaded = firstRenderedTriggerActivity(
      triggerActivityRenderers([registration(renderer)]),
      activity(),
    )
    expect(isValidElement(first?.node)).toBe(true)
    expect(isValidElement(reloaded?.node)).toBe(true)
    if (!isValidElement(first?.node) || !isValidElement(reloaded?.node)) return
    expect(first.node.key).not.toBe(reloaded.node.key)
  })
})
