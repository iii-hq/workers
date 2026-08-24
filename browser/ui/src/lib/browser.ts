import type { ExtensionIii } from '@iii-dev/console-ui'
import { z } from 'zod'

/**
 * Control plane for the `browser` worker's own injected UI: typed wrappers
 * over its session surface (start / list / stop / navigate / screenshot / act
 * / console / network / pick) plus parsers and formatting helpers for the
 * page, the chat function-trigger views, and the pick-to-clipboard flow.
 *
 * Every call goes through the tab's `host.iii` client (an `ExtensionIii`),
 * passed in by the page — there is no module-level singleton in injected UI.
 * Wire source: workers/browser/src/functions/*.rs (verbatim ids + payloads).
 */

export const BROWSER_SESSIONS_START_FUNCTION_ID = 'browser::sessions::start'
export const BROWSER_SESSIONS_LIST_FUNCTION_ID = 'browser::sessions::list'
export const BROWSER_SESSIONS_STOP_FUNCTION_ID = 'browser::sessions::stop'
export const BROWSER_NAVIGATE_FUNCTION_ID = 'browser::navigate'
export const BROWSER_HISTORY_FUNCTION_ID = 'browser::history'
export const BROWSER_DOCTOR_FUNCTION_ID = 'browser::doctor'
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
 * The page subscribes with a `type:'stream'` trigger instead of polling. */
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

const doctorSchema = z.object({
  chromium_version: z.string().nullable().optional(),
})
export type BrowserDoctorInfo = z.infer<typeof doctorSchema>

const historySchema = z.object({
  ok: z.boolean(),
  url: z.string(),
  moved: z.boolean(),
})
export type BrowserHistoryResult = z.infer<typeof historySchema>

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
 * Session id carried by a `browser::*` function trigger, wherever the worker
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
 * panels and the chat function-trigger views. */
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
 * What a dropped pin points at: `selector "text" (ref e12)`. The ref is the
 * handle `browser::act` / `browser::styles::read` take, so an agent reading
 * the note can reach the element directly. Never includes outer_html.
 */
export function pinLabel(evt: BrowserPickedEvent): string {
  const el = evt.element
  // The document roots carry the whole page's text (and its styles); the
  // selector alone says enough about them.
  const text = ROOT_TAGS.has(el.tag)
    ? ''
    : el.text.replace(/\s+/g, ' ').trim().slice(0, PICKED_TEXT_LIMIT)
  const summary = text.length > 0 ? ` "${text}"` : ''
  return `${pickedSelector(el)}${summary} (ref ${el.ref})`
}

const ROOT_TAGS = new Set(['html', 'body', 'head'])

export async function listBrowserSessions(
  iii: ExtensionIii,
): Promise<BrowserSessionInfo[]> {
  const res = await iii.trigger<unknown>(BROWSER_SESSIONS_LIST_FUNCTION_ID, {})
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
  iii: ExtensionIii,
  url?: string,
): Promise<BrowserSessionStart | null> {
  const res = await iii.trigger<unknown>(
    BROWSER_SESSIONS_START_FUNCTION_ID,
    url ? { url } : {},
  )
  const parsed = sessionStartSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export async function stopBrowserSession(
  iii: ExtensionIii,
  sessionId: string,
): Promise<void> {
  await iii.trigger(BROWSER_SESSIONS_STOP_FUNCTION_ID, {
    session_id: sessionId,
  })
}

export async function navigateBrowser(
  iii: ExtensionIii,
  sessionId: string,
  url: string,
): Promise<void> {
  await iii.trigger(BROWSER_NAVIGATE_FUNCTION_ID, {
    session_id: sessionId,
    url,
  })
}

export async function controlBrowserHistory(
  iii: ExtensionIii,
  sessionId: string,
  action: 'back' | 'forward' | 'reload',
): Promise<BrowserHistoryResult | null> {
  const res = await iii.trigger<unknown>(BROWSER_HISTORY_FUNCTION_ID, {
    session_id: sessionId,
    action,
  })
  const parsed = historySchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export async function readBrowserDoctor(
  iii: ExtensionIii,
): Promise<BrowserDoctorInfo | null> {
  const res = await iii.trigger<unknown>(BROWSER_DOCTOR_FUNCTION_ID, {})
  const parsed = doctorSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export async function takeBrowserScreenshot(
  iii: ExtensionIii,
  sessionId: string,
): Promise<BrowserScreenshot | null> {
  const res = await iii.trigger<unknown>(BROWSER_SCREENSHOT_FUNCTION_ID, {
    session_id: sessionId,
  })
  return parseScreenshotOutput(res)
}

export interface BrowserClickOptions {
  button?: 'left' | 'right' | 'middle'
  clickCount?: number
}

export async function clickBrowserAt(
  iii: ExtensionIii,
  sessionId: string,
  x: number,
  y: number,
  options: BrowserClickOptions = {},
): Promise<void> {
  await iii.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'click',
    x,
    y,
    ...(options.button ? { button: options.button } : {}),
    ...(options.clickCount ? { click_count: options.clickCount } : {}),
  })
}

export async function scrollBrowserAt(
  iii: ExtensionIii,
  sessionId: string,
  x: number,
  y: number,
  deltaY: number,
): Promise<void> {
  await iii.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'scroll',
    x,
    y,
    delta_y: deltaY,
  })
}

export async function typeBrowserText(
  iii: ExtensionIii,
  sessionId: string,
  text: string,
): Promise<void> {
  await iii.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'type',
    text,
  })
}

export async function pressBrowserKey(
  iii: ExtensionIii,
  sessionId: string,
  key: string,
): Promise<void> {
  await iii.trigger(BROWSER_ACT_FUNCTION_ID, {
    session_id: sessionId,
    action: 'press',
    key,
  })
}

export async function hintBrowserPick(
  iii: ExtensionIii,
  sessionId: string,
  x: number,
  y: number,
): Promise<BrowserPickHint | null> {
  const res = await iii.trigger<unknown>(BROWSER_PICK_HINT_FUNCTION_ID, {
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

export async function startBrowserScreencast(
  iii: ExtensionIii,
  sessionId: string,
): Promise<void> {
  await iii.trigger(BROWSER_SCREENCAST_START_FUNCTION_ID, {
    session_id: sessionId,
  })
}

export async function stopBrowserScreencast(
  iii: ExtensionIii,
  sessionId: string,
): Promise<void> {
  await iii.trigger(BROWSER_SCREENCAST_STOP_FUNCTION_ID, {
    session_id: sessionId,
  })
}

/**
 * Newest pushed screencast frame; a memory read on the worker, cheap to
 * poll fast. `frame` is absent while `sinceFrame` is still the newest seq.
 */
export async function readBrowserFrame(
  iii: ExtensionIii,
  sessionId: string,
  sinceFrame?: number,
): Promise<BrowserFrame | null> {
  const res = await iii.trigger<unknown>(BROWSER_FRAME_FUNCTION_ID, {
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
  iii: ExtensionIii,
  sessionId: string,
  options: BrowserConsoleReadOptions = {},
): Promise<BrowserConsoleRead | null> {
  const res = await iii.trigger<unknown>(BROWSER_CONSOLE_READ_FUNCTION_ID, {
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
  iii: ExtensionIii,
  sessionId: string,
  options: BrowserNetworkReadOptions = {},
): Promise<BrowserNetworkRead | null> {
  const res = await iii.trigger<unknown>(BROWSER_NETWORK_READ_FUNCTION_ID, {
    session_id: sessionId,
    ...(options.pattern ? { pattern: options.pattern } : {}),
    ...(options.failedOnly ? { failed_only: true } : {}),
    ...(options.sinceSeq != null ? { since_seq: options.sinceSeq } : {}),
    ...(options.limit != null ? { limit: options.limit } : {}),
  })
  const parsed = networkReadSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export async function startBrowserPick(
  iii: ExtensionIii,
  sessionId: string,
): Promise<void> {
  await iii.trigger(BROWSER_PICK_START_FUNCTION_ID, {
    session_id: sessionId,
  })
}

/** Resolve the element at a clicked point; the worker emits browser::picked.
 * Deterministic (same getNodeForLocation hit-test as the hover hint), unlike
 * a synthesized click through DevTools inspect mode. */
export async function resolveBrowserPick(
  iii: ExtensionIii,
  sessionId: string,
  x: number,
  y: number,
): Promise<void> {
  await iii.trigger(BROWSER_PICK_RESOLVE_FUNCTION_ID, {
    session_id: sessionId,
    x,
    y,
  })
}

export async function stopBrowserPick(
  iii: ExtensionIii,
  sessionId: string,
): Promise<void> {
  await iii.trigger(BROWSER_PICK_STOP_FUNCTION_ID, {
    session_id: sessionId,
  })
}

export const BROWSER_FIND_IN_PAGE_FUNCTION_ID = 'browser::find-in-page'
export const BROWSER_ZOOM_FUNCTION_ID = 'browser::zoom'
export const BROWSER_PDF_FUNCTION_ID = 'browser::pdf'

const findSchema = z.object({
  ok: z.boolean(),
  count: z.number(),
  index: z.number(),
  query: z.string(),
})
export type BrowserFindResult = z.infer<typeof findSchema>
export type BrowserFindAction = 'search' | 'next' | 'previous' | 'close'

export async function findInBrowserPage(
  iii: ExtensionIii,
  sessionId: string,
  query: string,
  action: BrowserFindAction = 'search',
  caseSensitive = false,
): Promise<BrowserFindResult> {
  const res = await iii.trigger<unknown>(BROWSER_FIND_IN_PAGE_FUNCTION_ID, {
    session_id: sessionId,
    query,
    action,
    case_sensitive: caseSensitive,
  })
  const parsed = findSchema.safeParse(res)
  if (!parsed.success) throw new Error('find returned an unexpected shape')
  return parsed.data
}

export const ZOOM_LEVELS = [
  50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200,
] as const
export type BrowserZoomAction = 'in' | 'out' | 'reset' | 'set' | 'read'

const zoomSchema = z.object({ ok: z.boolean(), level: z.number() })

export async function zoomBrowserPage(
  iii: ExtensionIii,
  sessionId: string,
  action: BrowserZoomAction,
  level?: number,
): Promise<number> {
  const res = await iii.trigger<unknown>(BROWSER_ZOOM_FUNCTION_ID, {
    session_id: sessionId,
    action,
    ...(level !== undefined ? { level } : {}),
  })
  const parsed = zoomSchema.safeParse(res)
  if (!parsed.success) throw new Error('zoom returned an unexpected shape')
  return parsed.data.level
}

const pdfSchema = z.object({
  ok: z.boolean(),
  data: z.string(),
  size_bytes: z.number(),
  file_name: z.string(),
  url: z.string(),
})
export type BrowserPdf = z.infer<typeof pdfSchema>

export async function printBrowserPageToPdf(
  iii: ExtensionIii,
  sessionId: string,
): Promise<BrowserPdf> {
  const res = await iii.trigger<unknown>(BROWSER_PDF_FUNCTION_ID, {
    session_id: sessionId,
  })
  const parsed = pdfSchema.safeParse(res)
  if (!parsed.success) throw new Error('pdf returned an unexpected shape')
  return parsed.data
}

/** A `File` from base64 bytes, for attachments and downloads. */
export function fileFromBase64(
  base64: string,
  name: string,
  type: string,
): File {
  const binary = atob(base64)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i)
  return new File([bytes], name, { type })
}

/** Hands a file to the browser's own download flow. */
export function downloadFile(file: File): void {
  const url = URL.createObjectURL(file)
  const link = document.createElement('a')
  link.href = url
  link.download = file.name
  link.click()
  window.setTimeout(() => URL.revokeObjectURL(url), 1000)
}

/** `screenshot-<host>-<stamp>.jpg`, safe for a file system. */
export function screenshotFileName(url: string, at = new Date()): string {
  const host = url
    .replace(/^https?:\/\//, '')
    .split(/[/?#]/)[0]
    .replace(/[^a-z0-9.-]+/gi, '-')
    .slice(0, 48)
  const stamp = at.toISOString().replace(/[:.]/g, '-')
  return `screenshot-${host || 'page'}-${stamp}.jpg`
}

export const BROWSER_HISTORY_LIST_FUNCTION_ID = 'browser::history::list'
export const BROWSER_CLEAR_DATA_FUNCTION_ID = 'browser::clear-data'
export const BROWSER_DOWNLOADS_LIST_FUNCTION_ID = 'browser::downloads::list'
export const BROWSER_DOWNLOAD_FUNCTION_ID = 'browser::download'
export const BROWSER_DOWNLOAD_REMOVE_FUNCTION_ID = 'browser::download::remove'
export const BROWSER_DOWNLOAD_CHANGED_TRIGGER = 'browser::download-changed'

const historyVisitSchema = z.object({
  url: z.string(),
  title: z.string(),
  timestamp: z.number(),
})
export type BrowserHistoryVisit = z.infer<typeof historyVisitSchema>
const historyListSchema = z.object({ visits: z.array(historyVisitSchema) })

export async function listBrowserHistory(
  iii: ExtensionIii,
  sessionId: string,
  query?: string,
): Promise<BrowserHistoryVisit[]> {
  const res = await iii.trigger<unknown>(BROWSER_HISTORY_LIST_FUNCTION_ID, {
    session_id: sessionId,
    ...(query ? { query } : {}),
  })
  return historyListSchema.safeParse(res).success
    ? historyListSchema.parse(res).visits
    : []
}

const clearDataSchema = z.object({
  ok: z.boolean(),
  cleared: z.array(z.string()),
})

export async function clearBrowserData(
  iii: ExtensionIii,
  sessionId: string,
): Promise<string[]> {
  const res = await iii.trigger<unknown>(BROWSER_CLEAR_DATA_FUNCTION_ID, {
    session_id: sessionId,
  })
  return clearDataSchema.safeParse(res).success
    ? clearDataSchema.parse(res).cleared
    : []
}

const downloadRecordSchema = z.object({
  guid: z.string(),
  file_name: z.string(),
  url: z.string(),
  state: z.enum(['in_progress', 'completed', 'canceled']),
  received_bytes: z.number(),
  total_bytes: z.number(),
  started_ms: z.number(),
})
export type BrowserDownload = z.infer<typeof downloadRecordSchema>
const downloadsListSchema = z.object({
  downloads: z.array(downloadRecordSchema),
})

export async function listBrowserDownloads(
  iii: ExtensionIii,
  sessionId: string,
): Promise<BrowserDownload[]> {
  const res = await iii.trigger<unknown>(BROWSER_DOWNLOADS_LIST_FUNCTION_ID, {
    session_id: sessionId,
  })
  return downloadsListSchema.safeParse(res).success
    ? downloadsListSchema.parse(res).downloads
    : []
}

const downloadSchema = z.object({
  ok: z.boolean(),
  data: z.string(),
  file_name: z.string(),
  size_bytes: z.number(),
})

export async function readBrowserDownload(
  iii: ExtensionIii,
  sessionId: string,
  guid: string,
): Promise<{ file: File } | null> {
  const res = await iii.trigger<unknown>(BROWSER_DOWNLOAD_FUNCTION_ID, {
    session_id: sessionId,
    guid,
  })
  const parsed = downloadSchema.safeParse(res)
  if (!parsed.success) return null
  try {
    return {
      file: fileFromBase64(
        parsed.data.data,
        parsed.data.file_name,
        'application/octet-stream',
      ),
    }
  } catch {
    return null
  }
}

export async function removeBrowserDownload(
  iii: ExtensionIii,
  sessionId: string,
  guid: string,
): Promise<void> {
  await iii.trigger(BROWSER_DOWNLOAD_REMOVE_FUNCTION_ID, {
    session_id: sessionId,
    guid,
  })
}

export const BROWSER_RESIZE_FUNCTION_ID = 'browser::resize'

const resizeSchema = z.object({
  ok: z.boolean(),
  width: z.number(),
  height: z.number(),
})

/** Set the session's live viewport to `width` x `height` CSS pixels. */
export async function resizeBrowser(
  iii: ExtensionIii,
  sessionId: string,
  width: number,
  height: number,
  options: { deviceScaleFactor?: number; mobile?: boolean; fit?: boolean } = {},
): Promise<{ width: number; height: number } | null> {
  const res = await iii.trigger<unknown>(BROWSER_RESIZE_FUNCTION_ID, {
    session_id: sessionId,
    width: Math.round(width),
    height: Math.round(height),
    ...(options.deviceScaleFactor
      ? { device_scale_factor: options.deviceScaleFactor }
      : {}),
    ...(options.mobile ? { mobile: options.mobile } : {}),
    ...(options.fit ? { fit: true } : {}),
  })
  const parsed = resizeSchema.safeParse(res)
  return parsed.success
    ? { width: parsed.data.width, height: parsed.data.height }
    : null
}
