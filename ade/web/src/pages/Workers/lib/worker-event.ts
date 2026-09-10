import { z } from 'zod'

/**
 * Wire shape for the engine `worker` trigger type (`WorkerCallRequest`).
 * Source of truth: `iii trigger engine::triggers::info id=worker`.
 */
export const workerEventSchema = z.object({
  operation: z.string(),
  stage: z.string(),
  worker: z.string(),
  timestamp_ms: z.number(),
  caller_mode: z.enum(['cli', 'trigger']).optional(),
  version: z.string().nullable().optional(),
  status: z.string().nullable().optional(),
  progress: z.number().nullable().optional(),
  source: z
    .object({
      kind: z.enum(['registry', 'oci', 'local']),
      ref: z.string(),
    })
    .nullable()
    .optional(),
  error: z
    .object({
      code: z.string(),
      message: z.string(),
    })
    .nullable()
    .optional(),
})

export type WorkerEvent = z.infer<typeof workerEventSchema>

export function parseWorkerEvent(payload: unknown): WorkerEvent | null {
  const parsed = workerEventSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}
