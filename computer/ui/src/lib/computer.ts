import type { ExtensionIii } from '@iii-dev/console-ui'
import { z } from 'zod'

/**
 * Control plane for the `computer` worker's own injected UI: typed wrappers
 * over its session surface (start / list / stop / screenshot / act) plus the
 * screencast plumbing the live viewport rides on, and parsers for the chat
 * function-trigger views.
 *
 * Every call goes through the tab's `host.iii` client, passed in by the page —
 * there is no module-level singleton in injected UI. Wire source:
 * `computer/src/functions/*.rs` (verbatim ids and payloads).
 */

export const SESSIONS_START_FUNCTION_ID = 'computer::sessions::start'
export const SESSIONS_LIST_FUNCTION_ID = 'computer::sessions::list'
export const SESSIONS_STOP_FUNCTION_ID = 'computer::sessions::stop'
export const DISPLAYS_FUNCTION_ID = 'computer::displays'
export const SCREENSHOT_FUNCTION_ID = 'computer::screenshot'
export const OBSERVE_FUNCTION_ID = 'computer::observe'
export const ACT_FUNCTION_ID = 'computer::act'
export const SCREENCAST_START_FUNCTION_ID = 'computer::screencast::start'
export const SCREENCAST_STOP_FUNCTION_ID = 'computer::screencast::stop'
export const FRAME_FUNCTION_ID = 'computer::frame'

export const SESSION_STARTED_TRIGGER = 'computer::session-started'
export const SESSION_STOPPED_TRIGGER = 'computer::session-stopped'

/** Session lifecycle trigger types the rail re-reads on. */
export const LIFECYCLE_TRIGGERS = [
  SESSION_STARTED_TRIGGER,
  SESSION_STOPPED_TRIGGER,
] as const

/**
 * Stream the worker pushes live desktop frames onto (group = session id). The
 * page subscribes with a `type:'stream'` trigger instead of polling.
 */
export const FRAMES_STREAM = 'computer:frames'

/** Every `computer::*` bus function belongs to this family. */
export function isComputerFunction(functionId: string): boolean {
  return functionId.startsWith('computer::')
}

const screenSchema = z.object({ width: z.number(), height: z.number() })
export type Screen = z.infer<typeof screenSchema>

export const sessionInfoSchema = z.object({
  session_id: z.string(),
  endpoint: z.string(),
  os: z.string(),
  screen: screenSchema,
  created_ms: z.number(),
  last_used_ms: z.number(),
  screencast_active: z.boolean(),
})
export type ComputerSessionInfo = z.infer<typeof sessionInfoSchema>

export const sessionStartSchema = z.object({
  session_id: z.string(),
  endpoint: z.string(),
  os: z.string(),
  screen: screenSchema,
})
export type ComputerSessionStart = z.infer<typeof sessionStartSchema>

export const sessionStopSchema = z.object({
  ok: z.boolean(),
  was_running: z.boolean(),
})

export const actResultSchema = z.object({
  ok: z.boolean(),
  detail: z.string(),
})

export const displayInfoSchema = z.object({
  index: z.number(),
  name: z.string(),
  primary: z.boolean(),
  builtin: z.boolean(),
  width: z.number(),
  height: z.number(),
})
export type ComputerDisplay = z.infer<typeof displayInfoSchema>

const frameSchema = z.object({
  frame: z.string().nullish(),
  mime: z.string(),
  width: z.number(),
  height: z.number(),
  frame_seq: z.number(),
  timestamp: z.number(),
  active: z.boolean(),
})
export type ComputerFrame = z.infer<typeof frameSchema>

/** One pushed screencast frame, as it arrives on the stream. */
const streamFrameSchema = z.object({
  data: z.string(),
  mime: z.string(),
  width: z.number(),
  height: z.number(),
  frame_seq: z.number(),
  timestamp: z.number(),
})
export type ComputerStreamFrame = z.infer<typeof streamFrameSchema>

const contentBlockSchema = z.object({
  type: z.string(),
  mime: z.string().nullish(),
  data: z.string().nullish(),
  text: z.string().nullish(),
})

const captureSchema = z.object({
  content: z.array(contentBlockSchema),
  details: z.object({
    session_id: z.string(),
    mime: z.string(),
    width: z.number().optional(),
    height: z.number().optional(),
    screen: screenSchema.optional(),
  }),
})

export interface ComputerCapture {
  sessionId: string
  dataUrl: string
  width: number
  height: number
  note?: string
}

/**
 * Unwrap a transcript output into the worker's plain result. Through the
 * harness, results arrive as `{content:[{type:'text', text:<stringified
 * result>}, ...], details}`; the text block is authoritative, `details` the
 * fallback. Direct bus results pass through untouched.
 */
export function decodeComputerResult(output: unknown): unknown {
  if (!output || typeof output !== 'object' || Array.isArray(output)) {
    return output
  }
  const obj = output as Record<string, unknown>
  if (!Array.isArray(obj.content)) return output
  for (const block of obj.content) {
    if (!block || typeof block !== 'object') continue
    const b = block as Record<string, unknown>
    if (b.type !== 'text' || typeof b.text !== 'string') continue
    try {
      return JSON.parse(b.text)
    } catch {
      // Not a stringified result; keep looking.
    }
  }
  return 'details' in obj ? obj.details : output
}

/** `screenshot` / `observe` output → a renderable image, or null. */
export function parseCapture(output: unknown): ComputerCapture | null {
  const parsed = captureSchema.safeParse(output)
  if (!parsed.success) return null
  const image = parsed.data.content.find(
    (b) => b.type === 'image' && typeof b.data === 'string',
  )
  if (!image?.data) return null
  const note = parsed.data.content.find(
    (b) => b.type === 'text' && typeof b.text === 'string',
  )?.text
  const details = parsed.data.details
  return {
    sessionId: details.session_id,
    dataUrl: `data:${image.mime ?? details.mime};base64,${image.data}`,
    width: details.width ?? details.screen?.width ?? 0,
    height: details.height ?? details.screen?.height ?? 0,
    note: note ?? undefined,
  }
}

/** Session id carried by a `computer::*` call, from its input or its result. */
export function sessionIdFromCall(
  input: unknown,
  output: unknown,
): string | null {
  if (input && typeof input === 'object') {
    const id = (input as Record<string, unknown>).session_id
    if (typeof id === 'string' && id) return id
  }
  const decoded = decodeComputerResult(output)
  if (decoded && typeof decoded === 'object') {
    const id = (decoded as Record<string, unknown>).session_id
    if (typeof id === 'string' && id) return id
  }
  return null
}

/** A stream push (`{event:{data}}` or `{data}`) → the frame it carries. */
export function extractStreamFrame(raw: unknown): ComputerStreamFrame | null {
  if (!raw || typeof raw !== 'object') return null
  const obj = raw as Record<string, unknown>
  const outer =
    obj.event && typeof obj.event === 'object'
      ? (obj.event as Record<string, unknown>)
      : obj
  const data = 'data' in outer ? outer.data : obj.data
  const parsed = streamFrameSchema.safeParse(data)
  return parsed.success ? parsed.data : null
}

export interface StartSessionInput {
  image?: string
  endpoint?: string
  os?: string
  monitor?: number
}

export async function startSession(
  iii: ExtensionIii,
  input: StartSessionInput,
): Promise<ComputerSessionStart> {
  const payload: Record<string, unknown> = {}
  if (input.image) payload.image = input.image
  if (input.endpoint) payload.endpoint = input.endpoint
  if (input.os) payload.os = input.os
  if (input.monitor != null) payload.monitor = input.monitor
  const res = await iii.trigger<unknown>(SESSIONS_START_FUNCTION_ID, payload, {
    timeoutMs: 120_000,
  })
  return sessionStartSchema.parse(decodeComputerResult(res))
}

export async function listSessions(
  iii: ExtensionIii,
): Promise<ComputerSessionInfo[]> {
  const res = await iii.trigger<unknown>(SESSIONS_LIST_FUNCTION_ID, {})
  const decoded = decodeComputerResult(res)
  const parsed = z
    .object({ sessions: z.array(sessionInfoSchema).optional() })
    .safeParse(decoded)
  return parsed.success ? (parsed.data.sessions ?? []) : []
}

export async function stopSession(
  iii: ExtensionIii,
  sessionId: string,
): Promise<void> {
  await iii.trigger(SESSIONS_STOP_FUNCTION_ID, { session_id: sessionId })
}

export async function listDisplays(
  iii: ExtensionIii,
): Promise<ComputerDisplay[]> {
  const res = await iii.trigger<unknown>(DISPLAYS_FUNCTION_ID, {})
  const parsed = z
    .object({ displays: z.array(displayInfoSchema).optional() })
    .safeParse(decodeComputerResult(res))
  return parsed.success ? (parsed.data.displays ?? []) : []
}

export async function takeScreenshot(
  iii: ExtensionIii,
  sessionId: string,
): Promise<ComputerCapture | null> {
  const res = await iii.trigger<unknown>(SCREENSHOT_FUNCTION_ID, {
    session_id: sessionId,
  })
  return parseCapture(res)
}

export type ActPayload = Record<string, unknown> & { action: string }

export async function act(
  iii: ExtensionIii,
  sessionId: string,
  payload: ActPayload,
): Promise<void> {
  await iii.trigger(ACT_FUNCTION_ID, { session_id: sessionId, ...payload })
}

export async function startScreencast(
  iii: ExtensionIii,
  sessionId: string,
): Promise<void> {
  await iii.trigger(SCREENCAST_START_FUNCTION_ID, { session_id: sessionId })
}

export async function stopScreencast(
  iii: ExtensionIii,
  sessionId: string,
): Promise<void> {
  await iii.trigger(SCREENCAST_STOP_FUNCTION_ID, { session_id: sessionId })
}

/**
 * Newest pushed screencast frame; a memory read on the worker, cheap to poll.
 * `frame` is absent while `sinceFrame` is still the newest seq.
 */
export async function readFrame(
  iii: ExtensionIii,
  sessionId: string,
  sinceFrame?: number,
): Promise<ComputerFrame | null> {
  const res = await iii.trigger<unknown>(FRAME_FUNCTION_ID, {
    session_id: sessionId,
    ...(sinceFrame != null ? { since_frame: sinceFrame } : {}),
  })
  const parsed = frameSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}
