/**
 * Evaluated in the harness's QuickJS context before the bundle: the globals
 * the native VM installs (IIIUI_HOST_GLOBALS and IIIUI_NO_EVAL, design
 * §3.1), with timers on the harness's fake clock. It is a function the
 * harness calls with its host functions; it returns the hooks the harness
 * drives the timers with, so nothing of the harness is left on the global.
 */
export const PRELUDE = `(function (hostLog, hostNow, hostFail, hostSend, maxDepth) {
  'use strict'
  const g = globalThis
  const stringify = JSON.stringify
  const resolved = Promise.resolve()
  const define = (obj, key, value) =>
    Object.defineProperty(obj, key, { value, writable: true, enumerable: false, configurable: true })

  const show = (v) => {
    if (typeof v === 'string') return v
    if (v instanceof Error) return v.name + ': ' + v.message + (v.stack ? '\\n' + v.stack : '')
    try {
      const s = stringify(v)
      return s === undefined ? String(v) : s
    } catch (e) {
      return String(v)
    }
  }
  // What the engine throws to stop the VM: never swallowed (the interrupt is
  // uncatchable anyway; out of memory is not, and it is fatal on the device).
  const isStop = (e) => e instanceof Error && e.name === 'InternalError' && /out of memory|interrupted/.test(e.message)
  const logger = (level) => (...args) => { hostLog(level, args.map(show).join(' ')) }
  define(g, 'console', { debug: logger(0), log: logger(1), info: logger(1), warn: logger(2), error: logger(3) })
  define(g, 'performance', { now: () => hostNow() })

  // Timers: ms truncated to [0, 2^31 - 1], intervals at least 4 ms, ids from 1.
  let nextId = 1
  const timers = new Map()
  const clamp = (ms) => {
    const n = Math.trunc(Number(ms) || 0)
    return n < 0 ? 0 : n > 2147483647 ? 2147483647 : n
  }
  const add = (fn, ms, args, repeat) => {
    if (typeof fn !== 'function') throw new TypeError('a timer needs a function')
    let delay = clamp(ms)
    if (repeat && delay < 4) delay = 4
    const id = nextId++
    timers.set(id, { fn, args, due: hostNow() + delay, every: repeat ? delay : 0 })
    return id
  }
  define(g, 'setTimeout', (fn, ms, ...args) => add(fn, ms, args, false))
  define(g, 'setInterval', (fn, ms, ...args) => add(fn, ms, args, true))
  define(g, 'clearTimeout', (id) => { timers.delete(id) })
  define(g, 'clearInterval', (id) => { timers.delete(id) })
  define(g, 'queueMicrotask', (fn) => {
    if (typeof fn !== 'function') throw new TypeError('queueMicrotask needs a function')
    resolved.then(() => {
      try { fn() } catch (e) { hostFail('microtask', show(e)) }
    })
  })

  // VM → host: the entries array, as json-v0 here (bin-v1 on a device).
  const deep = (v, depth) => {
    if (v === null || typeof v !== 'object') return true
    if (depth > maxDepth) return false
    const values = Array.isArray(v) ? v : Object.keys(v).map((k) => v[k])
    for (const x of values) if (!deep(x, depth + 1)) return false
    return true
  }
  define(g, '__host_send', (entries) => {
    if (!Array.isArray(entries)) throw new TypeError('__host_send takes the array of entries')
    if (!deep(entries, 1)) throw new RangeError('__host_send: the message is nested deeper than ' + maxDepth)
    const status = hostSend(stringify(entries))
    if (status === 1) throw new RangeError('__host_send: the message is larger than limits.maxMessageBytes')
  })

  // No eval: eval(), new Function() and every function constructor throw.
  const deny = (what) => function () { throw new EvalError(what + ' is not available in a native-ui VM') }
  const F = deny('new Function')
  F.prototype = Function.prototype
  Object.defineProperty(Function.prototype, 'constructor', { value: F, writable: false, configurable: false })
  for (const sample of [async function () {}, function* () {}, async function* () {}]) {
    Object.defineProperty(Object.getPrototypeOf(sample), 'constructor', {
      value: deny('new Function'), writable: false, configurable: false,
    })
  }
  define(g, 'Function', F)
  define(g, 'eval', deny('eval'))

  const runTimers = () => {
    const now = hostNow()
    const due = [...timers].filter(([, t]) => t.due <= now).sort((a, b) => a[1].due - b[1].due || a[0] - b[0])
    let ran = 0
    for (const [id, t] of due) {
      if (timers.get(id) !== t) continue
      if (t.every) t.due = now + t.every
      else timers.delete(id)
      ran++
      try { t.fn(...t.args) } catch (e) {
        if (isStop(e)) throw e
        hostFail('timer', show(e))
      }
    }
    return ran
  }
  const nextTimer = () => {
    let min = -1
    for (const t of timers.values()) if (min < 0 || t.due < min) min = t.due
    return min
  }
  return { runTimers, nextTimer }
})`
