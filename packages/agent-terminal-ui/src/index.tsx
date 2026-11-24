/**
 * The console page an agent-CLI worker injects: one full-pane terminal that
 * always runs that agent, never a shell.
 *
 * The session itself belongs to the `shell` worker — `shell::pty::open` with
 * a `program` is the whole terminal, so this page is the only new part. The
 * agent worker answers `<worker>::terminal::describe` with what to run and
 * where; the page never chooses a program, which is what keeps a terminal
 * page from being a shell in disguise.
 *
 * Output arrives through a browser-registered bus handler owned at module
 * scope (a React-owned subscription would be disposed by Strict Mode).
 * Delivery over the bus is NOT ordered, so frames apply through an ordered
 * writer: only contiguous sequences reach xterm, gaps hold in a pending map,
 * and a stalled gap re-attaches to replay the missing range from the
 * worker's ring buffer. A per-tab lease in sessionStorage reattaches to the
 * live agent across remounts and reloads; because a remount builds an empty
 * xterm, that reattach replays the WHOLE buffer — an agent TUI paints on
 * change, so a tail replay would leave an empty pane.
 */
import {
  Chip,
  type ChipTone,
  Eyebrow,
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
  PageSidebar,
  ShadcnButton,
  StatusBar,
} from '@iii-dev/console-ui'
import { MAX_FONT_SIZE, MIN_FONT_SIZE, stepFontSize, useTerminalFontSize } from '@iii-workers/terminal-font'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import { Minus, Plus } from 'lucide-react'
import { type ReactNode, type Ref, useCallback, useEffect, useImperativeHandle, useRef, useState } from 'react'

const STALL_MS = 250
const MAX_QUEUED_EVENTS = 4096
const CALL_TIMEOUT_MS = 10_000
const OPEN_ELSEWHERE = 'It is open in another console tab or window.'

/** What a rail knows of the terminal beside it, and how it shows another session there. */
export interface AgentTerminalApi {
  /** The PTY session on screen; null while none is. */
  current: string | null
  /** The row the terminal was opened for, or that claimed it since; null for none. */
  currentKey: string | null
  /** Whether this browser still holds a terminal it opened for `key`. */
  has(key: string): boolean
  /**
   * The row that runs in `pty` now. A terminal held for another one follows
   * it: a fork, /clear or /resume leaves the row the terminal was opened for.
   */
  claim(pty: string, key: string): void
  /**
   * Show another session: the PTY `pty`, else the one held for `key`, else a
   * new one that runs `spec`, which is what the worker's describe returned.
   * False when that session is gone; rejects with the reason it cannot be
   * shown. The terminal on screen is untouched unless it resolves true.
   */
  open(target: { key: string; pty?: string; spec?: TerminalSpec }): Promise<boolean>
}

/**
 * A rail beside the terminal, on the pane's outer edge. The terminal stays the
 * page; the rail is what a worker shows about its agent next to it, and it
 * collapses to the toggle when the operator wants the width back.
 */
export interface AgentTerminalSidebar {
  /** Accessible name of the rail. */
  label: string
  /** Host-owned key for the rail's width and collapsed state. */
  storageKey: string
  render(host: Host, terminal: AgentTerminalApi): ReactNode
}

export interface AgentTerminalOptions {
  /** Worker name: the page id, the handler prefix, and the describe function. */
  worker: string
  /** Nav title and the name used in status lines ("claude", "pi"). */
  title: string
  description: string
  sidebar?: AgentTerminalSidebar
}

interface PtyOutputEvent {
  session_id: string
  sequence: number | null
  data: string | null
  eof: boolean
  exit_code: number | null
  signal: string | null
  error: string | null
}

interface Lease {
  sessionId: string
  reconnectToken: string
}

interface AttachResponse {
  access_key: string
  reconnect_token: string
  frames: { sequence: number; data: string }[]
  truncated: boolean
  status: SessionSummary['status']
}

interface OpenResponse {
  session_id: string
  access_key: string
  reconnect_token: string
}

interface AuthStatus {
  billing: 'subscription' | 'api-key' | 'none' | 'unknown'
  label: string
  detail: string
}

export interface TerminalSpec {
  program: string
  args: string[]
  cwd: string
  env: Record<string, string>
  detail?: string
}

type Listener = (event: PtyOutputEvent) => void

interface OutputRouter {
  outputFunctionId: string
  subscribe(sessionId: string, listener: Listener): () => void
  drain(sessionId: string): PtyOutputEvent[]
  dispose(): void
}

/** The console injects an untyped host object; this is the boundary. */
type Host = any

function createRouter(host: Host, handler: string): OutputRouter {
  const listeners = new Map<string, Set<Listener>>()
  // Output can arrive before the page subscribes (open() returns after the
  // program already started printing). Queue unsubscribed events per session.
  const queues = new Map<string, PtyOutputEvent[]>()
  const off = host.iii.on(handler, (event: PtyOutputEvent) => {
    const set = listeners.get(event.session_id)
    if (set && set.size > 0) {
      for (const listener of set) listener(event)
      return
    }
    const queue = queues.get(event.session_id) ?? []
    queue.push(event)
    if (queue.length > MAX_QUEUED_EVENTS) queue.shift()
    queues.set(event.session_id, queue)
  })
  return {
    outputFunctionId: `${handler}::${host.iii.browserId}`,
    subscribe(sessionId, listener) {
      const set = listeners.get(sessionId) ?? new Set<Listener>()
      set.add(listener)
      listeners.set(sessionId, set)
      return () => {
        set.delete(listener)
        if (set.size === 0) listeners.delete(sessionId)
      }
    },
    drain(sessionId) {
      const queue = queues.get(sessionId) ?? []
      queues.delete(sessionId)
      return queue
    },
    dispose() {
      off()
      listeners.clear()
      queues.clear()
    },
  }
}

interface OrderedWriterOptions {
  write(bytes: Uint8Array): void
  onApplied(sequence: number): void
  onEof(event: PtyOutputEvent): void
  onStall(): void
}

/**
 * Applies frames to the terminal strictly in sequence order. Out-of-order
 * frames wait in `pending`; duplicates (sequence already applied) drop. A
 * gap that persists past STALL_MS calls onStall — the caller re-attaches and
 * replays the missing range.
 */
function createOrderedWriter(opts: OrderedWriterOptions) {
  let last = 0
  const pending = new Map<number, PtyOutputEvent>()
  // An EOF frame carries no sequence, so it cannot be ordered against frames
  // still in flight: it waits here until nothing is pending. Handing it
  // `last + 1` ended the session before its last output arrived AND took the
  // sequence that output was about to use, which dropped the final chunk.
  let eof: PtyOutputEvent | null = null
  let timer: number | null = null

  const flush = () => {
    while (pending.has(last + 1)) {
      const event = pending.get(last + 1) as PtyOutputEvent
      pending.delete(last + 1)
      last += 1
      if (event.data) opts.write(decodeB64(event.data))
      opts.onApplied(last)
      if (event.eof) opts.onEof(event)
    }
    if (eof !== null && pending.size === 0) {
      const event = eof
      eof = null
      opts.onEof(event)
    }
    if (pending.size > 0) {
      if (timer === null) {
        timer = window.setTimeout(() => {
          timer = null
          opts.onStall()
        }, STALL_MS)
      }
    } else if (timer !== null) {
      window.clearTimeout(timer)
      timer = null
    }
  }

  return {
    /** Set the already-applied base sequence within the SAME session. */
    base(sequence: number) {
      last = sequence
      for (const key of [...pending.keys()]) if (key <= sequence) pending.delete(key)
      flush()
    },
    /**
     * Start over on another session's stream. `base(0)` cannot do this: it
     * deletes keys `<= 0`, which is nothing, so a frame left pending by the
     * session that just died keeps its sequence and paints its bytes into the
     * fresh terminal.
     */
    reset(sequence: number) {
      pending.clear()
      eof = null
      last = sequence
      if (timer !== null) {
        window.clearTimeout(timer)
        timer = null
      }
    },
    feed(event: PtyOutputEvent) {
      if (event.eof && event.sequence == null) {
        eof = event
        flush()
        return
      }
      const sequence = event.sequence ?? last + 1
      if (sequence <= last) return
      pending.set(sequence, { ...event, sequence })
      flush()
    },
    lastSeq: () => last,
    dispose() {
      if (timer !== null) window.clearTimeout(timer)
      pending.clear()
      eof = null
    },
  }
}

function decodeB64(b64: string): Uint8Array {
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))
}

function encodeB64(text: string): string {
  const bytes = new TextEncoder().encode(text)
  let bin = ''
  for (const byte of bytes) bin += String.fromCharCode(byte)
  return btoa(bin)
}

/**
 * The lease lives in `localStorage`, not `sessionStorage`: a session outlives
 * the browser tab that opened it. sessionStorage is cleared when the tab
 * closes, so closing the console and coming back the next morning left the
 * running agent unreachable and started a second one beside it — two agents
 * in one workspace, one of them invisible.
 *
 * A lease written by the old build is read once and carried over, so an open
 * terminal survives this change instead of being abandoned.
 */
function readLease(prefix: string, tabId: string): Lease | null {
  try {
    const key = prefix + tabId
    const raw = localStorage.getItem(key) ?? sessionStorage.getItem(key)
    return raw ? (JSON.parse(raw) as Lease) : null
  } catch {
    return null
  }
}

function writeLease(prefix: string, tabId: string, lease: Lease | null): void {
  try {
    const key = prefix + tabId
    sessionStorage.removeItem(key)
    if (lease) localStorage.setItem(key, JSON.stringify(lease))
    else localStorage.removeItem(key)
  } catch {
    // Storage full or blocked — the terminal still works, only the reattach
    // across a reload is lost.
  }
}

interface Held {
  token: string
  /** The row it was opened for, or that claimed it since. */
  key?: string
}

/**
 * Every session this browser holds a token for, by PTY session id: the way
 * back to one no tab shows now. A page with no rail keeps no book.
 */
function readBook(prefix: string): Record<string, Held> {
  try {
    return JSON.parse(localStorage.getItem(`${prefix}book`) ?? '{}') as Record<string, Held>
  } catch {
    return {}
  }
}

/** A null token drops the entry; without a key, the entry keeps the one it has. */
function writeBook(prefix: string, sessionId: string, token: string | null, key?: string): void {
  try {
    const book = readBook(prefix)
    if (token) book[sessionId] = { token, key: key ?? book[sessionId]?.key }
    else delete book[sessionId]
    localStorage.setItem(`${prefix}book`, JSON.stringify(book))
  } catch {
    // As for the lease: only the way back is lost.
  }
}

function heldFor(prefix: string, key: string): string | undefined {
  return Object.entries(readBook(prefix)).find(([, held]) => held.key === key)?.[0]
}

interface SessionSummary {
  session_id: string
  program: string | null
  status: string | { exited?: unknown }
  /** Where output goes; null while detached. */
  output_function_id: string | null
  ui: string | null
}

/**
 * The session this page left behind, if there is one.
 *
 * A lost lease strands a running agent: the program keeps working in the
 * workspace and no page can reach it. `shell::pty::sessions` is the only view
 * of what is actually running, and the worker reports which console page each
 * session belongs to — never a browser id, so this recognises "a claude
 * terminal" rather than "my claude terminal". The program has to match too,
 * because one page family can run a session that is no longer what this
 * worker would start.
 *
 * Only an unattached session qualifies: a terminal someone is watching is not
 * an orphan, and the worker refuses to hand it over anyway.
 */
async function findOrphan(
  call: <T>(fn: string, payload: unknown) => Promise<T>,
  program: string,
  ui: string,
): Promise<string | null> {
  try {
    const { sessions } = await call<{ sessions: SessionSummary[] }>('shell::pty::sessions', {})
    // `ui` comes without its `-ui` suffix, so nothing matches. Leave it: with a rail, detached is
    // normal, and adopting one at random would steal it from the tab that holds it.
    const orphan = sessions.find(
      (session) => session.ui === ui && session.status === 'detached' && session.program === program,
    )
    return orphan?.session_id ?? null
  } catch {
    // An older shell worker has no `sessions`/`adopt`; a fresh session is
    // then the only behaviour available, which is what this page did before.
    return null
  }
}

/**
 * The bus rejects with plain objects as well as Errors (`{ code, message }`
 * from the engine, with the worker's text nested inside `message`), so the
 * whole value is searched rather than just `.message`: reading a stale
 * lease's failure as "unknown" shows an error pane where a fresh session is
 * the answer.
 */
function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  try {
    return JSON.stringify(error) ?? String(error)
  } catch {
    return String(error)
  }
}

function sessionGone(error: unknown): boolean {
  const message = `${errorText(error)} ${String(error)}`
  return (
    message.includes('terminal session does not exist') ||
    message.includes('terminal session is closed') ||
    message.includes('terminal reconnect token is invalid') ||
    message.includes('terminal session credentials are invalid')
  )
}

/**
 * Take a session over before the page lets go of the one on screen, so a
 * refusal leaves that one untouched: by its token, else by adoption. Null when
 * there is nothing to show. The worker keeps listing an ended session and
 * still attaches to it, so only the attach tells.
 */
async function takeSession(
  call: <T>(fn: string, payload: unknown) => Promise<T>,
  listed: SessionSummary,
  view: { output_function_id: string; cols: number; rows: number },
  token: string | undefined,
  /** Whether a live row named it, rather than the book. */
  named: boolean,
): Promise<AttachResponse | null> {
  // No replay: the connect that follows asks for the whole buffer.
  const request = { ...view, session_id: listed.session_id, after_sequence: Number.MAX_SAFE_INTEGER }
  let taken: AttachResponse | null = null
  if (token) {
    taken = await call<AttachResponse>('shell::pty::attach', { ...request, reconnect_token: token }).catch((error) => {
      if (!sessionGone(error)) throw error
      return null
    })
  }
  if (!taken) {
    if (listed.status === 'attached') {
      // An ended session still reads attached: without a live row, the worker's describe decides.
      if (!named) return null
      throw new Error(OPEN_ELSEWHERE)
    }
    taken = await call<AttachResponse>('shell::pty::adopt', request)
  }
  if (typeof taken.status !== 'object') return taken
  // Ended: it keeps one of the shell worker's 16 slots until someone closes it.
  void call('shell::pty::close', { session_id: listed.session_id, access_key: taken.access_key }).catch(() => undefined)
  return null
}

/**
 * Whether a terminal shown from the rail may take the keyboard: not under the
 * rail's open drawer, where Escape closes the drawer and here would reach the
 * agent, and not from a field the operator is typing in.
 */
function mayFocus(container: HTMLElement | null): boolean {
  if (container?.closest('main')?.parentElement?.querySelector(':scope > [data-drawer="open"]')) return false
  const active = document.activeElement
  return !active?.matches('input, textarea, select, [contenteditable]') || !!container?.contains(active)
}

function frameToEvent(sessionId: string, frame: { sequence: number; data: string }): PtyOutputEvent {
  return {
    session_id: sessionId,
    sequence: frame.sequence,
    data: frame.data,
    eof: false,
    exit_code: null,
    signal: null,
    error: null,
  }
}

/**
 * One palette, dark, whatever the console theme is.
 *
 * The terminal is not the page: an agent CLI paints its own interface with
 * ANSI colors chosen for a dark terminal, and it never learns that the
 * console around it went light. Following the console theme produced exactly
 * that mismatch — Claude Code's dim gray on white, unreadable, next to pi's
 * own dark background in the pane beside it. A terminal emulator is allowed
 * to be dark inside a light application; every one of them is.
 */
const TERMINAL_THEME = {
  background: '#1a1b1e',
  foreground: '#e6e6e6',
  cursor: '#e6e6e6',
}

function AgentTerminal({
  host,
  router,
  tabId,
  options,
  leasePrefix,
  openRef,
  onCurrent,
}: {
  host: Host
  router: OutputRouter
  tabId: string
  options: AgentTerminalOptions
  leasePrefix: string
  openRef: Ref<AgentTerminalApi['open']>
  onCurrent: (sessionId: string | null) => void
}) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [status, setStatus] = useState<'connecting' | 'running' | 'exited' | 'error'>('connecting')
  const [detail, setDetail] = useState('')
  const [generation, setGeneration] = useState(0)
  const [auth, setAuth] = useState<AuthStatus | null>(null)
  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  // What the next connect serves in place of the tab's own session.
  const requestRef = useRef<{ key: string; lease: Lease } | null>(null)
  const connectRef = useRef<Promise<void> | null>(null)
  // `current` as it is now, not as it was when the rail last rendered.
  const currentRef = useRef<string | null>(null)
  const [fontSize, setFontSize] = useTerminalFontSize()
  // Read inside the session effect, never a dependency of it: resizing the
  // type must not tear down a live agent.
  const fontSizeRef = useRef(fontSize)
  fontSizeRef.current = fontSize

  // A bigger font means fewer columns, so the PTY has to hear about it — the
  // resize goes out through xterm's onResize, which the session already
  // forwards to `shell::pty::resize`.
  useEffect(() => {
    const term = termRef.current
    if (!term) return
    term.options.fontSize = fontSize
    try {
      fitRef.current?.fit()
    } catch {
      // A pane mid-layout measures as a sliver; the ResizeObserver refits.
    }
  }, [fontSize])

  // Which plan a session spends is not a question anyone should answer by
  // reading a config file, so the page asks the worker and shows it. Re-asked
  // on restart, because a login inside the terminal changes the answer.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-ask on restart
  useEffect(() => {
    let disposed = false
    void host.iii
      .call(`${options.worker}::auth::status`, {}, { timeoutMs: CALL_TIMEOUT_MS })
      .then((status: AuthStatus) => {
        if (!disposed) setAuth(status)
      })
      .catch(() => {
        // An older worker without the function, or a host that cannot answer:
        // the terminal is the point, the badge is not.
        if (!disposed) setAuth(null)
      })
    return () => {
      disposed = true
    }
  }, [host, options.worker, generation])

  // `generation` is not read in the effect, it IS the restart signal:
  // bumping it tears this session down and opens a fresh one.
  // biome-ignore lint/correctness/useExhaustiveDependencies: restart signal
  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    let disposed = false
    let recovering = false
    let unsubscribe: (() => void) | null = null
    let conn: { sessionId: string; accessKey: string } | null = null
    // The token this terminal was last given: storage is shared with the other windows.
    let heldToken = ''

    const term = new Terminal({
      fontSize: fontSizeRef.current,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
      theme: TERMINAL_THEME,
      scrollback: 10_000,
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(container)
    termRef.current = term
    fitRef.current = fit

    const call = <T,>(fn: string, payload: unknown): Promise<T> =>
      host.iii.call(fn, payload, { timeoutMs: CALL_TIMEOUT_MS })
    // Detach, never close: the agent keeps running for the next mount.
    const detach = (session: { sessionId: string; accessKey: string }) =>
      void call('shell::pty::detach', {
        session_id: session.sessionId,
        access_key: session.accessKey,
      }).catch(() => undefined)

    const show = (sessionId: string | null) => {
      if (disposed) return
      currentRef.current = sessionId
      onCurrent(sessionId)
    }
    const book = (sessionId: string, token: string | null, key?: string) => {
      if (options.sidebar) writeBook(leasePrefix, sessionId, token, key)
    }
    const hold = (sessionId: string, reconnectToken: string, key?: string) => {
      heldToken = reconnectToken
      writeLease(leasePrefix, tabId, { sessionId, reconnectToken })
      book(sessionId, reconnectToken, key)
      show(sessionId)
    }
    const release = (sessionId: string) => {
      // Another window on this tab may have left it another session.
      if (readLease(leasePrefix, tabId)?.sessionId === sessionId) writeLease(leasePrefix, tabId, null)
      show(null)
    }

    // Shown next to the status while a session runs: "running · 12 frames,
    // 4.2 kB". A blank terminal with no frames is a delivery problem; a blank
    // one with frames counted is a rendering problem. Throttled — output
    // arrives far faster than a status line needs to change.
    let seen = 0
    let applied = 0
    let progressTimer: number | null = null
    const showProgress = () => {
      // Trailing edge, so the line always ends on the true count: a burst of
      // frames writes one update later, not one per frame.
      if (progressTimer !== null) return
      progressTimer = window.setTimeout(() => {
        progressTimer = null
        if (!disposed) setDetail(`${seen} frames, ${(applied / 1000).toFixed(1)} kB`)
      }, 250)
    }

    const writer = createOrderedWriter({
      write: (bytes) => {
        applied += bytes.length
        term.write(bytes)
      },
      onApplied: (sequence) => {
        seen = sequence
        showProgress()
      },
      onEof: (event) => {
        // The last frame's progress line must not land after the exit message.
        if (progressTimer !== null) window.clearTimeout(progressTimer)
        progressTimer = null
        // It keeps one of the shell worker's 16 slots until someone closes it.
        if (conn && options.sidebar) {
          void call('shell::pty::close', {
            session_id: conn.sessionId,
            access_key: conn.accessKey,
          }).catch(() => undefined)
        }
        conn = null
        book(event.session_id, null)
        release(event.session_id)
        setStatus('exited')
        setDetail(
          event.exit_code === null || event.exit_code === 0
            ? `${options.title} exited`
            : `${options.title} exited with code ${event.exit_code}`,
        )
      },
      onStall: () => void recover(),
    })

    // A frame never arrived (fire-and-forget delivery can drop): re-attach
    // and replay everything after the last applied sequence from the shell
    // worker's ring buffer.
    const recover = async () => {
      if (disposed || recovering || !conn) return
      const { sessionId } = conn
      recovering = true
      try {
        const lease = readLease(leasePrefix, tabId)
        const attached = await call<AttachResponse>('shell::pty::attach', {
          session_id: conn.sessionId,
          // Not the book's: that one may be another window's, and a stall must not take it.
          reconnect_token: lease?.sessionId === sessionId ? lease.reconnectToken : heldToken,
          output_function_id: router.outputFunctionId,
          after_sequence: writer.lastSeq(),
          cols: term.cols,
          rows: term.rows,
        })
        if (disposed) {
          // The cleanup detached with the key this attach replaced: keep the new token, let go again.
          book(sessionId, attached.reconnect_token)
          detach({ sessionId, accessKey: attached.access_key })
          return
        }
        conn.accessKey = attached.access_key
        hold(conn.sessionId, attached.reconnect_token)
        if (attached.truncated) {
          // The missing range is already off the end of the ring buffer, so
          // it will never arrive. Skipping to the newest sequence keeps the
          // writer from waiting on a gap that cannot close — the stall that
          // reads as a terminal ignoring the keyboard.
          const newest = attached.frames.at(-1)?.sequence ?? writer.lastSeq()
          writer.base(newest)
          await askForRepaint(conn)
          return
        }
        for (const frame of attached.frames) {
          writer.feed(frameToEvent(conn.sessionId, frame))
        }
      } catch (error) {
        if (disposed) return
        if (sessionGone(error)) {
          conn = null
          // Another window's token is still the way back to it.
          if (readBook(leasePrefix)[sessionId]?.token === heldToken) book(sessionId, null)
          release(sessionId)
          setStatus('exited')
          setDetail(`${options.title} session lost`)
        }
      } finally {
        recovering = false
      }
    }

    const handleEvent = (event: PtyOutputEvent) => {
      if (disposed) return
      writer.feed(event)
    }

    /**
     * Ask the agent to paint the screen it is on right now.
     *
     * A one-row resize and back is a SIGWINCH, which every full-screen TUI
     * answers with a full redraw. It is the only way to recover a picture the
     * page cannot reconstruct: the ring buffer is finite and an agent repaints
     * constantly, so a tab left alone for an hour comes back to a replay that
     * begins mid-frame. Feeding those bytes paints wreckage; asking for a
     * repaint paints the truth.
     */
    const askForRepaint = async (session: { sessionId: string; accessKey: string }) => {
      const { cols, rows } = term
      if (rows < 2) return
      try {
        await call('shell::pty::resize', {
          session_id: session.sessionId,
          access_key: session.accessKey,
          cols,
          rows: rows - 1,
        })
        await call('shell::pty::resize', {
          session_id: session.sessionId,
          access_key: session.accessKey,
          cols,
          rows,
        })
      } catch {
        // A session that will not resize is a session that is gone; the next
        // frame (or its absence) is the honest signal, not this.
      }
    }

    const connect = async () => {
      setStatus('connecting')
      setDetail('')
      show(null)
      // Let layout settle so the first fit measures the real pane — a session
      // opened at the default 80x24 and resized a beat later leaves redraw
      // artifacts in a TUI.
      await new Promise((resolve) => requestAnimationFrame(resolve))
      if (disposed) return
      // Served once: a restart or a remount is the tab's own session again.
      const request = requestRef.current
      requestRef.current = null
      fit.fit()
      const cols = term.cols
      const rows = term.rows
      const lease = request?.lease ?? readLease(leasePrefix, tabId)
      try {
        if (lease) {
          // A remount builds a NEW xterm with an empty screen, so the whole
          // buffer has to replay (after_sequence 0), not the tail after what
          // the previous terminal applied: the agent repaints only on change,
          // so a tail replay leaves a blank pane until the next keystroke.
          // Only the mid-session stall recovery, where the same terminal is
          // still on screen, replays from the last applied sequence.
          writer.reset(0)
          unsubscribe = router.subscribe(lease.sessionId, handleEvent)
          for (const event of router.drain(lease.sessionId)) writer.feed(event)
          try {
            const attached = await call<AttachResponse>('shell::pty::attach', {
              session_id: lease.sessionId,
              // Another tab may have shown it since, which rotated the token.
              reconnect_token: readBook(leasePrefix)[lease.sessionId]?.token ?? lease.reconnectToken,
              output_function_id: router.outputFunctionId,
              after_sequence: 0,
              cols,
              rows,
            })
            conn = { sessionId: lease.sessionId, accessKey: attached.access_key }
            hold(lease.sessionId, attached.reconnect_token, request?.key)
            if (disposed) {
              detach(conn)
              return
            }
            // It ended with no viewer, and the worker sends an EOF only to one.
            const ended = options.sidebar && typeof attached.status === 'object'
            if (attached.truncated) {
              // Partial history is worse than none: it lands inside whatever
              // the agent was drawing. Start from the newest sequence the
              // worker still holds and ask for the current screen instead.
              const newest = attached.frames.at(-1)?.sequence ?? 0
              term.reset()
              writer.base(newest)
            } else {
              for (const frame of attached.frames) {
                writer.feed(frameToEvent(lease.sessionId, frame))
              }
            }
            setStatus('running')
            if (ended) {
              const last = frameToEvent(lease.sessionId, { sequence: 0, data: '' })
              writer.feed({ ...last, sequence: null, data: null, eof: true })
            } else if (attached.truncated) {
              await askForRepaint(conn)
            }
            return
          } catch (error) {
            unsubscribe()
            unsubscribe = null
            if (sessionGone(error)) book(lease.sessionId, null)
            // Asked for by name: a fresh agent is not the answer, and the
            // lease still names what this tab showed before.
            if (request) throw error
            writeLease(leasePrefix, tabId, null)
            if (!sessionGone(error)) throw error
            // fall through to a fresh session
          }
        }

        // The worker owns the command: the page asks what to run and never
        // decides. A worker that cannot answer has no terminal to give.
        const spec = await call<TerminalSpec>(`${options.worker}::terminal::describe`, {})

        // Before starting a second agent, look for the first one. A lease can
        // be lost — cleared storage, a different browser, a token that went
        // stale — while the program keeps running in the workspace, and the
        // page has no other way to notice. `shell::pty::adopt` takes back an
        // unattached session that belongs to this page.
        const orphan = await findOrphan(call, spec.program, `${options.worker}-ui`)
        if (orphan) {
          unsubscribe = router.subscribe(orphan, handleEvent)
          try {
            const adopted = await call<AttachResponse>('shell::pty::adopt', {
              session_id: orphan,
              output_function_id: router.outputFunctionId,
              cols,
              rows,
              after_sequence: 0,
            })
            conn = { sessionId: orphan, accessKey: adopted.access_key }
            hold(orphan, adopted.reconnect_token)
            term.reset()
            if (adopted.truncated) {
              writer.reset(adopted.frames.at(-1)?.sequence ?? 0)
            } else {
              writer.reset(0)
              for (const frame of adopted.frames) writer.feed(frameToEvent(orphan, frame))
            }
            setStatus('running')
            await askForRepaint(conn)
            return
          } catch {
            // Someone attached first, or the session ended between the two
            // calls. A fresh one is the honest answer.
            unsubscribe()
            unsubscribe = null
          }
        }

        writer.reset(0)
        const opened = await call<OpenResponse>('shell::pty::open', {
          cwd: spec.cwd,
          cols,
          rows,
          output_function_id: router.outputFunctionId,
          program: spec.program,
          args: spec.args ?? [],
          env: spec.env ?? {},
        })
        if (disposed) {
          // It runs with nobody watching: the book is the way back to it.
          book(opened.session_id, opened.reconnect_token)
          detach({ sessionId: opened.session_id, accessKey: opened.access_key })
          return
        }
        conn = { sessionId: opened.session_id, accessKey: opened.access_key }
        unsubscribe = router.subscribe(opened.session_id, handleEvent)
        hold(opened.session_id, opened.reconnect_token)
        for (const event of router.drain(opened.session_id)) writer.feed(event)
        setStatus('running')
      } catch (error) {
        if (disposed) return
        setStatus('error')
        setDetail(errorText(error))
      }
    }

    const send = (fn: 'write' | 'resize', payload: object) => {
      if (!conn) return
      const { sessionId, accessKey } = conn
      call(`shell::pty::${fn}`, { session_id: sessionId, access_key: accessKey, ...payload }).catch((error) => {
        // A refused key: another window took the session, so its lease and its token are that
        // window's now. A recovery replaces the key too, and that one is ours.
        if (!options.sidebar || disposed || recovering || conn?.accessKey !== accessKey) return
        if (!errorText(error).includes('terminal session credentials are invalid')) return
        conn = null
        show(null)
        setStatus('error')
        setDetail(OPEN_ELSEWHERE)
      })
    }
    const onData = term.onData((data) => send('write', { data: encodeB64(data) }))
    const onResize = term.onResize(({ cols, rows }) => send('resize', { cols, rows }))
    const observer = new ResizeObserver(() => {
      try {
        fit.fit()
      } catch {
        // container mid-teardown
      }
    })
    observer.observe(container)

    // Closing the window runs no cleanup, and the worker never notices that its viewer is gone:
    // the session would read attached, and no other browser could show it. A page kept for the
    // back button keeps its terminal.
    const onPageHide = (event: PageTransitionEvent) => {
      if (conn && !event.persisted) detach(conn)
    }
    if (options.sidebar) window.addEventListener('pagehide', onPageHide)

    connectRef.current = connect()
    if (!requestRef.current || mayFocus(container)) term.focus()

    return () => {
      disposed = true
      window.removeEventListener('pagehide', onPageHide)
      observer.disconnect()
      onData.dispose()
      onResize.dispose()
      unsubscribe?.()
      if (progressTimer !== null) window.clearTimeout(progressTimer)
      writer.dispose()
      if (conn) detach(conn)
      term.dispose()
      termRef.current = null
      fitRef.current = null
    }
  }, [host, router, tabId, generation, options, leasePrefix])

  const restart = useCallback(() => {
    writeLease(leasePrefix, tabId, null)
    setGeneration((n) => n + 1)
  }, [leasePrefix, tabId])

  const open: AgentTerminalApi['open'] = async ({ key, pty, spec }) => {
    // A connect cut short would leave behind the session it is about to get.
    await connectRef.current
    const call = <T,>(fn: string, payload: unknown): Promise<T> =>
      host.iii.call(fn, payload, { timeoutMs: CALL_TIMEOUT_MS })
    const view = {
      output_function_id: router.outputFunctionId,
      cols: termRef.current?.cols ?? 80,
      rows: termRef.current?.rows ?? 24,
    }
    // Held before `spec`: a second click must not run the same session twice.
    let sessionId = pty ?? heldFor(leasePrefix, key)
    let taken: Pick<OpenResponse, 'access_key' | 'reconnect_token'> | null = null
    if (sessionId) {
      const { sessions } = await call<{ sessions: SessionSummary[] }>('shell::pty::sessions', {})
      const listed = sessions.find((session) => session.session_id === sessionId)
      // Not by `current` alone: another window may have taken it, and nothing told this one.
      const attachedHere = listed?.status === 'attached' && listed.output_function_id === view.output_function_id
      if (attachedHere && sessionId === currentRef.current) {
        if (mayFocus(containerRef.current)) termRef.current?.focus()
        return true
      }
      if (listed) taken = await takeSession(call, listed, view, readBook(leasePrefix)[sessionId]?.token, !!pty)
      writeBook(leasePrefix, sessionId, taken?.reconnect_token ?? null, key)
    }
    if (!taken && spec) {
      const opened = await call<OpenResponse>('shell::pty::open', {
        ...view,
        cwd: spec.cwd,
        program: spec.program,
        args: spec.args ?? [],
        env: spec.env ?? {},
      })
      sessionId = opened.session_id
      taken = opened
      writeBook(leasePrefix, sessionId, taken.reconnect_token, key)
    }
    if (!sessionId || !taken) return false
    if (!termRef.current) {
      // Unmounted meanwhile: nothing would show it, or ever detach it.
      void call('shell::pty::detach', { session_id: sessionId, access_key: taken.access_key }).catch(() => undefined)
      return true
    }
    requestRef.current = { key, lease: { sessionId, reconnectToken: taken.reconnect_token } }
    setGeneration((n) => n + 1)
    return true
  }
  useImperativeHandle(openRef, () => open)

  // Ctrl/⌘ + wheel is what every terminal emulator does, and it beats clicking
  // a stepper 20 times to get from 14 to 34.
  //
  // A native listener with `{ passive: false }`, not React's `onWheel`: React
  // registers wheel handlers as passive, so `preventDefault()` inside one is
  // ignored and the browser zooms the whole page underneath the terminal.
  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    const zoom = (event: globalThis.WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return
      event.preventDefault()
      setFontSize(stepFontSize(fontSizeRef.current, event.deltaY < 0 ? 1 : -1))
    }
    container.addEventListener('wheel', zoom, { passive: false })
    return () => container.removeEventListener('wheel', zoom)
  }, [setFontSize])

  return (
    <div className="agent-terminal">
      <div className="agent-terminal-viewport" ref={containerRef} data-autofocus="true" />
      <StatusBar
        className="agent-terminal-strip"
        aria-label={`${options.title} terminal status`}
        end={
          <>
            {auth && (
              <span className="agent-terminal-strip-billing" data-billing={auth.billing} title={auth.detail}>
                <Eyebrow>Billing</Eyebrow>
                <Chip tone={BILLING_TONE[auth.billing] ?? 'neutral'}>
                  <span className="agent-terminal-truncate">{auth.label}</span>
                </Chip>
              </span>
            )}
            <span
              className="agent-terminal-strip-field"
              title={`Terminal font size in pixels (${MIN_FONT_SIZE}–${MAX_FONT_SIZE}). Ctrl or ⌘ + scroll does the same.`}
            >
              <Eyebrow>Font</Eyebrow>
              <ShadcnButton
                variant="ghost"
                size="icon-sm"
                onClick={() => setFontSize(stepFontSize(fontSize, -1))}
                disabled={fontSize <= MIN_FONT_SIZE}
                aria-label="Smaller terminal font"
              >
                <Minus aria-hidden />
              </ShadcnButton>
              <output aria-label="Terminal font size in pixels">{fontSize}</output>
              <ShadcnButton
                variant="ghost"
                size="icon-sm"
                onClick={() => setFontSize(stepFontSize(fontSize, 1))}
                disabled={fontSize >= MAX_FONT_SIZE}
                aria-label="Larger terminal font"
              >
                <Plus aria-hidden />
              </ShadcnButton>
            </span>
            {(status === 'exited' || status === 'error') && (
              <ShadcnButton size="sm" onClick={restart}>
                Restart {options.title}
              </ShadcnButton>
            )}
          </>
        }
      >
        {status === 'error' ? (
          <span className="agent-terminal-truncate agent-terminal-strip-error" title={detail}>
            {detail}
          </span>
        ) : (
          <span className="agent-terminal-truncate">
            {status === 'exited' ? detail : detail ? `${status} · ${detail}` : status}
          </span>
        )}
      </StatusBar>
    </div>
  )
}

/**
 * Which plan the session spends. Accent is reserved for actions, so the
 * subscription case stays neutral and only the two cases an operator might
 * not expect — a metered key, or no credentials at all — carry a colour.
 */
const BILLING_TONE: Record<AuthStatus['billing'], ChipTone> = {
  subscription: 'neutral',
  unknown: 'neutral',
  'api-key': 'warning',
  none: 'danger',
}

/** The page: the terminal, and the rail that reads what is on it and asks it for another session. */
function AgentTerminalPage({
  host,
  router,
  tabId,
  options,
  leasePrefix,
  onClose,
}: {
  host: Host
  router: OutputRouter
  tabId: string
  options: AgentTerminalOptions
  leasePrefix: string
  onClose?: () => void
}) {
  const [current, setCurrent] = useState<string | null>(null)
  const openRef = useRef<AgentTerminalApi['open']>(null)
  const { sidebar } = options
  const terminal = (
    <PageMain>
      <AgentTerminal
        host={host}
        router={router}
        tabId={tabId}
        options={options}
        leasePrefix={leasePrefix}
        openRef={openRef}
        onCurrent={setCurrent}
      />
    </PageMain>
  )
  return (
    <PageShell>
      <PageHeader title={options.title} description={options.description} onClose={onClose} />
      {sidebar ? (
        <PageBody side="right">
          <PageSidebar
            label={sidebar.label}
            side="right"
            collapsible
            resizable
            storageKey={sidebar.storageKey}
            defaultWidth={300}
            minWidth={240}
            maxWidth={440}
            narrowBelow={960}
            narrowMode="drawer"
            closeDrawerOnSelect={false}
          >
            {sidebar.render(host, {
              current,
              currentKey: (current && readBook(leasePrefix)[current]?.key) || null,
              has: (key) => heldFor(leasePrefix, key) !== undefined,
              claim: (pty, key) => {
                const held = readBook(leasePrefix)[pty]
                if (held && held.key !== key) writeBook(leasePrefix, pty, held.token, key)
              },
              open: async (target) => (await openRef.current?.(target)) ?? false,
            })}
          </PageSidebar>
          {terminal}
        </PageBody>
      ) : (
        <PageBody>{terminal}</PageBody>
      )}
    </PageShell>
  )
}

/**
 * The `setup(host)` an agent-CLI worker's page asset exports. One page, one
 * output handler for the worker, and a terminal per tab.
 */
export function createAgentTerminalPage(options: AgentTerminalOptions) {
  const handler = `iii::${options.worker}-ui::pty-output`
  const leasePrefix = `iii::${options.worker}-ui::lease::`

  return function setup(host: Host) {
    const router = createRouter(host, handler)
    host.pages.register({
      id: options.worker,
      title: options.title,
      render: (props: { tabId?: string; onRequestClose?: () => void }) => (
        <AgentTerminalPage
          host={host}
          router={router}
          tabId={props.tabId || 'default'}
          options={options}
          leasePrefix={leasePrefix}
          onClose={props.onRequestClose}
        />
      ),
    })
    return () => router.dispose()
  }
}
