import { z } from 'zod'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Control plane for the optional `browser` worker: typed wrappers over its
 * session surface (start / list / stop / navigate / screenshot / act /
 * console / network / pick) plus parsers and formatting helpers for the
 * Browser page, the chat function-call view, and the pick-to-chat flow.
 * Everything here is gated by the caller on browser presence
 * (`use-browser-status`).
 */

export const BROWSER_SESSIONS_START_FUNCTION_ID = 'browser::sessions::start'
export const BROWSER_SESSIONS_LIST_FUNCTION_ID = 'browser::sessions::list'
export const BROWSER_SESSIONS_STOP_FUNCTION_ID = 'browser::sessions::stop'
export const BROWSER_NAVIGATE_FUNCTION_ID = 'browser::navigate'
export const BROWSER_SCREENSHOT_FUNCTION_ID = 'browser::screenshot'
export const BROWSER_ACT_FUNCTION_ID = 'browser::act'
export const BROWSER_CONSOLE_READ_FUNCTION_ID = 'browser::console::read'
export const BROWSER_NETWORK_READ_FUNCTION_ID = 'browser::network::read'
export const BROWSER_PICK_START_FUNCTION_ID = 'browser::pick::start'
export const BROWSER_PICK_RESOLVE_FUNCTION_ID = 'browser::pick::resolve'
export const BROWSER_PICK_STOP_FUNCTION_ID = 'browser::pick::stop'
export const BROWSER_PICK_HINT_FUNCTION_ID = 'browser::pick::hint'
export const BROWSER_SCREENCAST_START_FUNCTION_ID = 'browser::screencast::start'
export const BROWSER_SCREENCAST_STOP_FUNCTION_ID = 'browser::screencast::stop'
export const BROWSER_FRAME_FUNCTION_ID = 'browser::frame'

export const BROWSER_SESSION_STARTED_TRIGGER = 'browser::session-started'
export const BROWSER_SESSION_STOPPED_TRIGGER = 'browser::session-stopped'
export const BROWSER_NAVIGATED_TRIGGER = 'browser::navigated'
export const BROWSER_CONSOLE_EVENT_TRIGGER = 'browser::console-event'
export const BROWSER_NETWORK_EVENT_TRIGGER = 'browser::network-event'
export const BROWSER_PICKED_TRIGGER = 'browser::picked'

/** Stream the worker pushes live viewport frames onto (group = session id).
 * The console subscribes with a `type:'stream'` trigger instead of polling. */
export const BROWSER_FRAMES_STREAM = 'browser:frames'

/** Session lifecycle trigger types the sessions rail re-reads on. */
export const BROWSER_LIFECYCLE_TRIGGERS = [
  BROWSER_SESSION_STARTED_TRIGGER,
  BROWSER_SESSION_STOPPED_TRIGGER,
  BROWSER_NAVIGATED_TRIGGER,
] as const

/** Every `browser::*` bus function belongs to this family. */
export function isBrowserFunction(functionId: string): boolean {
  return functionId.startsWith('browser::')
}

export const sessionInfoSchema = z.object({
  session_id: z.string(),
  url: z.string(),
  title: z.string().optional(),
  headless: z.boolean(),
  created_ms: z.number(),
  last_used_ms: z.number(),
  console_entries: z.number(),
})
export type BrowserSessionInfo = z.infer<typeof sessionInfoSchema>

const sessionListSchema = z.object({
  sessions: z.array(z.unknown()).optional(),
})

export const sessionStartSchema = z.object({
  session_id: z.string(),
  url: z.string(),
  headless: z.boolean(),
})
export type BrowserSessionStart = z.infer<typeof sessionStartSchema>

const consoleEntrySchema = z.object({
  seq: z.number(),
  timestamp: z.number(),
  level: z.string(),
  text: z.string(),
  source: z.string().optional(),
})
export type BrowserConsoleEntry = z.infer<typeof consoleEntrySchema>

export const consoleReadSchema = z.object({
  entries: z.array(consoleEntrySchema),
  last_seq: z.number(),
  dropped: z.number(),
})
export type BrowserConsoleRead = z.infer<typeof consoleReadSchema>

const networkEntrySchema = z.object({
  seq: z.number(),
  timestamp: z.number(),
  method: z.string(),
  url: z.string(),
  status: z.number().nullable().optional(),
  mime_type: z.string().nullable().optional(),
  failed: z.boolean(),
  error: z.string().nullable().optional(),
})
export type BrowserNetworkEntry = z.infer<typeof networkEntrySchema>

export const networkReadSchema = z.object({
  entries: z.array(networkEntrySchema),
  last_seq: z.number(),
  dropped: z.number(),
})
export type BrowserNetworkRead = z.infer<typeof networkReadSchema>

const contentBlockSchema = z.object({
  type: z.string(),
  mime: z.string().optional(),
  data: z.string().optional(),
  text: z.string().optional(),
})

const screenshotDetailsSchema = z.object({
  session_id: z.string(),
  url: z.string(),
  width: z.number(),
  height: z.number(),
})

const screenshotSchema = z.object({
  content: z.array(contentBlockSchema),
  // Through the harness, `details` is the whole worker return, so the real
  // details sit one level deeper at `details.details`; a direct bus call has
  // them at the top. Accept either.
  details: z
    .union([
      screenshotDetailsSchema,
      z.object({ details: screenshotDetailsSchema }),
    ])
    .optional(),
})

export interface BrowserScreenshot {
  /** `data:` URL for the captured JPEG; null when no image block arrived. */
  dataUrl: string | null
  sessionId: string
  url: string
  /** Page viewport size the capture maps to (click coordinates space). */
  width: number
  height: number
}

/**
 * Parse a `browser::screenshot` result into a renderable shape. The image
 * block lives at `content` in both the direct bus result and the harness
 * transcript output; the metadata (`details`) may be nested one level under
 * the harness result envelope, so both shapes are accepted.
 */
export function parseScreenshotOutput(
  payload: unknown,
): BrowserScreenshot | null {
  const parsed = screenshotSchema.safeParse(payload)
  if (!parsed.success) return null
  const image = parsed.data.content.find(
    (block) => block.type === 'image' && block.data,
  )
  const d = parsed.data.details
  const meta = d && 'details' in d ? d.details : d
  return {
    dataUrl: image?.data
      ? `data:${image.mime ?? 'image/jpeg'};base64,${image.data}`
      : null,
    sessionId: meta?.session_id ?? '',
    url: meta?.url ?? '',
    width: meta?.width ?? 0,
    height: meta?.height ?? 0,
  }
}

const boundsSchema = z.object({
  x: z.number(),
  y: z.number(),
  width: z.number(),
  height: z.number(),
})
export type BrowserBounds = z.infer<typeof boundsSchema>

const pickHintSchema = z.object({
  hit: z.boolean(),
  tag: z.string().optional(),
  id: z.string().optional(),
  classes: z.string().optional(),
  bounds: boundsSchema.optional(),
})
export type BrowserPickHint = z.infer<typeof pickHintSchema>

/**
 * `tag#id.class` label in the DevTools grammar, shared by the hover hint
 * chip and the dom outline rows. `classes` is the space-separated class
 * list as the worker reports it.
 */
export function elementLabel(
  tag: string | undefined,
  id: string | undefined | null,
  classes: string | undefined | null,
): string {
  const idPart = id ? `#${id}` : ''
  const classPart = (classes ?? '')
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 3)
    .map((cls) => `.${cls}`)
    .join('')
  return `${tag ?? 'element'}${idPart}${classPart}`
}

const pickedElementSchema = z.object({
  ref: z.string(),
  tag: z.string(),
  attributes: z.record(z.string(), z.string()),
  outer_html: z.string(),
  text: z.string(),
  bounds: boundsSchema,
  url: z.string(),
  console_recent: z.array(z.string()),
})
export type BrowserPickedElement = z.infer<typeof pickedElementSchema>

const pickedEventSchema = z.object({
  session_id: z.string(),
  element: pickedElementSchema,
  timestamp: z.number(),
})
export type BrowserPickedEvent = z.infer<typeof pickedEventSchema>

export function parsePickedEvent(payload: unknown): BrowserPickedEvent | null {
  const parsed = pickedEventSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

const consoleEventSchema = z.object({
  session_id: z.string(),
  entry: consoleEntrySchema,
})
export type BrowserConsoleEvent = z.infer<typeof consoleEventSchema>

export function parseConsoleEvent(
  payload: unknown,
): BrowserConsoleEvent | null {
  const parsed = consoleEventSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

const networkEventSchema = z.object({
  session_id: z.string(),
  entry: networkEntrySchema,
})
export type BrowserNetworkEvent = z.infer<typeof networkEventSchema>

export function parseNetworkEvent(
  payload: unknown,
): BrowserNetworkEvent | null {
  const parsed = networkEventSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

const streamFrameSchema = z.object({
  frame: z.string(),
  width: z.number(),
  height: z.number(),
  frame_seq: z.number(),
  timestamp: z.number(),
})
export type BrowserStreamFrame = z.infer<typeof streamFrameSchema>

/** Pull the frame payload out of a raw `stream::set` frame. The Create/Update
 * shape nests the data at `event.data`; a flat `data` is the fallback. */
export function extractStreamFrame(raw: unknown): BrowserStreamFrame | null {
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

/**
 * Unwrap a transcript output into the worker's plain result. Through the
 * harness, results arrive as `{content:[{type:'text', text:<stringified
 * result>}, ...], details}`; the text block is authoritative, `details` is
 * the fallback cross-check. Direct bus results pass through untouched.
 */
export function decodeBrowserResult(output: unknown): unknown {
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
      return JSON.parse(b.text) as unknown
    } catch {
      break
    }
  }
  if (obj.details != null) return obj.details
  return output
}

/**
 * Session id carried by a `browser::*` function call, wherever the worker
 * put it: the result (`sessions::start`), the harness result envelope
 * around it, or the request payload (everything else).
 */
export function browserSessionIdFromCall(
  input: unknown,
  output: unknown,
): string | null {
  const fromObject = (value: unknown): string | null => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return null
    const obj = value as Record<string, unknown>
    if (typeof obj.session_id === 'string' && obj.session_id.length > 0) {
      return obj.session_id
    }
    return fromObject(obj.details)
  }
  return (
    fromObject(decodeBrowserResult(output)) ??
    fromObject(output) ??
    fromObject(input)
  )
}

/** Human-readable message from anything a bus call can reject with: Error
 * instances, or the engine's plain `{ code, message }` error objects (which
 * String() would render as [object Object]). */
export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'object' && err !== null) {
    const msg = (err as { message?: unknown }).message
    if (typeof msg === 'string' && msg.length > 0) return msg
  }
  return String(err)
}

/** 24-hour clock for a console/network entry timestamp. Shared by the live
 * panels and the chat function-call views. */
export function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString(undefined, { hour12: false })
}

/** Badge variant for a console entry level (`consoleLevelTone` mapped to the
 * Badge component's variant names). */
export function levelBadgeVariant(level: string): 'default' | 'warn' | 'alert' {
  const tone = consoleLevelTone(level)
  return tone === 'ink' ? 'default' : tone
}

export function consoleLevelTone(level: string): 'ink' | 'warn' | 'alert' {
  switch (level) {
    case 'error':
    case 'exception':
      return 'alert'
    case 'warning':
      return 'warn'
    default:
      return 'ink'
  }
}

const PICKED_TEXT_LIMIT = 80
const PICKED_ERROR_LIMIT = 3
const PICKED_ERROR_LINE_LIMIT = 200

/**
 * Selector-ish summary from the picked element's attributes, restricted to
 * id/class/name/type (never the raw outer_html).
 */
export function pickedSelector(element: BrowserPickedElement): string {
  const base = elementLabel(
    element.tag,
    element.attributes.id,
    element.attributes.class,
  )
  const extras = (['name', 'type'] as const)
    .filter((attr) => element.attributes[attr])
    .map((attr) => `[${attr}="${element.attributes[attr]}"]`)
    .join('')
  return `${base}${extras}`
}

/**
 * Compact text block handed to the chat composer for a picked element:
 * one summary line, the url, recent console errors when present, and the
 * ref the agent can use directly. Never includes outer_html.
 */
export function formatPickedElement(evt: BrowserPickedEvent): string {
  const el = evt.element
  const text = el.text.replace(/\s+/g, ' ').trim().slice(0, PICKED_TEXT_LIMIT)
  const summary = text.length > 0 ? ` "${text}"` : ''
  const lines = [
    `picked element ${pickedSelector(el)}${summary} (session ${evt.session_id}, ref ${el.ref})`,
    `url: ${el.url}`,
  ]
  if (el.console_recent.length > 0) {
    lines.push('recent console errors:')
    for (const err of el.console_recent.slice(-PICKED_ERROR_LIMIT)) {
      lines.push(
        `- ${err.replace(/\s+/g, ' ').trim().slice(0, PICKED_ERROR_LINE_LIMIT)}`,
      )
    }
  }
  lines.push(`ref ${el.ref} works with browser::act / browser::styles::read`)
  return lines.join('\n')
}

export async function listBrowserSessions(): Promise<BrowserSessionInfo[]> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(
    BROWSER_SESSIONS_LIST_FUNCTION_ID,
    {},
  )
  const parsed = sessionListSchema.safeParse(res)
  if (!parsed.success) return []
  return (parsed.data.sessions ?? [])
    .map((raw) => {
      const session = sessionInfoSchema.safeParse(raw)
      return session.success ? session.data : null
    })
    .filter((s): s is BrowserSessionInfo => s !== null)
}

export async function startBrowserSession(
  url?: string,
): Promise<BrowserSessionStart | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(
    BROWSER_SESSIONS_START_FUNCTION_ID,
    url ? { url } : {},
  )
  const parsed = sessionStartSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export async function stopBrowserSession(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_SESSIONS_STOP_FUNCTION_ID, {
    session_id: sessionId,
  })
}

export async function navigateBrowser(
  sessionId: string,
  url: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_NAVIGATE_FUNCTION_ID, {
    session_id: sessionId,
    url,
  })
}

export async function takeBrowserScreenshot(
  sessionId: string,
): Promise<BrowserScreenshot | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(BROWSER_SCREENSHOT_FUNCTION_ID, {
    session_id: sessionId,
  })
  return parseScreenshotOutput(res)
}

export interface BrowserClickOptions {
  button?: 'left' | 'right' | 'middle'
  clickCount?: number
}

export async function clickBrowserAt(
  sessionId: string,
  x: number,
  y: number,
  options: BrowserClickOptions = {},
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'click',
    x,
    y,
    ...(options.button ? { button: options.button } : {}),
    ...(options.clickCount ? { click_count: options.clickCount } : {}),
  })
}

export async function scrollBrowserAt(
  sessionId: string,
  x: number,
  y: number,
  deltaY: number,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'scroll',
    x,
    y,
    delta_y: deltaY,
  })
}

export async function typeBrowserText(
  sessionId: string,
  text: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'type',
    text,
  })
}

export async function pressBrowserKey(
  sessionId: string,
  key: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'press',
    key,
  })
}

export async function hintBrowserPick(
  sessionId: string,
  x: number,
  y: number,
): Promise<BrowserPickHint | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(BROWSER_PICK_HINT_FUNCTION_ID, {
    session_id: sessionId,
    x,
    y,
  })
  const parsed = pickHintSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

const frameSchema = z.object({
  frame: z.string().optional(),
  width: z.number(),
  height: z.number(),
  frame_seq: z.number(),
  timestamp: z.number(),
  active: z.boolean(),
})
export type BrowserFrame = z.infer<typeof frameSchema>

export async function startBrowserScreencast(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_SCREENCAST_START_FUNCTION_ID, {
    session_id: sessionId,
  })
}

export async function stopBrowserScreencast(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_SCREENCAST_STOP_FUNCTION_ID, {
    session_id: sessionId,
  })
}

/**
 * Newest pushed screencast frame; a memory read on the worker, cheap to
 * poll fast. `frame` is absent while `sinceFrame` is still the newest seq.
 */
export async function readBrowserFrame(
  sessionId: string,
  sinceFrame?: number,
): Promise<BrowserFrame | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(BROWSER_FRAME_FUNCTION_ID, {
    session_id: sessionId,
    ...(sinceFrame != null ? { since_frame: sinceFrame } : {}),
  })
  const parsed = frameSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export interface BrowserConsoleReadOptions {
  pattern?: string
  level?: string
  sinceSeq?: number
  limit?: number
}

export async function readBrowserConsole(
  sessionId: string,
  options: BrowserConsoleReadOptions = {},
): Promise<BrowserConsoleRead | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(BROWSER_CONSOLE_READ_FUNCTION_ID, {
    session_id: sessionId,
    ...(options.pattern ? { pattern: options.pattern } : {}),
    ...(options.level ? { level: options.level } : {}),
    ...(options.sinceSeq != null ? { since_seq: options.sinceSeq } : {}),
    ...(options.limit != null ? { limit: options.limit } : {}),
  })
  const parsed = consoleReadSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export interface BrowserNetworkReadOptions {
  pattern?: string
  failedOnly?: boolean
  sinceSeq?: number
  limit?: number
}

export async function readBrowserNetwork(
  sessionId: string,
  options: BrowserNetworkReadOptions = {},
): Promise<BrowserNetworkRead | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(BROWSER_NETWORK_READ_FUNCTION_ID, {
    session_id: sessionId,
    ...(options.pattern ? { pattern: options.pattern } : {}),
    ...(options.failedOnly ? { failed_only: true } : {}),
    ...(options.sinceSeq != null ? { since_seq: options.sinceSeq } : {}),
    ...(options.limit != null ? { limit: options.limit } : {}),
  })
  const parsed = networkReadSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export async function startBrowserPick(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_PICK_START_FUNCTION_ID, {
    session_id: sessionId,
  })
}

/** Resolve the element at a clicked point; the worker emits browser::picked.
 * Deterministic (same getNodeForLocation hit-test as the hover hint), unlike
 * a synthesized click through DevTools inspect mode. */
export async function resolveBrowserPick(
  sessionId: string,
  x: number,
  y: number,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_PICK_RESOLVE_FUNCTION_ID, {
    session_id: sessionId,
    x,
    y,
  })
}

export async function stopBrowserPick(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_PICK_STOP_FUNCTION_ID, {
    session_id: sessionId,
  })
}
