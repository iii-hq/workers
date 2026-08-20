import { isValidElement } from 'react'
import type { TriggerActivityMessage } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { createCronTriggerActivityRenderer, readCronConfig } from '.'

function activity(
  overrides: Partial<TriggerActivityMessage> = {},
): TriggerActivityMessage {
  return {
    id: 'activity-1',
    kind: 'registration',
    triggerType: 'cron',
    config: { expression: '0 */5 * * * *' },
    delivery: { kind: 'notify' },
    lifecycle: { state: 'active', once: false, fires: 0 },
    ...overrides,
  }
}

describe('cron trigger activity renderer', () => {
  const renderer = createCronTriggerActivityRenderer()

  it('matches only the cron trigger type', () => {
    expect(renderer.isMatch('cron')).toBe(true)
    expect(renderer.isMatch('state')).toBe(false)
  })

  it('renders a cron source section for every host-owned activity kind', () => {
    for (const kind of ['registration', 'fired', 'retirement'] as const) {
      expect(isValidElement(renderer.tryRender(activity({ kind })))).toBe(true)
    }
  })

  it('falls through for another type or an unusable cron config', () => {
    expect(
      renderer.tryRender(activity({ triggerType: 'state' })),
    ).toBeNull()
    expect(renderer.tryRender(activity({ config: {} }))).toBeNull()
    expect(
      renderer.tryRender(activity({ config: { expression: '   ' } })),
    ).toBeNull()
  })

  it('retains the optional condition function id', () => {
    expect(
      readCronConfig({
        expression: '0 0 9 * * *',
        condition_function_id: 'gates::weekday',
      }),
    ).toEqual({
      expression: '0 0 9 * * *',
      conditionFunctionId: 'gates::weekday',
    })
  })
})
