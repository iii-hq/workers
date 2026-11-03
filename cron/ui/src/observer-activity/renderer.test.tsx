import { isValidElement } from 'react'
import type { ObserverActivityMessage } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { createCronObserverActivityRenderer, readCronArgs } from '.'

function activity(
  overrides: Partial<ObserverActivityMessage> = {},
): ObserverActivityMessage {
  return {
    id: 'activity-1',
    kind: 'registration',
    functionId: 'cron::on-tick',
    args: { expression: '0 */5 * * * *' },
    delivery: { kind: 'notify' },
    lifecycle: { state: 'active', once: false, fires: 0 },
    ...overrides,
  }
}

describe('cron observer activity renderer', () => {
  const renderer = createCronObserverActivityRenderer()

  it('matches only observers of cron::on-tick', () => {
    expect(renderer.isMatch('cron::on-tick')).toBe(true)
    expect(renderer.isMatch('state::on-changed')).toBe(false)
  })

  it('renders a cron source section for every host-owned activity kind', () => {
    for (const kind of ['registration', 'fired', 'retirement'] as const) {
      expect(isValidElement(renderer.tryRender(activity({ kind })))).toBe(true)
    }
  })

  it('falls through for another function or unusable args', () => {
    expect(
      renderer.tryRender(activity({ functionId: 'state::on-changed' })),
    ).toBeNull()
    expect(renderer.tryRender(activity({ args: {} }))).toBeNull()
    expect(
      renderer.tryRender(activity({ args: { expression: '   ' } })),
    ).toBeNull()
  })

  it('reads the trimmed expression from the args', () => {
    expect(readCronArgs({ expression: ' 0 0 9 * * * ' })).toEqual({
      expression: '0 0 9 * * *',
    })
    expect(readCronArgs(null)).toBeNull()
    expect(readCronArgs({ expression: 5 })).toBeNull()
  })
})
