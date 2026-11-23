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

  it('publishes keys with defaults as optional, so a value saved before needs_you existed validates', () => {
    const schema = z.toJSONSchema(configSchema, { io: 'input' }) as { required?: string[] }
    expect(schema.required ?? []).not.toContain('needs_you')
    expect(z.toJSONSchema(configSchema).required).toContain('needs_you')
  })
})
