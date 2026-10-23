/** The test harness of @iii-dev/native-ui (design §10.5). */

type Json = null | boolean | number | string | Json[] | { [key: string]: Json }

export interface Env {
  windowClass: 'compact' | 'wide'
  posture: 'flat' | 'tabletop' | 'book'
  platform: 'android' | 'ios' | 'ipados' | 'macos'
  visible: boolean
  theme: 'dark' | 'light'
  locale: string
  online: boolean
}

export declare const DEFAULT_ENV: Readonly<Env>

/** A failure the fake host answers a request with: `calls` handlers throw it. */
export declare class HostError extends Error {
  readonly code: string
  constructor(code: string, message: string)
}

/** The VM stopped for good: `watchdog` (a step passed its deadline), `memory`, `reported` (fatal reportError), `load`, `protocol`, `catalog` (the bundle needs a newer catalogue than the host's). */
export declare class VmFailure extends Error {
  readonly kind: string
}

/** A node in named form: type and prop names, enums and the events mask readable, slots by name. */
export interface NodeView {
  type: string
  id: number
  props?: Record<string, unknown>
  slots?: Record<string, NodeView[]>
  /** A List's items as the host holds them. */
  items?: Json[]
}

/** A testTag, a node id, or `{ type, ...props }` matched against the readable props. */
export type Target = string | number | ({ type?: string } & Record<string, unknown>)

export interface ReadyInfo {
  sdk: string
  catalog: number
  features: string[]
  pages: { name: string; modal: boolean; fullScreen: boolean }[]
}

export interface Card {
  inst: number
  name: string
  params: Json
  /** The navPush that made the card, or null. */
  reqId: number | null
}

export interface TestHostOptions {
  /** The bundle source (what buildNativeUi wrote). */
  bundle: string
  /** The app's namespace (default `demo`). */
  ns?: string
  /**
   * The catalogue this host renders (`init.catalog`; default the SDK's,
   * `catalog.json` › catalogVersion). A bundle whose `ready` needs a newer
   * one fails to load (`VmFailure` kind `catalog`), as the app shows
   * "Needs a newer iii app".
   */
  catalog?: number
  appKey?: string
  /** `init.host` (default `android 0.1.0`) and `init.engine` (default `quickjs-emscripten 0.31.0`). */
  host?: string
  engine?: string
  env?: Partial<Env>
  /**
   * Answers of `call` by function id: a value, or a function of the payload
   * (it may return a Promise or throw a HostError). Like the device host, a
   * call that has not answered when the fake clock passes its `timeoutMs`
   * (default `limits.callTimeoutMsDefault`, 30 000) fails with
   * `{code: "timeout", message: "The engine didn't answer in time"}`; a
   * later answer is dropped. `tick()` moves the clock.
   */
  calls?: Record<string, Json | ((payload: Json, host: TestHost) => unknown)>
  /** Answers of the dialogs; without one: actionMenu -1, confirm false, prompt null, openUrl true. */
  ui?: {
    actionMenu?: (args: Json, host: TestHost) => number | Promise<number>
    confirm?: (args: Json, host: TestHost) => boolean | Promise<boolean>
    prompt?: (args: Json, host: TestHost) => string | null | Promise<string | null>
    openUrl?: (url: string, host: TestHost) => boolean | Promise<boolean>
  }
  storage?: Record<string, Json>
  /** The fake clock's start in ms (default 0). */
  now?: number
  /**
   * Exposes `globalThis.__iiiui_request(method, args)` in the VM: a request
   * without the SDK's own checks, to show what the host answers to a call
   * the SDK would refuse (test-only).
   */
  rawRequests?: boolean
  /** Seeds the SDK's id counters, to reproduce exact ids. */
  ids?: { node?: number; req?: number; sub?: number }
  /** Throw on protocol violations and uncaught VM errors (default true). */
  strict?: boolean
  /** Send `init` with the first action (default true). */
  autoInit?: boolean
  memoryLimit?: number
  stackSize?: number
  stepBudgetMs?: number
  loadBudgetMs?: number
  maxMessageBytes?: number
  /** Every console line of the VM. */
  log?: (level: 'debug' | 'info' | 'warn' | 'error', message: string) => void
}

export interface TestHost {
  readonly ready: ReadyInfo
  /** VM → host messages, json-v0. */
  readonly messages: string[]
  /** Every VM → host entry, parsed. */
  readonly ops: unknown[][]
  /** Host → VM messages, json-v0. */
  readonly toVm: string[]
  readonly transcript: { dir: 'toVm' | 'fromVm'; message: string }[]
  readonly requests: { reqId: number; method: string; args: Record<string, Json> }[]
  readonly pending: Map<number, { method: string; args: Json }>
  readonly logs: { level: string; message: string }[]
  /** reportError requests. */
  readonly errors: { message: string; stack?: string; fatal?: boolean }[]
  readonly exceptions: { where: string; message: string; stack?: string }[]
  readonly violations: string[]
  readonly toasts: { text: string; kind: string }[]
  readonly clipboard: string[]
  readonly menus: Json[]
  readonly dialogs: Json[]
  readonly shared: Json[]
  readonly saved: Json[]
  readonly opened: string[]
  readonly focusRequests: Json[]
  readonly subscriptions: Map<number, { trigger: string; config: Json }>
  readonly storage: Map<string, Json>
  readonly deck: Card[]
  readonly roots: Map<number, number>
  readonly droppedWrites: { node: number; text: string; count: number; hostCount: number; hostText: string }[]
  readonly failed: { kind: string; message: string } | null
  readonly envState: Env
  clock: number

  initInfo(): Json
  settle(): Promise<void>
  mount(name?: string, params?: Json): Promise<number>
  unmount(inst: number): Promise<void>
  back(): Promise<void>
  tick(ms?: number): Promise<void>
  deliver(entries: unknown[][]): Promise<void>
  respond(reqId: number, value: Json): Promise<void>
  fail(reqId: number, code: string, message: string): Promise<void>
  env(changes: Partial<Env>): Promise<void>
  emitTrigger(type: string, payload: Json): Promise<number>
  find(target: Target): NodeView
  findAll(target: Target, options?: { templates?: boolean; within?: number }): NodeView[]
  tree(page?: number): NodeView | null
  prop(target: Target, name: string): unknown
  emit(target: Target, event: string, args?: Json[]): Promise<void>
  press(target: Target): Promise<void>
  longPress(target: Target, args?: Json[]): Promise<void>
  type(target: Target, texts: string | string[]): Promise<void>
  typeWhileBusy(target: Target, text: string): void
  submit(target: Target): Promise<void>
  toggle(target: Target): Promise<void>
  /** A Segmented's option, or a Lanes' lane (one per page): shown at once, then `select [index]`. */
  select(target: Target, index: number): Promise<void>
  /** A tap on a link of a Markdown: `link [href]` (`href` must be in its text, before limits.maxMarkdownChars). */
  link(target: Target, href: string): Promise<void>
  /**
   * Drags item `key` out of the List `source` (default: `target`, a reorder) and drops it on the List
   * `target` at `index` (among the target's items once the item is out of its old place): `drop [key,
   * index, source id]` on the target. Both Lists need the same `dragGroup`, and the target `onDrop`.
   */
  drop(target: Target, key: string | number, index: number, source?: Target): Promise<void>
  /** A List's items as the host holds them. */
  items(target: Target): Json[]
  /** The keys of a List's items, in order. */
  keys(target: Target): (string | number)[]
  /** A Lanes as the host has it: titles, counts (null for none), the selected lane and the lanes' nodes. */
  lanes(target: Target): { titles: string[]; counts: (string | null)[]; selected: number; lanes: NodeView[] }
  itemEmit(list: Target, key: string | number, node: Target | undefined, event: string, args?: Json[]): Promise<void>
  itemPress(list: Target, key: string | number, node?: Target): Promise<void>
  itemLongPress(list: Target, key: string | number, node?: Target): Promise<void>
  memory(): Record<string, number>
  dispose(): void
}

export declare function createTestHost(options: TestHostOptions): Promise<TestHost>

export interface Decoder {
  decode(
    message: string | unknown[][],
    direction: 'fromVm' | 'toVm',
    context?: Record<string, string>,
  ): Record<string, unknown>[]
}
export declare function createDecoder(): Decoder
export declare function decodeMessage(
  message: string | unknown[][],
  direction: 'fromVm' | 'toVm',
): Record<string, unknown>[]
export declare class HostTree {}
