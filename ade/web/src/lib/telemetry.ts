/**
 * Product telemetry for the console, sent to PostHog from the browser.
 *
 * The engine decides whether it may run: `GET /runtime` carries a
 * `telemetry` object with the engine's own opt-out result and its device id
 * as the PostHog person (see `ade/src/telemetry.rs`). With telemetry off,
 * posthog-js is never started and nothing here sends.
 *
 * Events are kept few on purpose (setup wizard steps are reported through
 * the engine instead, see `lib/onboarding/telemetry.ts`):
 * - `$pageview`: once per workspace tab change, naming the screen kinds shown.
 * - `ade_usage`: a batched report of the last 25 palette commands and, per
 *   screen kind, how often it opened and how long it was visible. A visit
 *   reports at 15, 30, 45 and 60 minutes, then hourly, and once more when the
 *   page closes. Right after setup completes on a device, the schedule is 30s,
 *   1m, 5m, 30m and 60m instead, then hourly. Each report holds only what
 *   happened since the one before; an empty one is not sent.
 *
 * No text a user wrote is sent: command ids that can hold paths or titles are
 * reduced to their kind, chat screens to `chat`, URLs lose their hash and
 * query string, and session replay masks every text node and input and blocks
 * media. An exception keeps only its error type and stack frames: its message
 * can hold what a user typed.
 */
import posthog, { type CaptureOptions, type CaptureResult } from 'posthog-js'
import { isChatScreen, type TabScreen } from '@/lib/workspace-tabs'

const MIN = 60_000
const HOUR = 60 * MIN

/** Report offsets from the start of a schedule; after the last, hourly. */
export const VISIT_OFFSETS = [15, 30, 45, 60].map((m) => m * MIN)
export const POST_SETUP_OFFSETS = [0.5, 1, 5, 30, 60].map((m) => m * MIN)

const MAX_COMMANDS = 25
const POST_SETUP_KEY = 'iii-telemetry-post-setup'

/** Palette kinds whose ids are fixed by the console, never by user data. */
const SAFE_COMMAND_KINDS = new Set(['action', 'command', 'page'])

interface RuntimeTelemetry {
  enabled: boolean
  distinct_id?: string | null
  api_key?: string
  host?: string
}

type ScreenUsage = Record<string, { opens: number; ms: number }>

let enabled = false
/** Setup finished before the settings arrived. */
let setupPending = false
let commands: string[] = []
let screens: ScreenUsage = {}
/** Screen kinds on the active tab, and since when their time is counted. */
let current: string[] = []
let since: number | null = null
let visible = true

let offsets = VISIT_OFFSETS
let scheduleStart = 0
let index = 0
let timer: ReturnType<typeof setTimeout> | undefined

export function offsetAt(list: number[], i: number): number {
  const last = list.length - 1
  return i <= last ? list[i] : list[last] + (i - last) * HOUR
}

export function commandLabel(entry: { id: string; kind: string }): string {
  return SAFE_COMMAND_KINDS.has(entry.kind) ? entry.id : entry.kind
}

export function screenKind(screen: TabScreen): string {
  return isChatScreen(screen) ? 'chat' : screen
}

export async function initTelemetry(): Promise<void> {
  const settings = await fetchSettings()
  if (!settings?.enabled || !settings.api_key) return
  posthog.init(settings.api_key, {
    api_host: settings.host,
    persistence: 'localStorage',
    bootstrap: settings.distinct_id
      ? { distinctID: settings.distinct_id, isIdentifiedID: true }
      : undefined,
    // Hash routes can carry worker context (`#/worker/...?context=`), so no
    // URL field keeps its hash or query and no referrer is kept; `scrubEvent`
    // also cuts both from the URLs these options do not reach.
    disable_capture_url_hashes: true,
    mask_personal_data_properties: true,
    save_referrer: false,
    save_campaign_params: false,
    autocapture: false,
    rageclick: false,
    capture_dead_clicks: false,
    disable_surveys: true,
    // Sent per workspace tab by `trackScreens` instead.
    capture_pageview: false,
    enable_heatmaps: true,
    capture_performance: { web_vitals: true },
    capture_exceptions: true,
    before_send: scrubEvent,
    mask_all_text: true,
    mask_all_element_attributes: true,
    session_recording: {
      maskAllInputs: true,
      maskTextSelector: '*',
      blockSelector: 'img, video, canvas, iframe, svg image',
    },
  })
  enabled = true
  if (current.length > 0) capture('$pageview', { screens: current })
  window.addEventListener('pagehide', () => flush('sendBeacon'))
  document.addEventListener('visibilitychange', () =>
    setVisible(document.visibilityState === 'visible'),
  )
  setVisible(document.visibilityState === 'visible')
  startSchedule(VISIT_OFFSETS)
  if (setupPending) setupCompleted()
}

/** Referrer properties: `$referrer`, `$referring_domain` and their
 * `$initial_` and `$session_entry_` forms. `save_referrer: false` does not
 * cover the initial and session entry ones. */
const REFERRER_PROPERTY = /referr/

/** The `before_send` hook: no referrer is sent, no URL keeps its query string
 * or hash, since any parameter can hold a path or a token
 * (`mask_personal_data_properties` only masks known ad identifiers), and
 * exceptions keep only their type. URLs are found at any depth: web vitals
 * nest `$current_url` and `navigationURL`. */
export function scrubEvent(event: CaptureResult | null): CaptureResult | null {
  if (!event) return event
  for (const props of [event.properties, event.$set, event.$set_once])
    for (const key of Object.keys(props ?? {}))
      if (REFERRER_PROPERTY.test(key)) delete props?.[key]
  event.properties = stripUrls(event.properties)
  if (event.$set) event.$set = stripUrls(event.$set)
  if (event.$set_once) event.$set_once = stripUrls(event.$set_once)
  return scrubException(event)
}

function stripUrl(value: string): string {
  return /^https?:\/\//.test(value) ? value.split(/[?#]/)[0] : value
}

/** Strip URLs in values and in keys: `$heatmap_data` is keyed by page URL.
 * Keys that become the same URL have their lists joined. */
// biome-ignore lint/suspicious/noExplicitAny: walks arbitrary event JSON
function stripUrls(value: any): any {
  if (typeof value === 'string') return stripUrl(value)
  if (Array.isArray(value)) return value.map(stripUrls)
  if (!value || typeof value !== 'object') return value
  // biome-ignore lint/suspicious/noExplicitAny: walks arbitrary event JSON
  const out: Record<string, any> = {}
  for (const [k, v] of Object.entries(value)) {
    const key = stripUrl(k)
    const next = stripUrls(v)
    out[key] =
      Array.isArray(out[key]) && Array.isArray(next)
        ? [...out[key], ...next]
        : next
  }
  return out
}

/** Replace each exception message with its error type, and drop the
 * message copies and the steps leading up to it. */
export function scrubException(
  event: CaptureResult | null,
): CaptureResult | null {
  if (event?.event !== '$exception') return event
  const p = event.properties
  const list: { type?: string }[] = Array.isArray(p.$exception_list)
    ? p.$exception_list
    : []
  p.$exception_list = list.map((e) => ({ ...e, value: e.type }))
  p.$exception_message = p.$exception_type ?? list[0]?.type
  delete p.$exception_values
  delete p.$exception_steps
  return event
}

/** The screens shown on the newly active tab. */
export function trackScreens(tabScreens: (TabScreen | null)[]): void {
  const kinds = [
    ...new Set(tabScreens.filter((s): s is TabScreen => !!s).map(screenKind)),
  ]
  settle()
  for (const kind of kinds) {
    screens[kind] ??= { opens: 0, ms: 0 }
    screens[kind].opens += 1
  }
  current = kinds
  capture('$pageview', { screens: kinds })
}

export function trackCommand(entry: { id: string; kind: string }): void {
  commands.push(commandLabel(entry))
  if (commands.length > MAX_COMMANDS) commands = commands.slice(-MAX_COMMANDS)
}

/** Setup finished: run the post-setup schedule, once per device. */
export function setupCompleted(): void {
  setupPending = !enabled
  if (!enabled || read(POST_SETUP_KEY)) return
  write(POST_SETUP_KEY, '1')
  startSchedule(POST_SETUP_OFFSETS)
}

export function setVisible(next: boolean): void {
  settle()
  visible = next
}

/** Send what happened since the last report, if anything. */
export function flush(transport?: CaptureOptions['transport']): void {
  settle()
  if (!enabled || (commands.length === 0 && Object.keys(screens).length === 0))
    return
  capture(
    'ade_usage',
    { commands, screens },
    transport ? { transport, send_instantly: true } : undefined,
  )
  commands = []
  screens = {}
}

/** Add the time the current screens were visible since the last settle. */
function settle(): void {
  const now = Date.now()
  if (since !== null && visible && now > since) {
    for (const kind of current) {
      screens[kind] ??= { opens: 0, ms: 0 }
      screens[kind].ms += now - since
    }
  }
  since = now
}

function startSchedule(next: number[]): void {
  clearTimeout(timer)
  offsets = next
  scheduleStart = Date.now()
  index = 0
  schedule()
}

function schedule(): void {
  timer = setTimeout(
    () => {
      flush()
      index += 1
      schedule()
    },
    scheduleStart + offsetAt(offsets, index) - Date.now(),
  )
}

function capture(
  name: string,
  properties: Record<string, unknown>,
  options?: CaptureOptions,
): void {
  if (enabled) posthog.capture(name, properties, options)
}

async function fetchSettings(): Promise<RuntimeTelemetry | null> {
  try {
    const response = await fetch(new URL('./runtime', window.location.href), {
      cache: 'no-store',
    })
    if (!response.ok) return null
    const value = (await response.json()) as { telemetry?: RuntimeTelemetry }
    return value.telemetry ?? null
  } catch {
    return null
  }
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value)
  } catch {
    // Private mode or blocked storage: the post-setup schedule may run again.
  }
}
