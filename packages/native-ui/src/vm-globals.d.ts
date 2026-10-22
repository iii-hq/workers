/**
 * The globals of a native-ui VM besides ECMAScript itself (design §3.1,
 * §4.1): console, timers, microtasks and performance.now. Nothing else:
 * no DOM, fetch, URL, TextEncoder, atob/btoa or Intl. Declared with the
 * same names as lib.dom and @types/node, so they merge when those are
 * loaded too.
 */

interface Console {
  debug(...data: unknown[]): void
  log(...data: unknown[]): void
  info(...data: unknown[]): void
  warn(...data: unknown[]): void
  error(...data: unknown[]): void
}
declare var console: Console

interface Performance {
  /** Monotonic milliseconds since the VM started. */
  now(): number
}
declare var performance: Performance

declare function setTimeout(handler: (...args: any[]) => void, timeout?: number, ...args: any[]): number
declare function clearTimeout(id: number | undefined): void
declare function setInterval(handler: (...args: any[]) => void, timeout?: number, ...args: any[]): number
declare function clearInterval(id: number | undefined): void
declare function queueMicrotask(callback: () => void): void
