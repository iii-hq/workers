import { z } from 'zod'
import type { ControlPlaneConfig } from './types.js'

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
      /** The reasoning effort picked with the model, as the chat stores it (`default`, `lowest`, or a model effort). */
      reasoning: z.string().min(1).optional(),
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
  /** Retired 2026-11-23 with the at-risk rule; still accepted so a saved value keeps validating. Ignored. */
  at_risk_days: z.number().optional(),
})
export const defaultConfig = (): ControlPlaneConfig => configSchema.parse({})
