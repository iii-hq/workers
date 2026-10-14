import { beforeEach, describe, expect, it, vi } from 'vitest'

const trigger = vi.fn(async () => ({}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger }),
}))

import {
  reportSetupStep,
  SETUP_TOUR_ID,
  STEP_TOPIC,
  setupStepMessage,
} from './telemetry'

beforeEach(() => trigger.mockClear())

describe('setup telemetry', () => {
  it('publishes a closed step on the topic the engine forwards', async () => {
    reportSetupStep('models', 2, {
      outcome: 'connected',
      providers: ['claude-code'],
      provider_count: 1,
    })
    await vi.waitFor(() => expect(trigger).toHaveBeenCalledTimes(1))
    expect(trigger).toHaveBeenCalledWith(
      'iii::durable::publish',
      {
        topic: STEP_TOPIC,
        data: {
          tour_id: SETUP_TOUR_ID,
          tour_title: 'Set up the harness',
          step_id: 'models',
          step_title: 'Models',
          step_number: 2,
          outcome: 'connected',
          providers: ['claude-code'],
          provider_count: 1,
        },
      },
      { timeoutMs: 10_000 },
    )
  })

  it('keeps the step identity over any property of the same name', () => {
    const message = setupStepMessage('judge', 3, {
      step_id: 'spoofed',
      tour_id: 'other',
      outcome: 'skipped',
    })
    expect(message.step_id).toBe('judge')
    expect(message.tour_id).toBe(SETUP_TOUR_ID)
  })

  it('never throws when the engine cannot take it', async () => {
    trigger.mockRejectedValueOnce(new Error('no queue worker'))
    expect(() => reportSetupStep('welcome', 1)).not.toThrow()
    await vi.waitFor(() => expect(trigger).toHaveBeenCalled())
  })
})
