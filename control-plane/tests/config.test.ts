import { describe, expect, it } from 'vitest'
import { z } from 'zod'
import { configSchema } from '../src/config.js'

describe('configuration schema', () => {
  it('keeps validating a saved value that still carries the retired at_risk_days key', () => {
    const saved = {
      stall_after_min: 30,
      summary: { interval_min: 10, language: 'English' },
      prs: { watched_interval_min: 5, idle_interval_min: 30 },
      escalate: {},
      at_risk_days: 3,
    }
    expect(() => configSchema.parse(saved)).not.toThrow()
    const schema = z.toJSONSchema(configSchema)
    expect(schema.properties).toHaveProperty('at_risk_days')
  })
})
