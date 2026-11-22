import { z } from 'zod'
import type { TowerConfig } from './types.js'

const classes = z.enum([
  'security',
  'publish',
  'external_cost',
  'data',
  'scope',
  'run_health',
  'routine',
])
export const configSchema = z.strictObject({
  stall_after_min: z.number().positive().default(30),
  summary: z
    .strictObject({
      model: z
        .string()
        .regex(/^[^:]+::.+$/)
        .optional(),
      interval_min: z.number().positive().default(10),
      language: z.string().min(1).default('English'),
    })
    .prefault({}),
  prs: z
    .strictObject({
      watched_interval_min: z.number().positive().default(5),
      idle_interval_min: z.number().positive().default(30),
    })
    .prefault({}),
  escalate: z.record(z.string(), z.array(classes)).default({}),
  at_risk_days: z.number().nonnegative().default(3),
})
export const defaultConfig = (): TowerConfig => configSchema.parse({})
