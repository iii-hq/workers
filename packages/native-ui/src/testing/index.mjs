/**
 * The test harness (design §10.5): a bundle in quickjs-emscripten with the
 * native VM's globals (fake-clock timers, console, performance.now, no
 * eval), its heap cap and step deadline, driven turn by turn like the app's
 * WorkerVm loop (deliver, timers, microtasks) and answered by a fake host
 * that keeps the tree, the requests, the subscriptions and the deck.
 */

import { Buffer } from 'node:buffer'
import { getQuickJS } from 'quickjs-emscripten'
import { HostTree } from './host-tree.mjs'
import { PRELUDE } from './prelude.mjs'
import { catalog, eventsByName, propByName, requestsByMethod, typesByName, typesByTag } from './protocol.mjs'

export { createDecoder, decodeMessage } from './protocol.mjs'
export { HostTree }

/** A failure a fake host answers a request with (`calls` handlers throw it). */
export class HostError extends Error {
  constructor(code, message) {
    super(message)
    this.name = 'HostError'
    this.code = code
  }
}

/** The VM stopped for good (deadline, memory, a fatal reportError). */
export class VmFailure extends Error {
  constructor(kind, message) {
    super(`${kind}: ${message}`)
    this.name = 'VmFailure'
    this.kind = kind
  }
}

export const DEFAULT_ENV = Object.freeze({
  windowClass: 'compact',
  posture: 'flat',
  platform: 'android',
  visible: true,
  theme: 'dark',
  locale: 'en-US',
  online: true,
})

const LOG_LEVELS = ['debug', 'info', 'warn', 'error']
const URL_SCHEMES = /^(https?|mailto):/i

/** Creates a host, evaluates `bundle` and waits for its `ready`; `init` goes with the first action. */
export async function createTestHost(options) {
  const QuickJS = await getQuickJS()
  const host = new TestHost(QuickJS, options)
  host._load()
  return host
}

class TestHost {
  constructor(QuickJS, options) {
    const o = options ?? {}
    if (typeof o.bundle !== 'string') throw new Error('createTestHost needs the bundle source')
    this.options = o
    this.ns = o.ns ?? 'demo'
    this.appKey = o.appKey ?? `worker:${this.ns}`
    this.envState = { ...DEFAULT_ENV, ...(o.env ?? {}) }
    this.calls = o.calls ?? {}
    this.uiAnswers = o.ui ?? {}
    this.strict = o.strict ?? true
    this.clock = o.now ?? 0
    this.stepBudgetMs = o.stepBudgetMs ?? catalog.limits.stepBudgetMs
    this.loadBudgetMs = o.loadBudgetMs ?? catalog.limits.loadBudgetMs
    this.maxMessageBytes = o.maxMessageBytes ?? catalog.limits.maxMessageBytes
    this.logSink = o.log ?? null

    /** VM → host messages, json-v0. */
    this.messages = []
    /** Every VM → host entry, parsed, in order. */
    this.ops = []
    /** Host → VM messages delivered, json-v0. */
    this.toVm = []
    /** Both directions in order: `{ dir: 'toVm' | 'fromVm', message }`. */
    this.transcript = []
    /** Every request: `{ reqId, method, args }` (method by name). */
    this.requests = []
    this.logs = []
    /** `reportError` requests: `{ message, stack?, fatal? }`. */
    this.errors = []
    /** Uncaught errors in the VM (a turn, a timer, a microtask). */
    this.exceptions = []
    /** Broken protocol guarantees. */
    this.violations = []
    this.toasts = []
    this.clipboard = []
    this.menus = []
    this.dialogs = []
    this.shared = []
    this.saved = []
    this.opened = []
    this.focusRequests = []
    /** sub → { trigger, config } */
    this.subscriptions = new Map()
    this.storage = new Map(Object.entries(o.storage ?? {}))
    /** The cards: `{ inst, name, params, reqId }` (reqId of the navPush that made it, or null). */
    this.deck = []
    /** Requests nobody answered yet: reqId → { method, args }. */
    this.pending = new Map()
    this.ready = null
    this.failed = null
    this.model = new HostTree((p) => this._violation(p))
    this.queue = []
    this.inflight = new Set()
    this.outbox = []
    this.busyEdits = []
    this.unmountedRoots = []
    this.nextInst = 1
    this.deadline = Number.POSITIVE_INFINITY
    this.reported = { exceptions: 0, violations: 0 }

    this.rt = QuickJS.newRuntime()
    this.rt.setMemoryLimit(o.memoryLimit ?? catalog.limits.heapBytesDefault)
    this.rt.setMaxStackSize(o.stackSize ?? 1024 * 1024)
    this.rt.setInterruptHandler(() => Date.now() > this.deadline)
    this.ctx = this.rt.newContext()
  }

  // ── VM plumbing ───────────────────────────────────────────────────────────

  _load() {
    const ctx = this.ctx
    const fns = [
      ctx.newFunction('hostLog', (level, text) => {
        this._log(ctx.getNumber(level), ctx.getString(text))
      }),
      ctx.newFunction('hostNow', () => ctx.newNumber(this.clock)),
      ctx.newFunction('hostFail', (kind, text) => {
        this.exceptions.push({ where: ctx.getString(kind), message: ctx.getString(text) })
      }),
      ctx.newFunction('hostSend', (text) => {
        const s = ctx.getString(text)
        if (Buffer.byteLength(s, 'utf8') > this.maxMessageBytes) return ctx.newNumber(1)
        this.outbox.push(s)
        return ctx.newNumber(0)
      }),
      ctx.newNumber(64),
    ]
    const prelude = this._unwrap(ctx.evalCode(PRELUDE, 'iiiui-prelude.js', { type: 'global', strict: true }), 'prelude')
    const hooks = this._unwrap(ctx.callFunction(prelude, ctx.undefined, ...fns), 'prelude')
    prelude.dispose()
    for (const f of fns) f.dispose()
    this.runTimersFn = ctx.getProp(hooks, 'runTimers')
    this.nextTimerFn = ctx.getProp(hooks, 'nextTimer')
    hooks.dispose()

    if (this.options.rawRequests) {
      ctx.setProp(ctx.global, '__IIIUI_TEST_RAW__', ctx.true)
    }
    if (this.options.ids) {
      const seed = this._unwrap(
        ctx.evalCode(`(${JSON.stringify(this.options.ids)})`, 'seed.js', { type: 'global' }),
        'seed',
      )
      ctx.setProp(ctx.global, '__IIIUI_TEST_IDS__', seed)
      seed.dispose()
    }

    this._budget(this.loadBudgetMs, () => {
      const r = ctx.evalCode(this.options.bundle, 'native.js', { type: 'global' })
      const v = this._unwrap(r, 'load', true)
      v?.dispose()
      this._runJobs('load')
    })
    this._drainOutbox()
    if (!this.ready) {
      this._fail('load', "This bundle didn't call defineApp (no ready during the load)")
    }
    if (this.options.autoInit !== false) this._enqueue([1, this.initInfo()])
    this._report()
  }

  /** The `init` info this host sends (design §5.6). */
  initInfo() {
    return {
      catalog: catalog.catalogVersion,
      features: [...catalog.features],
      host: this.options.host ?? 'android 0.1.0',
      engine: this.options.engine ?? 'quickjs-emscripten 0.31.0',
      app: { key: this.appKey, ns: this.ns },
      env: { ...this.envState },
    }
  }

  _budget(ms, fn) {
    this.deadline = Date.now() + ms
    try {
      return fn()
    } finally {
      this.deadline = Number.POSITIVE_INFINITY
    }
  }

  /** The value of a VM call result, or null after recording its error (fatal ones throw). */
  _unwrap(result, where, keep = false) {
    if (result.error) {
      const err = this.ctx.dump(result.error)
      result.error.dispose()
      const message = err && typeof err === 'object' ? `${err.name}: ${err.message}` : String(err)
      if (/interrupted/i.test(message))
        this._fail('watchdog', `a script ran for more than ${this.stepBudgetMs} ms (${where})`)
      if (/out of memory/i.test(message)) this._fail('memory', `the VM ran out of memory (${where})`)
      this.exceptions.push({ where, message, stack: err?.stack })
      if (where === 'prelude' || where === 'load') this._fail(where, message)
      return null
    }
    if (!keep) return result.value
    return result.value
  }

  _runJobs(where) {
    const r = this.rt.executePendingJobs(-1)
    if (r.error) {
      this._unwrap(r, where)
      return
    }
    r.dispose?.()
  }

  _fail(kind, message) {
    this.failed ??= { kind, message }
    throw new VmFailure(kind, message)
  }

  _violation(problem) {
    this.violations.push(problem)
  }

  _log(level, message) {
    const entry = { level: LOG_LEVELS[level] ?? 'info', message }
    this.logs.push(entry)
    this.logSink?.(entry.level, message)
  }

  _enqueue(entry, key) {
    const text = JSON.stringify(entry)
    if (key) {
      const i = this.queue.findIndex((q) => q.key === key)
      if (i >= 0) {
        this.queue[i] = { text, key }
        return
      }
    }
    this.queue.push({ text, key })
  }

  _timerDue() {
    const r = this.ctx.callFunction(this.nextTimerFn, this.ctx.undefined)
    const v = this._unwrap(r, 'nextTimer')
    const due = v ? this.ctx.getNumber(v) : -1
    v?.dispose()
    return due >= 0 && due <= this.clock ? true : due
  }

  /** One VM turn: deliver the queue, run due timers, run microtasks, then read what the VM sent. */
  _turn() {
    if (this.failed) throw new VmFailure(this.failed.kind, this.failed.message)
    const entries = this.queue.splice(0)
    const unmounted = this.unmountedRoots.splice(0)
    this._budget(this.stepBudgetMs, () => {
      if (entries.length) {
        const message = `[${entries.map((e) => e.text).join(',')}]`
        this.toVm.push(message)
        this.transcript.push({ dir: 'toVm', message })
        const receive = this.ctx.getProp(this.ctx.global, '__vm_receive')
        if (this.ctx.typeof(receive) !== 'function') {
          receive.dispose()
          this._fail('protocol', 'the bundle defines no __vm_receive')
        }
        const arg = this.ctx.newString(message)
        this._unwrap(this.ctx.callFunction(receive, this.ctx.undefined, arg), 'deliver')?.dispose()
        arg.dispose()
        receive.dispose()
      }
    })
    this._budget(this.stepBudgetMs, () => {
      this._unwrap(this.ctx.callFunction(this.runTimersFn, this.ctx.undefined), 'timers')?.dispose()
    })
    this._budget(this.stepBudgetMs, () => this._runJobs('jobs'))
    for (const id of unmounted) this.model.touched.add(id)
    this._drainOutbox()
    if (unmounted.length) this.model.endOfMessage()
  }

  _drainOutbox() {
    while (this.outbox.length) this._receive(this.outbox.shift())
  }

  _receive(message) {
    // Edits the person made while the VM ran reach the host before this message.
    for (const { id, text } of this.busyEdits.splice(0)) {
      this._enqueue([4, id, this.model.edit(id, text), text], `text:${id}`)
    }
    this.messages.push(message)
    this.transcript.push({ dir: 'fromVm', message })
    let entries
    try {
      entries = JSON.parse(message)
    } catch {
      return this._violation(`a message is not JSON: ${message.slice(0, 80)}`)
    }
    if (!Array.isArray(entries) || entries.length === 0)
      return this._violation(`a message must be a non-empty array: ${message}`)
    for (const e of entries) {
      this.ops.push(e)
      if (!Array.isArray(e)) {
        this._violation(`an entry is not an array: ${JSON.stringify(e)}`)
        continue
      }
      if (e[0] === 20) this._request(e[1], e[2], e[3] ?? {})
      else if (e[0] === 23) {
        if (this.ready) this._violation('ready came twice')
        this.ready = e[1]
      } else this.model.apply(e)
    }
    this.model.endOfMessage()
  }

  /** Throws the problems of the last action in strict mode. */
  _report() {
    if (!this.strict) return
    const exceptions = this.exceptions.slice(this.reported.exceptions)
    const violations = this.violations.slice(this.reported.violations)
    this.reported = { exceptions: this.exceptions.length, violations: this.violations.length }
    const problems = [
      ...exceptions.map((e) => `uncaught in ${e.where}: ${e.message}`),
      ...violations.map((v) => `protocol: ${v}`),
    ]
    if (problems.length) throw new Error(`native-ui test host:\n  ${problems.join('\n  ')}`)
  }

  // ── Requests ──────────────────────────────────────────────────────────────

  _allowed(id) {
    return typeof id === 'string' && id.startsWith(`${this.ns}::`) && !/^(iii|engine)::/.test(id)
  }

  _respond(reqId, ok, value) {
    if (!this.pending.delete(reqId)) return
    this._enqueue([8, reqId, ok ? 1 : 0, ok ? (value === undefined ? null : value) : value])
  }

  _error(reqId, code, message) {
    this._respond(reqId, false, { code, message })
  }

  /** Answers with `value`, or later when it is a promise (or a function returning one). */
  _answer(reqId, value, map = (v) => v) {
    const settle = (v) => this._respond(reqId, true, map(v))
    const failed = (e) =>
      this._error(reqId, typeof e?.code === 'string' ? e.code : 'invocation_failed', e?.message ?? String(e))
    if (typeof value === 'function' || (value && typeof value.then === 'function')) {
      const p = Promise.resolve()
        .then(() => (typeof value === 'function' ? value() : value))
        .then(settle, failed)
      this.inflight.add(p)
      p.finally(() => this.inflight.delete(p))
      return
    }
    settle(value)
  }

  _request(reqId, method, args) {
    const name = requestsByMethod.get(method)?.name
    this.requests.push({ reqId, method: name ?? method, args })
    this.pending.set(reqId, { method: name ?? method, args })
    const ns = this.ns
    const ui = this.uiAnswers
    switch (name) {
      case 'call': {
        if (!this._allowed(args.fn)) return this._error(reqId, 'forbidden', `${ns} may only call ${ns}::… functions`)
        const handler = this.calls[args.fn]
        if (handler === undefined) {
          return this._answer(reqId, () => {
            throw new HostError('function_not_found', `Function ${args.fn} not found`)
          })
        }
        return this._answer(reqId, () =>
          typeof handler === 'function' ? handler(args.payload ?? null, this) : handler,
        )
      }
      case 'on':
        if (!this._allowed(args.trigger))
          return this._error(reqId, 'forbidden', `${ns} may only bind ${ns}::… triggers`)
        this.subscriptions.set(args.sub, { trigger: args.trigger, config: args.config ?? {} })
        return this._respond(reqId, true, null)
      case 'off':
        this.subscriptions.delete(args.sub)
        return this._respond(reqId, true, null)
      case 'navPush': {
        if (!this.ready?.pages.some((p) => p.name === args.page)) {
          return this._error(reqId, 'invalid', `"${args.page}" is not a page of this app`)
        }
        const params = args.params ?? {}
        const top = this.deck.at(-1)
        if (top && top.name === args.page && JSON.stringify(top.params) === JSON.stringify(params)) {
          return this._respond(reqId, true, { result: null })
        }
        const inst = this.nextInst++
        this.deck.push({ inst, name: args.page, params, reqId })
        return this._enqueue([6, inst, args.page, params])
      }
      case 'navPop': {
        const top = this.deck.at(-1)
        if (!top || top.inst !== args.page || top.reqId === null) {
          return this._error(reqId, 'invalid', `page ${args.page} is not the top card`)
        }
        this._popTop(args.result ?? null)
        return this._respond(reqId, true, null)
      }
      case 'navReplace': {
        const card = this.deck.find((c) => c.inst === args.page)
        if (!card) return this._error(reqId, 'invalid', `page ${args.page} is not on the deck`)
        if (!this.ready?.pages.some((p) => p.name === args.name)) {
          return this._error(reqId, 'invalid', `"${args.name}" is not a page of this app`)
        }
        this._unmountCard(card.inst)
        card.inst = this.nextInst++
        card.name = args.name
        card.params = args.params ?? {}
        this._enqueue([6, card.inst, card.name, card.params])
        return this._respond(reqId, true, null)
      }
      case 'navPopToRoot':
        while (this.deck.at(-1)?.reqId != null) this._popTop(null)
        return this._respond(reqId, true, null)
      case 'toast':
        this.toasts.push({ text: args.text, kind: catalog.enums.toastKind[args.kind ?? 0] })
        return this._respond(reqId, true, null)
      case 'actionMenu':
        this.menus.push(args)
        return this._answer(reqId, ui.actionMenu ? () => ui.actionMenu(args, this) : -1, (index) => ({ index }))
      case 'confirm':
        this.dialogs.push({ kind: 'confirm', ...args })
        return this._answer(reqId, ui.confirm ? () => ui.confirm(args, this) : false, (ok) => ({ ok: ok === true }))
      case 'prompt':
        this.dialogs.push({ kind: 'prompt', ...args })
        return this._answer(reqId, ui.prompt ? () => ui.prompt(args, this) : null, (text) => ({ text: text ?? null }))
      case 'focus':
        this.focusRequests.push(args)
        return this._respond(reqId, true, null)
      case 'clipboardWrite':
        this.clipboard.push(args.text)
        return this._respond(reqId, true, null)
      case 'openUrl':
        if (!URL_SCHEMES.test(String(args.url))) return this._error(reqId, 'forbidden', 'only http, https and mailto')
        this.opened.push(args.url)
        return this._answer(reqId, ui.openUrl ? () => ui.openUrl(args.url, this) : true, (opened) => ({
          opened: opened === true,
        }))
      case 'share':
        this.shared.push(args)
        return this._respond(reqId, true, null)
      case 'saveFile':
        this.saved.push(args)
        return this._respond(reqId, true, { path: `Downloads/iii/${args.filename}` })
      case 'storageGet':
        return this._respond(reqId, true, { value: this.storage.has(args.key) ? this.storage.get(args.key) : null })
      case 'storageSet': {
        const size = Buffer.byteLength(JSON.stringify(args.value ?? null), 'utf8')
        let total = size
        for (const [k, v] of this.storage) if (k !== args.key) total += Buffer.byteLength(JSON.stringify(v), 'utf8')
        if (size > catalog.limits.maxStorageValueBytes || total > catalog.limits.maxStorageBytesPerWorker) {
          return this._error(reqId, 'quota', 'storage is full')
        }
        this.storage.set(args.key, args.value ?? null)
        return this._respond(reqId, true, null)
      }
      case 'storageRemove':
        this.storage.delete(args.key)
        return this._respond(reqId, true, null)
      case 'storageKeys':
        return this._respond(reqId, true, { keys: [...this.storage.keys()] })
      case 'reportError':
        this.errors.push(args)
        if (args.fatal) this.failed ??= { kind: 'reported', message: args.message }
        return this._respond(reqId, true, null)
      default:
        return this._error(reqId, 'unsupported', `unknown request method ${method}`)
    }
  }

  _popTop(result) {
    const card = this.deck.pop()
    this._unmountCard(card.inst)
    if (card.reqId != null) this._respond(card.reqId, true, { result })
  }

  /** The card left the deck: the host forgets its root (the script destroys it) and sends `unmount`. */
  _unmountCard(inst) {
    const rootId = this.model.roots.get(inst)
    this.model.roots.delete(inst)
    // Checked once the script got the unmount: by then it must have destroyed the root.
    if (rootId !== undefined) this.unmountedRoots.push(rootId)
    this._enqueue([7, inst])
  }

  // ── Driving ───────────────────────────────────────────────────────────────

  /**
   * Runs turns until nothing is left to do now: queued entries, due timers,
   * microtasks and the answers of `calls` handlers that already settled.
   */
  async settle() {
    for (let guard = 0; guard < 10_000; guard++) {
      if (this.failed) break
      // Handlers that settle at once answer in the same delivery as the
      // synchronous answers (the app answers an `on` before the engine
      // answers a call, design §5.7).
      if (this.inflight.size) await new Promise((r) => setImmediate(r))
      if (this.queue.length || this._timerDue() === true || this.rt.hasPendingJob()) {
        this._turn()
        continue
      }
      break
    }
    this._report()
    if (this.failed) throw new VmFailure(this.failed.kind, this.failed.message)
  }

  /** Mounts page `name` as a new card; resolves with its page instance id. */
  async mount(name = 'root', params = {}) {
    const inst = this.nextInst++
    this.deck.push({ inst, name, params, reqId: null })
    this._enqueue([6, inst, name, params])
    await this.settle()
    return inst
  }

  async unmount(inst) {
    this.deck = this.deck.filter((c) => c.inst !== inst)
    this._unmountCard(inst)
    await this.settle()
  }

  /** Back on the top card: unmounts it and answers its navPush with `{result: null}`. */
  async back() {
    const top = this.deck.at(-1)
    if (!top || top.reqId == null) throw new Error('back: the top card was not pushed by the app')
    this._popTop(null)
    await this.settle()
  }

  /** Moves the fake clock `ms` forward, running each timer when it falls due. */
  async tick(ms = 0) {
    const target = this.clock + ms
    for (;;) {
      await this.settle()
      const due = this._timerDue()
      if (due === true) continue
      if (due < 0 || due > target) break
      this.clock = due
    }
    this.clock = target
    await this.settle()
  }

  /** Sends host → VM entries as they are (the conformance flows). */
  async deliver(entries) {
    for (const e of entries) this._enqueue(e)
    await this.settle()
  }

  /** Answers a pending request by hand. */
  async respond(reqId, value) {
    this._respond(reqId, true, value)
    await this.settle()
  }

  async fail(reqId, code, message) {
    this._error(reqId, code, message)
    await this.settle()
  }

  async env(changes) {
    const diff = {}
    for (const [k, v] of Object.entries(changes)) {
      if (this.envState[k] !== v) diff[k] = v
    }
    Object.assign(this.envState, diff)
    if (Object.keys(diff).length) this._enqueue([5, diff])
    await this.settle()
  }

  /** Delivers `payload` to every subscription bound to `type`; resolves with how many got it. */
  async emitTrigger(type, payload) {
    let n = 0
    for (const [sub, s] of this.subscriptions) {
      if (s.trigger !== type) continue
      this._enqueue([9, sub, payload ?? null])
      n++
    }
    await this.settle()
    return n
  }

  // ── Nodes ─────────────────────────────────────────────────────────────────

  /**
   * A node attached under a page root (List templates excluded): `target`
   * is a testTag, a node id, or `{ type, ...props }` (prop values in the
   * readable form of `tree()`).
   */
  find(target) {
    const all = this.findAll(target)
    if (!all.length) throw new Error(`no node matches ${JSON.stringify(target)}`)
    if (all.length > 1) throw new Error(`${all.length} nodes match ${JSON.stringify(target)}`)
    return all[0]
  }

  findAll(target, { templates = false, within } = {}) {
    const nodes = within ? [...this._subtree(within)] : [...this.model.walk({ templates })]
    return nodes.filter((n) => this._matches(n, target)).map((n) => this.model.view(n.id))
  }

  *_subtree(id) {
    const n = this.model.nodes.get(id)
    if (!n) return
    yield n
    for (const list of n.slots.values()) for (const c of list) yield* this._subtree(c)
  }

  _matches(n, target) {
    if (typeof target === 'number') return n.id === target
    if (typeof target === 'string') return n.props.get(3) === target
    const view = this.model.view(n.id)
    for (const [k, v] of Object.entries(target)) {
      if (k === 'type') {
        if (view.type !== v) return false
      } else if (JSON.stringify(view.props?.[k]) !== JSON.stringify(v)) return false
    }
    return true
  }

  /** The named tree of a page instance (default: the top card). */
  tree(page) {
    const inst = page ?? this.deck.at(-1)?.inst
    const id = this.model.roots.get(inst)
    return id === undefined ? null : this.model.view(id)
  }

  /** page instance → root node id. */
  get roots() {
    return this.model.roots
  }

  get droppedWrites() {
    return this.model.droppedWrites
  }

  _subscribed(id, event) {
    const n = this.model.nodes.get(id)
    const mask = n?.props.get(2) ?? 0
    const tag = eventsByName.get(event)?.tag
    if (tag === undefined) throw new Error(`unknown event ${event}`)
    if (!(mask & (1 << tag))) {
      throw new Error(`${typesByTag.get(n?.type)?.name ?? 'node'} ${id} is not subscribed to ${event}`)
    }
    return tag
  }

  /** Sends event `event` of the node `target` with `args`, as the host does for a subscribed event. */
  async emit(target, event, args = []) {
    const { id } = this.find(target)
    const tag = this._subscribed(id, event)
    this._enqueue([2, id, tag, args])
    await this.settle()
  }

  press(target) {
    return this.emit(target, 'press')
  }

  longPress(target, args = []) {
    return this.emit(target, 'longPress', args)
  }

  /** The person types: each string of `texts` is one edit; the edits of one turn coalesce into one `text` entry. */
  async type(target, texts) {
    const { id, type } = this.find(target)
    if (type !== 'TextField') throw new Error(`type: node ${id} is a ${type}`)
    this._subscribed(id, 'change')
    for (const text of Array.isArray(texts) ? texts : [texts]) {
      const count = this.model.edit(id, text)
      this._enqueue([4, id, count, text], `text:${id}`)
    }
    await this.settle()
  }

  /**
   * The person types `text` while the VM runs its next turn: the edit
   * reaches the host before the message of that turn (so a write in it
   * carries an old count and is dropped), and its `text` entry goes in the
   * turn after. Drive the turn with the next action (a trigger, a tick).
   */
  typeWhileBusy(target, text) {
    const { id, type } = this.find(target)
    if (type !== 'TextField') throw new Error(`typeWhileBusy: node ${id} is a ${type}`)
    this._subscribed(id, 'change')
    this.busyEdits.push({ id, text })
  }

  /** The IME action (or Enter) on a TextField: `submit` with the host's text. */
  async submit(target) {
    const node = this.find(target)
    await this.emit(node.id, 'submit', [node.props?.value ?? ''])
  }

  /** The person flips a Switch: the host shows the new value at once, then sends `change`. */
  async toggle(target) {
    const { id } = this.find(target)
    const n = this.model.nodes.get(id)
    const checked = !(n.props.get(16) ?? false)
    this._subscribed(id, 'change')
    n.props.set(16, checked)
    this._enqueue([2, id, eventsByName.get('change').tag, [checked]])
    await this.settle()
  }

  /** The person picks option `index` of a Segmented (shown at once, then `select`). */
  async select(target, index) {
    const { id } = this.find(target)
    this._subscribed(id, 'select')
    this.model.nodes.get(id).props.set(17, index)
    this._enqueue([2, id, eventsByName.get('select').tag, [index]])
    await this.settle()
  }

  /**
   * An event of a List row: `node` is a testTag (or a matcher) inside the
   * item's template; without it, the template's root.
   */
  async itemEmit(listTarget, key, node, event, args = []) {
    const list = this.model.nodes.get(this.find(listTarget).id)
    if (list.type !== typesByName.get('List').tag) throw new Error('itemEmit: not a List')
    const keyField = list.props.get(16) ?? 'key'
    const item = list.items.find((it) => it[keyField] === key)
    if (!item) throw new Error(`itemEmit: no item with key ${JSON.stringify(key)}`)
    const kindField = list.props.get(17)
    const kind = kindField ? Number(item[kindField] ?? 0) : 0
    const template = list.slots.get(1 + kind)?.[0]
    if (template === undefined) throw new Error(`itemEmit: the List has no template ${kind}`)
    const nodeId = node === undefined ? template : this._findIn(template, node)
    const tag = this._subscribed(nodeId, event)
    this._enqueue([3, list.id, key, nodeId, tag, args])
    await this.settle()
  }

  _findIn(root, target) {
    const all = [...this._subtree(root)].filter((n) => this._matches(n, target))
    if (all.length !== 1) throw new Error(`${all.length} template nodes match ${JSON.stringify(target)}`)
    return all[0].id
  }

  itemPress(listTarget, key, node) {
    return this.itemEmit(listTarget, key, node, 'press')
  }

  itemLongPress(listTarget, key, node) {
    return this.itemEmit(listTarget, key, node, 'longPress')
  }

  /** What a prop of a node holds now, readable (`find(t).props[name]`). */
  prop(target, name) {
    const node = this.find(target)
    const spec = propByName(typesByName.get(node.type)?.tag, name)
    if (!spec) throw new Error(`${node.type} has no prop ${name}`)
    return node.props?.[name]
  }

  /** The engine's heap use (`JSMemoryUsage` subset). */
  memory() {
    const handle = this.rt.computeMemoryUsage()
    const usage = this.ctx.dump(handle)
    handle.dispose()
    return usage
  }

  dispose() {
    try {
      this.runTimersFn?.dispose()
      this.nextTimerFn?.dispose()
      this.ctx.dispose()
      this.rt.dispose()
    } catch {
      // A failed VM may hold handles the runtime frees with it.
    }
  }
}
