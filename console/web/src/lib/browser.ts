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

export const BROWSER_WORKER_NAME = 'browser'

export const BROWSER_SESSIONS_START_FUNCTION_ID = 'browser::sessions::start'
export const BROWSER_SESSIONS_LIST_FUNCTION_ID = 'browser::sessions::list'
export const BROWSER_SESSIONS_STOP_FUNCTION_ID = 'browser::sessions::stop'
export const BROWSER_NAVIGATE_FUNCTION_ID = 'browser::navigate'
export const BROWSER_SCREENSHOT_FUNCTION_ID = 'browser::screenshot'
export const BROWSER_ACT_FUNCTION_ID = 'browser::act'
export const BROWSER_CONSOLE_READ_FUNCTION_ID = 'browser::console::read'
export const BROWSER_NETWORK_READ_FUNCTION_ID = 'browser::network::read'
export const BROWSER_PICK_START_FUNCTION_ID = 'browser::pick::start'
export const BROWSER_PICK_STOP_FUNCTION_ID = 'browser::pick::stop'

export const BROWSER_SESSION_STARTED_TRIGGER = 'browser::session-started'
export const BROWSER_SESSION_STOPPED_TRIGGER = 'browser::session-stopped'
export const BROWSER_NAVIGATED_TRIGGER = 'browser::navigated'
export const BROWSER_CONSOLE_EVENT_TRIGGER = 'browser::console-event'
export const BROWSER_PICKED_TRIGGER = 'browser::picked'

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

const sessionInfoSchema = z.object({
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

const sessionStartSchema = z.object({
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

const consoleReadSchema = z.object({
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

const networkReadSchema = z.object({
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

const screenshotSchema = z.object({
  content: z.array(contentBlockSchema),
  details: z.object({
    session_id: z.string(),
    url: z.string(),
    width: z.number(),
    height: z.number(),
  }),
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
 * Parse a `browser::screenshot` result envelope into a renderable shape.
 * Also used by the chat function-call view on transcript outputs.
 */
export function parseScreenshotOutput(
  payload: unknown,
): BrowserScreenshot | null {
  const parsed = screenshotSchema.safeParse(payload)
  if (!parsed.success) return null
  const image = parsed.data.content.find(
    (block) => block.type === 'image' && block.data,
  )
  return {
    dataUrl: image?.data
      ? `data:${image.mime ?? 'image/jpeg'};base64,${image.data}`
      : null,
    sessionId: parsed.data.details.session_id,
    url: parsed.data.details.url,
    width: parsed.data.details.width,
    height: parsed.data.details.height,
  }
}

const boundsSchema = z.object({
  x: z.number(),
  y: z.number(),
  width: z.number(),
  height: z.number(),
})

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

/**
 * Session id carried by a `browser::*` function call, wherever the worker
 * put it: the result (`sessions::start`), the result's `details` envelope
 * (`screenshot`), or the request payload (everything else).
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
  return fromObject(output) ?? fromObject(input)
}

/** Badge tone per DESIGN.md status semantics for a console entry level. */
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

const SELECTOR_CLASS_LIMIT = 3
const PICKED_TEXT_LIMIT = 160
const PICKED_ERROR_LIMIT = 3
const PICKED_ERROR_LINE_LIMIT = 200

/** `tag#id.class.class` summary from the picked element's attributes. */
export function pickedSelector(element: BrowserPickedElement): string {
  const id = element.attributes.id ? `#${element.attributes.id}` : ''
  const classes = (element.attributes.class ?? '')
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, SELECTOR_CLASS_LIMIT)
    .map((cls) => `.${cls}`)
    .join('')
  return `${element.tag}${id}${classes}`
}

/**
 * Compact text block handed to the chat composer for a picked element:
 * ref, selector summary, trimmed text, url, and recent console errors,
 * shaped so the agent can act on the ref with `browser::act` directly.
 */
export function formatPickedElement(evt: BrowserPickedEvent): string {
  const el = evt.element
  const text = el.text.replace(/\s+/g, ' ').trim().slice(0, PICKED_TEXT_LIMIT)
  const lines = [
    `picked element ${el.ref} on browser session ${evt.session_id}:`,
    `selector: ${pickedSelector(el)}`,
  ]
  if (text.length > 0) lines.push(`text: "${text}"`)
  lines.push(`url: ${el.url}`)
  if (el.console_recent.length > 0) {
    lines.push('recent console errors:')
    for (const err of el.console_recent.slice(-PICKED_ERROR_LIMIT)) {
      lines.push(
        `- ${err.replace(/\s+/g, ' ').trim().slice(0, PICKED_ERROR_LINE_LIMIT)}`,
      )
    }
  }
  lines.push(
    `act on it with browser::act {"session_id":"${evt.session_id}","action":"click","ref":"${el.ref}"}`,
  )
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

export async function clickBrowserAt(
  sessionId: string,
  x: number,
  y: number,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'click',
    x,
    y,
  })
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

export async function stopBrowserPick(sessionId: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(BROWSER_PICK_STOP_FUNCTION_ID, {
    session_id: sessionId,
  })
}
