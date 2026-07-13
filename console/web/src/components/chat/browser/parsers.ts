import { z } from 'zod'
import {
  consoleReadSchema,
  decodeBrowserResult,
  networkReadSchema,
  sessionInfoSchema,
  sessionStartSchema,
} from '@/lib/browser'

/**
 * Zod schemas + decode helpers for the `browser::*` chat views.
 *
 * Wire source: workers/browser/src/functions/*.rs (Serialize structs).
 * Through the harness, outputs arrive as the result envelope
 * `{content:[{type:'text', text:<stringified result>}], details}`;
 * `decodeBrowserResult` (lib/browser) unwraps it, JSON.parsing the text
 * block. Schemas are non-strict so additive wire fields don't break the UI.
 */

export { decodeBrowserResult }

export const snapshotResultSchema = z.object({
  url: z.string(),
  title: z.string().nullable().optional(),
  tree: z.string(),
  truncated: z.boolean(),
})
export type SnapshotResult = z.infer<typeof snapshotResultSchema>

export const sessionStopResultSchema = z.object({
  ok: z.boolean(),
  was_running: z.boolean(),
})
export type SessionStopResult = z.infer<typeof sessionStopResultSchema>

export const sessionListResultSchema = z.object({
  sessions: z.array(sessionInfoSchema),
})
export type SessionListResult = z.infer<typeof sessionListResultSchema>

export const navigateResultSchema = z.object({
  ok: z.boolean(),
  url: z.string(),
  title: z.string().nullable().optional(),
  timed_out: z.boolean(),
})
export type NavigateResult = z.infer<typeof navigateResultSchema>

export const actResultSchema = z.object({
  ok: z.boolean(),
  detail: z.string(),
})
export type ActResult = z.infer<typeof actResultSchema>

export const historyResultSchema = z.object({
  ok: z.boolean(),
  url: z.string(),
  moved: z.boolean(),
})
export type HistoryResult = z.infer<typeof historyResultSchema>

const stylePropertySchema = z.object({
  name: z.string(),
  value: z.string(),
})
export type StyleProperty = z.infer<typeof stylePropertySchema>

export const stylesReadResultSchema = z.object({
  ref: z.string(),
  properties: z.array(stylePropertySchema),
  inline_style: z.string().nullable().optional(),
})
export type StylesReadResult = z.infer<typeof stylesReadResultSchema>

export const stylesWriteResultSchema = z.object({
  ok: z.boolean(),
  inline_style: z.string(),
})
export type StylesWriteResult = z.infer<typeof stylesWriteResultSchema>

export interface DomNode {
  ref: string
  tag: string
  id?: string | null
  classes?: string | null
  text?: string | null
  child_count: number
  children: DomNode[]
}

const domNodeSchema: z.ZodType<DomNode> = z.lazy(() =>
  z.object({
    ref: z.string(),
    tag: z.string(),
    id: z.string().nullable().optional(),
    classes: z.string().nullable().optional(),
    text: z.string().nullable().optional(),
    child_count: z.number(),
    children: z.array(domNodeSchema).default([]),
  }),
)

export const domReadResultSchema = z.object({
  root: domNodeSchema,
  truncated: z.boolean(),
})
export type DomReadResult = z.infer<typeof domReadResultSchema>

export const evaluateResultSchema = z.object({
  ok: z.boolean(),
  value: z.unknown().optional(),
  error: z.string().nullable().optional(),
})
export type EvaluateResult = z.infer<typeof evaluateResultSchema>

export { consoleReadSchema, networkReadSchema, sessionStartSchema }

export function safeDecode<T>(schema: z.ZodType<T>, output: unknown): T | null {
  const parsed = schema.safeParse(decodeBrowserResult(output))
  return parsed.success ? parsed.data : null
}

export function safeParseInput<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}
