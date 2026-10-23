/**
 * The page's one terminal: a real tmux client (`tmux attach`) the worker
 * runs in a PTY, painted by xterm here.
 *
 * - The client is disposable. Durable state lives in tmux, so a remount,
 *   a reload or a transport that comes back simply opens a new client and
 *   tmux repaints the whole screen on attach — no lease to keep.
 * - Moving between sessions moves this client only (`tmux::terminal::switch`,
 *   tmux's `switch-client`); choosing a window is the page's business
 *   (`tmux::windows::select`), which every client of that session follows.
 * - Output arrives unordered: an ordered writer applies contiguous frames,
 *   replays a gap that outlives 250 ms, and asks tmux to redraw when the gap
 *   is already gone from the worker's ring buffer.
 * - `resize` doubles as the heartbeat: the worker closes a client that has
 *   not been heard from for 30 s, so one goes out every 10 s while mounted.
 */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { readFontSize, stepFontSize, useTerminalFontSize, writeFontSize } from '@iii-workers/terminal-font'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { TerminalCredentials, TmuxApi } from '../lib/api'
import { binaryStringToBytes, createInputWriter, decodeBase64, encodeBase64 } from '../lib/input-writer'
import { terminalGone } from '../lib/model'
import type { OutputRouter } from '../lib/output-router'
import { createOrderedWriter, type OrderedWriter } from '../lib/terminal-stream'
import type { TerminalOutputEvent } from '../lib/types'

type XtermModule = typeof import('../../xterm')
type Terminal = InstanceType<XtermModule['Terminal']>

export type TerminalStatus = 'loading' | 'idle' | 'connecting' | 'ready' | 'ended' | 'error'

export interface TerminalTarget {
  session: string
  window?: string
}

export interface TmuxTerminal {
  status: TerminalStatus
  /** The session this client is on, as the worker last reported it. */
  sessionId: string | null
  cols: number
  rows: number
  /** Why the terminal ended or failed. */
  detail: string | null
  setContainer(node: HTMLDivElement | null): void
  focus(): void
  /**
   * Put this client on `target`: `switch` while it runs, a fresh `open`
   * when it has ended.
   */
  moveTo(target: TerminalTarget): void
  /** Open a fresh client on `target` (or where the last one was). */
  reconnect(target?: TerminalTarget): void
}

const RESIZE_DEBOUNCE_MS = 60
const HEARTBEAT_MS = 10_000

/**
 * One palette, dark, whatever the console theme is: tmux and the agent
 * CLIs inside it paint with ANSI colors chosen for a dark terminal, and a
 * terminal emulator is allowed to be dark inside a light application.
 */
const TERMINAL_THEME = {
  background: '#141414',
  foreground: '#e6e6e6',
  cursor: '#e6e6e6',
  cursorAccent: '#141414',
  selectionBackground: '#3a3d41',
  black: '#484f58',
  red: '#ff7b72',
  green: '#3fb950',
  yellow: '#d29922',
  blue: '#58a6ff',
  magenta: '#bc8cff',
  cyan: '#39c5cf',
  white: '#b1bac4',
  brightBlack: '#6e7681',
  brightRed: '#ffa198',
  brightGreen: '#56d364',
  brightYellow: '#e3b341',
  brightBlue: '#79c0ff',
  brightMagenta: '#d2a8ff',
  brightCyan: '#56d4dd',
  brightWhite: '#f0f6fc',
}

let xtermLoad: Promise<XtermModule> | null = null

/**
 * The emulator ships as its own console:module (ui/xterm.ts) so a tab that
 * never opens this page never downloads it. A failed import clears the memo
 * so the next attempt imports again.
 */
function loadXterm(host: Host): Promise<XtermModule> {
  if (!host.importModule) {
    return Promise.reject(new Error('this console predates lazy modules; update the ade worker'))
  }
  xtermLoad ??= host.importModule<XtermModule>('tmux/xterm.js').catch((error: unknown) => {
    xtermLoad = null
    throw error
  })
  return xtermLoad
}

/** A pane mid-layout or inside a hidden tab measures as a sliver or nothing. */
function measurable(container: Element): boolean {
  const rect = container.getBoundingClientRect()
  return rect.width >= 40 && rect.height >= 24
}

export function useTmuxTerminal({
  host,
  api,
  router,
  target,
}: {
  host: Host
  api: TmuxApi
  router: OutputRouter
  /** Where the first client opens; later moves go through `moveTo`. */
  target: TerminalTarget | null
}): TmuxTerminal {
  const [container, setContainer] = useState<HTMLDivElement | null>(null)
  const [status, setStatus] = useState<TerminalStatus>('loading')
  const [detail, setDetail] = useState<string | null>(null)
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [size, setSize] = useState({ cols: 0, rows: 0 })
  const [termReady, setTermReady] = useState(false)
  const [generation, setGeneration] = useState(0)
  const [fontSize] = useTerminalFontSize()

  const termRef = useRef<Terminal | null>(null)
  const fitRef = useRef<(() => void) | null>(null)
  const connRef = useRef<TerminalCredentials | null>(null)
  const writerRef = useRef<OrderedWriter | null>(null)
  const inputRef = useRef<((data: Uint8Array) => void) | null>(null)
  const targetRef = useRef<TerminalTarget | null>(target)
  const fontSizeRef = useRef(fontSize)
  fontSizeRef.current = fontSize
  const hasTarget = target !== null
  if (targetRef.current === null && target !== null) targetRef.current = target

  const markEnded = useCallback((message: string) => {
    connRef.current = null
    inputRef.current = null
    setStatus('ended')
    setDetail(message)
  }, [])

  // ── the emulator ────────────────────────────────────────────────────
  useEffect(() => {
    if (!container) return
    let cancelled = false
    let retryTimer = 0
    let cleanup: (() => void) | null = null
    setStatus('loading')
    const load = (attempt: number): Promise<XtermModule> =>
      loadXterm(host).catch((error: unknown) => {
        if (cancelled || !host.importModule) throw error
        // A module still registering 404s at first: retry, backing off.
        setDetail(`Terminal failed to load: ${errorMessage(error)}`)
        return new Promise<XtermModule>((resolve) => {
          retryTimer = window.setTimeout(() => resolve(load(attempt + 1)), Math.min(30_000, 1_000 * 2 ** attempt))
        })
      })
    void load(0).then(
      ({ Terminal, FitAddon }) => {
        if (cancelled) return
        setDetail(null)
        const styles = window.getComputedStyle(container)
        const fontFamily =
          styles.getPropertyValue('--font-code').trim() || 'ui-monospace, SFMono-Regular, Menlo, monospace'
        const terminal = new Terminal({
          cursorBlink: true,
          fontFamily,
          fontSize: fontSizeRef.current,
          lineHeight: 1.15,
          scrollback: 2_000,
          theme: TERMINAL_THEME,
          allowProposedApi: false,
        })
        const fit = new FitAddon()
        terminal.loadAddon(fit)
        const hostNode = document.createElement('div')
        hostNode.className = 'tmux-xterm-host'
        container.appendChild(hostNode)
        terminal.open(hostNode)
        termRef.current = terminal

        const send = (bytes: Uint8Array) => inputRef.current?.(bytes)
        const onData = terminal.onData((data) => send(new TextEncoder().encode(data)))
        const onBinary = terminal.onBinary((data) => send(binaryStringToBytes(data)))

        let resizeTimer = 0
        const onResize = terminal.onResize(({ cols, rows }) => {
          setSize({ cols, rows })
          window.clearTimeout(resizeTimer)
          resizeTimer = window.setTimeout(() => {
            const conn = connRef.current
            if (!conn) return
            api.resizeTerminal(conn, cols, rows).catch((error: unknown) => {
              if (terminalGone(error) && connRef.current === conn) markEnded('The tmux client is gone.')
            })
          }, RESIZE_DEBOUNCE_MS)
        })

        let fitFrame = 0
        const fitNow = () => {
          if (!measurable(container)) return
          try {
            fit.fit()
          } catch {
            // Mid-teardown; the next observation fits again.
          }
        }
        const scheduleFit = () => {
          if (fitFrame) window.cancelAnimationFrame(fitFrame)
          fitFrame = window.requestAnimationFrame(() => {
            fitFrame = 0
            fitNow()
          })
        }
        fitRef.current = fitNow
        const observer = new ResizeObserver(scheduleFit)
        observer.observe(container)

        // Ctrl/⌘ + scroll is the gesture every terminal emulator answers to;
        // a native non-passive listener, since React's wheel handlers are
        // passive and the page would zoom underneath.
        const zoom = (event: WheelEvent) => {
          if (!event.ctrlKey && !event.metaKey) return
          event.preventDefault()
          writeFontSize(stepFontSize(readFontSize(), event.deltaY < 0 ? 1 : -1))
        }
        container.addEventListener('wheel', zoom, { passive: false })

        fitNow()
        setSize({ cols: terminal.cols, rows: terminal.rows })
        setTermReady(true)

        cleanup = () => {
          window.clearTimeout(resizeTimer)
          if (fitFrame) window.cancelAnimationFrame(fitFrame)
          container.removeEventListener('wheel', zoom)
          observer.disconnect()
          onResize.dispose()
          onBinary.dispose()
          onData.dispose()
          terminal.dispose()
          hostNode.remove()
          if (termRef.current === terminal) termRef.current = null
          if (fitRef.current === fitNow) fitRef.current = null
        }
      },
      () => undefined,
    )
    return () => {
      cancelled = true
      window.clearTimeout(retryTimer)
      setTermReady(false)
      cleanup?.()
    }
  }, [api, container, host, markEnded])

  // A new size goes to the emulator; fitting changes cols/rows, which the
  // resize handler forwards to the tmux client.
  useEffect(() => {
    const terminal = termRef.current
    if (!terminal) return
    terminal.options.fontSize = fontSize
    fitRef.current?.()
  }, [fontSize])

  // ── the tmux client ─────────────────────────────────────────────────
  // `generation` is the reconnect signal: bumping it closes this client and
  // opens a fresh one where `targetRef` points.
  // biome-ignore lint/correctness/useExhaustiveDependencies: generation is the reconnect signal
  useEffect(() => {
    const terminal = termRef.current
    if (!termReady || !terminal || !hasTarget) {
      if (termReady && !hasTarget) {
        // Nothing to attach to (no sessions); the next target starts afresh.
        targetRef.current = null
        setStatus('idle')
      }
      return
    }
    let disposed = false
    let unsubscribe: (() => void) | null = null
    let recovering = false
    let opened: TerminalCredentials | null = null

    const writer = createOrderedWriter({
      write: (data) => terminal.write(decodeBase64(data)),
      onEof: (event: TerminalOutputEvent) => {
        if (disposed) return
        const reason = event.error
          ? event.error
          : event.exit_code === null || event.exit_code === 0
            ? 'The tmux client detached — the session ended or was left.'
            : `The tmux client exited with code ${event.exit_code}.`
        markEnded(reason)
      },
      onStall: () => void recover(),
    })
    writerRef.current = writer

    // A frame never arrived: replay what follows the last painted one, or,
    // when the ring buffer no longer holds it, have tmux repaint the screen.
    const recover = async () => {
      const conn = connRef.current
      if (disposed || recovering || !conn) return
      recovering = true
      try {
        const replay = await api.replayTerminal(conn, writer.lastSequence())
        if (disposed || connRef.current !== conn) return
        if (replay.truncated) {
          writer.skipTo(Math.max(0, replay.next_sequence - 1))
          await api.redrawTerminal(conn)
        } else {
          writer.feedFrames(replay.frames)
        }
      } catch (error) {
        if (!disposed && terminalGone(error) && connRef.current === conn) markEnded('The tmux client is gone.')
      } finally {
        recovering = false
      }
    }

    // Attaching at a guessed size would resize the window for every other
    // client of the session: wait until the pane can be measured (it is not
    // while the Ports tab hides the terminal), then fit and attach.
    let stopWaiting: (() => void) | null = null
    const whenMeasurable = (node: HTMLDivElement) =>
      new Promise<void>((resolve) => {
        if (measurable(node)) {
          resolve()
          return
        }
        const observer = new ResizeObserver(() => {
          if (!measurable(node)) return
          observer.disconnect()
          stopWaiting = null
          resolve()
        })
        observer.observe(node)
        stopWaiting = () => observer.disconnect()
      })

    const connect = async () => {
      const where = targetRef.current
      if (!where || !container) return
      setStatus('connecting')
      setDetail(null)
      terminal.reset()
      await new Promise((resolve) => window.requestAnimationFrame(resolve))
      if (disposed) return
      await whenMeasurable(container)
      if (disposed) return
      fitRef.current?.()
      try {
        const response = await api.openTerminal({
          session: where.session,
          ...(where.window !== undefined ? { window: where.window } : {}),
          cols: Math.max(2, terminal.cols),
          rows: Math.max(2, terminal.rows),
          output_function_id: router.outputFunctionId,
        })
        const conn = { terminalId: response.terminal_id, accessKey: response.access_key }
        if (disposed) {
          router.discard(conn.terminalId)
          void api.closeTerminal(conn).catch(() => undefined)
          return
        }
        opened = conn
        connRef.current = conn
        inputRef.current = createInputWriter(async (bytes) => {
          if (connRef.current !== conn) return
          try {
            await api.writeTerminal(conn, encodeBase64(bytes))
          } catch (error) {
            if (terminalGone(error) && connRef.current === conn) markEnded('The tmux client is gone.')
          }
        })
        unsubscribe = router.subscribe(conn.terminalId, (event) => {
          if (!disposed) writer.feed(event)
        })
        for (const event of router.drain(conn.terminalId)) writer.feed(event)
        setSessionId(response.session_id)
        targetRef.current = { session: response.session_id }
        // A terminal that already ended (eof drained above) keeps that status.
        if (connRef.current !== conn) return
        setStatus('ready')
        // The pane may have settled to another size while `open` was in flight.
        void api.resizeTerminal(conn, terminal.cols, terminal.rows).catch(() => undefined)
      } catch (error) {
        if (disposed) return
        setStatus('error')
        setDetail(errorMessage(error))
      }
    }
    void connect()

    const heartbeat = window.setInterval(() => {
      const conn = connRef.current
      if (!conn || disposed) return
      api.resizeTerminal(conn, terminal.cols, terminal.rows).catch((error: unknown) => {
        if (terminalGone(error) && connRef.current === conn) markEnded('The tmux client is gone.')
      })
    }, HEARTBEAT_MS)

    // Closing the browser tab cannot wait for a reply; the worker reaps a
    // silent client anyway, this only makes it immediate when it can be.
    const onPageHide = () => {
      const conn = connRef.current
      if (conn) void api.closeTerminal(conn).catch(() => undefined)
    }
    window.addEventListener('pagehide', onPageHide)

    return () => {
      disposed = true
      stopWaiting?.()
      window.clearInterval(heartbeat)
      window.removeEventListener('pagehide', onPageHide)
      unsubscribe?.()
      writer.dispose()
      if (writerRef.current === writer) writerRef.current = null
      if (opened) {
        router.discard(opened.terminalId)
        // Detach only: the session keeps running in tmux.
        void api.closeTerminal(opened).catch(() => undefined)
        if (connRef.current === opened) {
          connRef.current = null
          inputRef.current = null
        }
      }
    }
  }, [api, container, generation, hasTarget, markEnded, router, termReady])

  // A transport that drops loses deliveries, and the worker closes a client
  // it cannot deliver to: once it is back, open a fresh one in its place.
  useEffect(() => {
    let interrupted = false
    return host.iii.addConnectionStateListener((state) => {
      if (state === 'connected') {
        if (interrupted) {
          interrupted = false
          setGeneration((value) => value + 1)
        }
        return
      }
      interrupted = true
    })
  }, [host])

  const moveTo = useCallback(
    (where: TerminalTarget) => {
      const conn = connRef.current
      if (!conn) {
        targetRef.current = where
        setGeneration((value) => value + 1)
        return
      }
      api
        .switchTerminal(conn, where.session, where.window)
        .then((response) => {
          if (connRef.current !== conn) return
          setSessionId(response.session_id)
          targetRef.current = { session: response.session_id }
        })
        .catch((error: unknown) => {
          if (connRef.current !== conn) return
          if (terminalGone(error)) {
            // The client is gone; a fresh one opens where it was asked to go.
            targetRef.current = where
            setGeneration((value) => value + 1)
            return
          }
          setDetail(errorMessage(error))
        })
    },
    [api],
  )

  const reconnect = useCallback((where?: TerminalTarget) => {
    if (where) targetRef.current = where
    setGeneration((value) => value + 1)
  }, [])

  const focus = useCallback(() => termRef.current?.focus(), [])

  return {
    status,
    sessionId,
    cols: size.cols,
    rows: size.rows,
    detail,
    setContainer,
    focus,
    moveTo,
    reconnect,
  }
}
