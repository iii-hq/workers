/**
 * The capability object (design §4.2): everything a script can do outside
 * its own tree goes through `iii`, as a request the host checks. Calls and
 * trigger bindings are limited to the app's own namespace here too (D8), so
 * a mistake fails at once with a clear error instead of a round trip.
 */

import { batch, createSignal, type Setter } from 'solid-js'
import { declaredPages, type PageRef } from './app'
import { ENUMS, IN, LIMITS, METHOD } from './generated/catalog'
import { IiiError, notify, onEntry, reportError, request, run, subscribe } from './runtime/wire'
import type { JsonObject, JsonValue, NodeRef, Unsubscribe } from './types'

export { IiiError }

export interface Env {
  /** The window's 720 dp cut (the app's navigation rules). */
  windowClass: 'compact' | 'wide'
  /** The hinge; `flat` when there is none. */
  posture: 'flat' | 'tabletop' | 'book'
  platform: 'android' | 'ios' | 'ipados' | 'macos'
  /** Some page of the app is on the visible tab and the app is in the foreground. */
  visible: boolean
  /** Always `dark` for now. */
  theme: 'dark' | 'light'
  /** BCP 47, e.g. `pt-BR`. */
  locale: string
  /** The engine session is connected. */
  online: boolean
}

export interface CallOptions {
  /** Default 30 000, at most 120 000. */
  timeoutMs?: number
}

export interface MenuAction {
  label: string
  /** A Lucide name. */
  icon?: string
  destructive?: boolean
  /** Text on the right ("110%"). */
  hint?: string
  /** A check mark (On/Off); on the Mac, the NSMenuItem state. */
  checked?: boolean
}

export type KeyboardType = 'text' | 'url' | 'search' | 'email' | 'number' | 'password'

export interface Ui {
  toast(text: string, kind?: 'info' | 'success' | 'error'): void
  /** A list of actions: an action sheet on the phone, a context menu (NSMenu) on the Mac. Resolves with the index, or -1. */
  actionMenu(
    anchor: NodeRef | null,
    menu: { title?: string; subtitle?: string; actions: MenuAction[] },
  ): Promise<number>
  confirm(opts: { title: string; message?: string; confirm: string; destructive?: boolean }): Promise<boolean>
  prompt(opts: {
    title: string
    message?: string
    initial?: string
    placeholder?: string
    confirm: string
    keyboard?: KeyboardType
    mono?: boolean
  }): Promise<string | null>
  /** Asks for (or drops) the focus of a TextField; only on the top card of the visible tab. */
  focus(node: NodeRef, focused?: boolean): void
}

export interface Nav {
  /** Pushes a page declared in defineApp; resolves with the result of its pop, or null (Back). */
  push<R extends JsonValue = JsonValue>(page: string, params?: JsonObject): Promise<R | null>
  /** Pops `page`'s card (only when it is the top one) with a result for whoever pushed it. */
  pop(page: PageRef, result?: JsonValue): void
  /** Replaces `page`'s card with another page. */
  replace(page: PageRef, name: string, params?: JsonObject): void
  popToRoot(): void
}

export interface ShareText {
  text: string
  title?: string
}
export interface ShareFile {
  /** base64 */
  data: string
  mime: string
  filename: string
}

/** Key/value on the device, per (environment, worker). Key up to 128 characters; value JSON up to 64 KiB; 256 KiB in all. */
export interface KvStorage {
  get<T extends JsonValue = JsonValue>(key: string): Promise<T | null>
  set(key: string, value: JsonValue): Promise<void>
  remove(key: string): Promise<void>
  keys(): Promise<string[]>
}

export interface Iii {
  /** `worker:<ns>` and `<ns>`. */
  readonly app: { readonly key: string; readonly ns: string }
  /** Calls a function of the app's own namespace (`<ns>::…`); any other fails with `forbidden`. */
  call<T = JsonValue>(fn: string, payload?: JsonValue, opts?: CallOptions): Promise<T>
  /**
   * Binds a trigger type of the app's own namespace (`<ns>::…` or `<ns>:…`) to `handler` (the host uses EngineSession.watch).
   * Bind before reading: whoever needs initial state reads after the `ready` Promise resolves.
   */
  on<P = JsonValue>(
    triggerType: string,
    config: JsonObject,
    handler: (payload: P) => void,
  ): Unsubscribe & { ready: Promise<void> }
  readonly nav: Nav
  /** The environment now (read-only; it changes only when the host sends `env`). Reactive in Solid scopes. */
  readonly env: Readonly<Env>
  onEnv(handler: (env: Readonly<Env>, changed: ReadonlyArray<keyof Env>) => void): Unsubscribe
  readonly ui: Ui
  readonly clipboard: { writeText(text: string): Promise<void> }
  /** Opens in the system browser after asking the person (always, with the URL in view). Only http, https and mailto. */
  openUrl(url: string): Promise<boolean>
  share(item: ShareText | ShareFile): Promise<void>
  /** Saves into Downloads/iii (Android) and resolves with the path shown to the person. */
  saveFile(file: ShareFile): Promise<{ path: string }>
  readonly storage: KvStorage
  /** Not fatal: the log (and a toast in a debug build of the app). Fatal: the failure screen with Restart. */
  reportError(error: unknown, opts?: { fatal?: boolean }): void
}

// ── App identity and the namespace rule ─────────────────────────────────────

declare const __IIIUI_NS__: string | undefined

/** The namespace `buildNativeUi` compiled in; the host's `init` is the authority. */
const builtNs = typeof __IIIUI_NS__ === 'string' ? __IIIUI_NS__ : ''
const app = { key: builtNs ? `worker:${builtNs}` : '', ns: builtNs }
let hostInfo: JsonObject | null = null

/** The `init` info of the host (null before it arrives). */
export function host(): Readonly<JsonObject> | null {
  return hostInfo
}

/**
 * Functions are `<ns>::…`; trigger types are `<ns>::…` or `<ns>:…` (a
 * worker may name its triggers `kanban:ticket-changed`). Never `iii::` or
 * `engine::`. The host's policy checks the same.
 */
function checkOwn(kind: 'call' | 'bind', id: string): void {
  if (typeof id !== 'string' || !id) throw new IiiError('invalid', `iii.${kind === 'call' ? 'call' : 'on'} needs an id`)
  const ns = app.ns
  if (!ns) return // before init, without a built-in namespace: the host decides
  const prefix = kind === 'call' ? `${ns}::` : `${ns}:`
  if (id.startsWith(prefix) && !id.startsWith('iii::') && !id.startsWith('engine::')) return
  throw new IiiError(
    'forbidden',
    kind === 'call'
      ? `${ns} may only call ${ns}::… functions (not ${id})`
      : `${ns} may only bind ${ns}:… or ${ns}::… triggers (not ${id})`,
  )
}

// ── Env ─────────────────────────────────────────────────────────────────────

const ENV_DEFAULTS: Env = {
  windowClass: 'compact',
  posture: 'flat',
  platform: 'android',
  visible: true,
  theme: 'dark',
  locale: 'en-US',
  online: true,
}
type EnvKey = keyof Env
const envSetters = {} as Record<EnvKey, Setter<unknown>>
const env = {} as Env
for (const key of Object.keys(ENV_DEFAULTS) as EnvKey[]) {
  const [get, set] = createSignal<unknown>(ENV_DEFAULTS[key])
  envSetters[key] = set
  Object.defineProperty(env, key, { get, enumerable: true })
}
Object.freeze(env)
const envHandlers = new Set<(env: Readonly<Env>, changed: ReadonlyArray<EnvKey>) => void>()

function applyEnv(changes: unknown): void {
  if (!changes || typeof changes !== 'object') return
  const changed: EnvKey[] = []
  batch(() => {
    for (const [key, value] of Object.entries(changes as Record<string, unknown>)) {
      if (!(key in ENV_DEFAULTS) || value === undefined || env[key as EnvKey] === value) continue
      envSetters[key as EnvKey](() => value)
      changed.push(key as EnvKey)
    }
  })
  if (!changed.length) return
  for (const handler of [...envHandlers]) run(() => handler(env, changed))
}

onEntry(IN.init, ([, info]) => {
  hostInfo = (info ?? {}) as JsonObject
  const given = hostInfo.app as { key?: unknown; ns?: unknown } | undefined
  if (given && typeof given.ns === 'string') {
    if (builtNs && given.ns !== builtNs) {
      console.warn(`native-ui: this bundle was built for "${builtNs}" but runs as "${given.ns}"`)
    }
    app.ns = given.ns
    app.key = typeof given.key === 'string' ? given.key : `worker:${given.ns}`
  }
  applyEnv(hostInfo.env)
})

onEntry(IN.env, ([, changes]) => applyEnv(changes))

// ── Helpers ─────────────────────────────────────────────────────────────────

/** `{key: value}` without the keys whose value is undefined, in the given order. */
function args(fields: Record<string, unknown>): JsonObject {
  const out: JsonObject = {}
  for (const [k, v] of Object.entries(fields)) if (v !== undefined) out[k] = v as JsonValue
  return out
}

function enumIndex(name: 'toastKind' | 'keyboardType', value: string | undefined): number | undefined {
  if (value === undefined) return undefined
  const i = (ENUMS[name] as readonly string[]).indexOf(value)
  return i < 0 ? undefined : i
}

const field =
  <T>(key: string, fallback: T) =>
  (v: JsonValue) => {
    const x = v && typeof v === 'object' && !Array.isArray(v) ? (v as JsonObject)[key] : undefined
    return (x === undefined ? fallback : x) as T
  }

// ── The object ──────────────────────────────────────────────────────────────

const nav: Nav = {
  push<R extends JsonValue = JsonValue>(page: string, params?: JsonObject): Promise<R | null> {
    if (!declaredPages.has(page)) {
      return Promise.reject(new IiiError('invalid', `nav.push: "${page}" is not one of the pages of defineApp`))
    }
    return request(METHOD.navPush, args({ page, params })).then(field<R | null>('result', null))
  },
  pop(page, result) {
    notify(METHOD.navPop, args({ page: page.inst, result }))
  },
  replace(page, name, params) {
    if (!declaredPages.has(name))
      throw new IiiError('invalid', `nav.replace: "${name}" is not one of the pages of defineApp`)
    notify(METHOD.navReplace, args({ page: page.inst, name, params }))
  },
  popToRoot() {
    notify(METHOD.navPopToRoot, {})
  },
}

const ui: Ui = {
  toast(text, kind) {
    notify(METHOD.toast, args({ text: String(text), kind: enumIndex('toastKind', kind) }))
  },
  actionMenu(anchor, menu) {
    const actions = (menu?.actions ?? []).map((a) =>
      args({ label: String(a.label), icon: a.icon, destructive: a.destructive, hint: a.hint, checked: a.checked }),
    )
    return request(
      METHOD.actionMenu,
      args({ anchor: anchor ? anchor.id : undefined, title: menu?.title, subtitle: menu?.subtitle, actions }),
    ).then((v) => {
      const index = field<number>('index', -1)(v)
      return typeof index === 'number' ? index : -1
    })
  },
  confirm(opts) {
    return request(
      METHOD.confirm,
      args({ title: opts.title, message: opts.message, confirm: opts.confirm, destructive: opts.destructive }),
    ).then((v) => field<boolean>('ok', false)(v) === true)
  },
  prompt(opts) {
    return request(
      METHOD.prompt,
      args({
        title: opts.title,
        message: opts.message,
        initial: opts.initial,
        placeholder: opts.placeholder,
        confirm: opts.confirm,
        keyboard: enumIndex('keyboardType', opts.keyboard),
        mono: opts.mono,
      }),
    ).then((v) => {
      const text = field<string | null>('text', null)(v)
      return typeof text === 'string' ? text : null
    })
  },
  focus(node, focused = true) {
    notify(METHOD.focus, { node: node.id, focused })
  },
}

function checkKey(key: string): IiiError | null {
  if (typeof key !== 'string' || !key) return new IiiError('invalid', 'storage keys are non-empty strings')
  if (key.length > LIMITS.maxStorageKeyChars) {
    return new IiiError('invalid', `storage keys have at most ${LIMITS.maxStorageKeyChars} characters`)
  }
  return null
}

const storage: KvStorage = {
  get<T extends JsonValue = JsonValue>(key: string): Promise<T | null> {
    const bad = checkKey(key)
    if (bad) return Promise.reject(bad)
    return request(METHOD.storageGet, { key }).then(field<T | null>('value', null))
  },
  set(key, value) {
    const bad = checkKey(key)
    if (bad) return Promise.reject(bad)
    return request(METHOD.storageSet, { key, value: value ?? null }).then(() => undefined)
  },
  remove(key) {
    const bad = checkKey(key)
    if (bad) return Promise.reject(bad)
    return request(METHOD.storageRemove, { key }).then(() => undefined)
  },
  keys() {
    return request(METHOD.storageKeys, {}).then((v) => {
      const keys = field<unknown>('keys', [])(v)
      return Array.isArray(keys) ? keys.map(String) : []
    })
  },
}

const URL_SCHEMES = /^(https?|mailto):/i

export const iii: Iii = {
  app,
  call<T = JsonValue>(fn: string, payload?: JsonValue, opts?: CallOptions): Promise<T> {
    try {
      checkOwn('call', fn)
    } catch (err) {
      return Promise.reject(err)
    }
    let timeoutMs: number | undefined
    if (opts?.timeoutMs !== undefined) {
      timeoutMs = Math.min(
        Math.max(1, Math.trunc(opts.timeoutMs) || LIMITS.callTimeoutMsDefault),
        LIMITS.callTimeoutMsMax,
      )
    }
    return request(METHOD.call, args({ fn, payload, timeoutMs })) as Promise<T>
  },
  on<P = JsonValue>(triggerType: string, config: JsonObject, handler: (payload: P) => void) {
    checkOwn('bind', triggerType)
    const { ready, off } = subscribe(triggerType, (config ?? {}) as JsonObject, handler as (p: JsonValue) => void)
    return Object.assign(off, { ready })
  },
  nav,
  env,
  onEnv(handler) {
    envHandlers.add(handler)
    return () => {
      envHandlers.delete(handler)
    }
  },
  ui,
  clipboard: {
    writeText(text) {
      return request(METHOD.clipboardWrite, { text: String(text) }).then(() => undefined)
    },
  },
  openUrl(url) {
    if (!URL_SCHEMES.test(String(url))) {
      return Promise.reject(new IiiError('forbidden', `openUrl only opens http, https and mailto URLs (not ${url})`))
    }
    return request(METHOD.openUrl, { url: String(url) }).then((v) => field<boolean>('opened', false)(v) === true)
  },
  share(item) {
    const fields =
      'data' in item
        ? { data: item.data, mime: item.mime, filename: item.filename }
        : { text: item.text, title: (item as ShareText).title }
    return request(METHOD.share, args(fields)).then(() => undefined)
  },
  saveFile(file) {
    return request(METHOD.saveFile, { data: file.data, mime: file.mime, filename: file.filename }).then((v) => ({
      path: String(field<string>('path', '')(v)),
    }))
  },
  storage,
  reportError(error, opts) {
    reportError(error, opts)
  },
}
