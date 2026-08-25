import type { Host } from '@iii-dev/console-ui'
import { useTerminalFontSize } from '@iii-workers/terminal-font'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from 'react'
import { errorMessage } from '../lib/format'
import {
  backendConfirmedSessionMissing,
  closeTerminalConnection,
  connectTerminalSession,
  disposeTerminalConnection,
  legacyTerminalStorageKey,
  reclaimTerminalLease,
  type TerminalConnection,
} from './terminal-connection'
import {
  type LocalTerminalLease,
  loadRecoverableTerminalLeases,
  removeRecoverableTerminalLease,
  saveRecoverableTerminalLease,
} from './terminal-leases'
import {
  createTerminalOutputRouter,
  type PtyOutputEvent,
  terminalOutputRouterHost,
  type TerminalOutputRouter,
} from './terminal-output-router'
import {
  createTerminalConnectionCoordinator,
  createTerminalSessionState,
  exitedStatus,
  normalizeTerminalDimensions,
  REPLAY_TRUNCATION_NOTICE,
  reduceTerminalLease,
  reduceTerminalSessionState,
  shouldDetachStaleConnection,
  type TerminalConnectionCoordinator,
  type TerminalStatus,
} from './terminal-session-state'
import { terminalAnsiPalette } from './terminal-palette'
import { bufferTerminalFrame, type TerminalFrame } from './terminal-stream'

export type {
  ActivatedTerminalConnection,
  ReclaimableTerminalLease,
  TerminalConnection,
} from './terminal-connection'
export type {
  PtySessionStatus,
  TerminalLeaseAction,
  TerminalSessionAction,
  TerminalSessionState,
  TerminalStatus,
} from './terminal-session-state'
export {
  closeTerminalConnection,
  connectTerminalSession,
  createTerminalConnectionCoordinator,
  createTerminalSessionState,
  disposeTerminalConnection,
  legacyTerminalStorageKey,
  normalizeTerminalDimensions,
  REPLAY_TRUNCATION_NOTICE,
  reclaimTerminalLease,
  reduceTerminalLease,
  reduceTerminalSessionState,
  shouldDetachStaleConnection,
}

export interface TerminalSessionOptions {
  paneId: string
  root: string
  visible: boolean
  router: TerminalOutputRouter | null
  leaseStore: Storage | null
  storageKey?: string
  connectionCoordinator?: TerminalConnectionCoordinator
}

interface LegacyTerminalSessionOptions {
  host: Host
  root: string
  branch: string | null
  jobIds: string[]
  onJobIdsChange: (ids: string[]) => void
}

interface ActiveTerminalSession {
  generation: number
  sessionId: string
  accessKey: string
  reconnectToken: string
  lastSequence: number
  unsubscribe: () => void
}

export type UnmountTerminalSession = Omit<ActiveTerminalSession, 'generation'>

export interface TerminalSession {
  atBottom: boolean
  cwd: string
  error: string | null
  focus: () => void
  jumpToLatest: () => void
  restart: () => void
  startFresh: () => void
  forget: () => void
  close: () => Promise<string | null>
  setContainer: (node: HTMLDivElement | null) => void
  status: TerminalStatus
}

const MAX_QUEUED_INPUT_BYTES = 64 * 1024
const HEARTBEAT_MS = 10_000

function decodeBase64(value: string): Uint8Array {
  const binary = window.atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index)
  }
  return bytes
}

function encodeBase64(bytes: Uint8Array): string {
  let binary = ''
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000))
  }
  return window.btoa(binary)
}

function binaryStringToBytes(value: string): Uint8Array {
  const bytes = new Uint8Array(value.length)
  for (let index = 0; index < value.length; index += 1) {
    bytes[index] = value.charCodeAt(index) & 0xff
  }
  return bytes
}

function browserLocalStorage(): Storage | null {
  if (typeof window === 'undefined') return null
  try {
    return window.localStorage
  } catch {
    return null
  }
}

function findPaneLease(
  storage: Storage | null,
  storageKey: string,
  paneId: string,
): LocalTerminalLease | null {
  return (
    loadRecoverableTerminalLeases(storage, storageKey).find(
      (lease) => lease.paneId === paneId,
    ) ?? null
  )
}

export async function detachTerminalSessionForUnmount(
  host: Host,
  router: TerminalOutputRouter,
  storageKey: string,
  paneId: string,
  active: UnmountTerminalSession,
): Promise<void> {
  saveRecoverableTerminalLease(null, storageKey, {
    paneId,
    sessionId: active.sessionId,
    reconnectToken: active.reconnectToken,
    lastSequence: active.lastSequence,
  })
  active.unsubscribe()
  router.drain(active.sessionId)
  await host.iii.trigger('shell::pty::detach', {
    session_id: active.sessionId,
    access_key: active.accessKey,
  })
}

export function useTerminalSession(
  options: TerminalSessionOptions | LegacyTerminalSessionOptions,
): TerminalSession {
  const legacyHost = 'host' in options ? options.host : null
  const ownedRouter = useMemo(
    () => (legacyHost ? createTerminalOutputRouter(legacyHost) : null),
    [legacyHost],
  )
  useEffect(() => () => ownedRouter?.dispose(), [ownedRouter])

  const paneId = 'host' in options ? 'legacy-terminal' : options.paneId
  const root = options.root
  const visible = 'host' in options ? root.length > 0 : options.visible
  const router = 'host' in options ? ownedRouter : options.router
  const leaseStore =
    'host' in options ? browserLocalStorage() : options.leaseStore
  const storageKey =
    'host' in options
      ? legacyTerminalStorageKey(root)
      : (options.storageKey ?? 'iii::shell-ui::terminal-leases')
  const host = router ? terminalOutputRouterHost(router) : legacyHost
  const [container, setContainer] = useState<HTMLDivElement | null>(null)
  const [state, dispatch] = useReducer(
    reduceTerminalSessionState,
    root,
    createTerminalSessionState,
  )
  const [atBottom, setAtBottom] = useState(true)
  const [restartToken, setRestartToken] = useState(0)
  const terminalRef = useRef<Terminal | null>(null)
  const fitAddonRef = useRef<FitAddon | null>(null)
  // The size every terminal in the console shares, agent pages included. Held
  // in a ref for the mount effect below: changing the type must not rebuild a
  // terminal, which would drop the pane's scrollback.
  const [fontSize] = useTerminalFontSize()
  const fontSizeRef = useRef(fontSize)
  fontSizeRef.current = fontSize
  const terminalCleanupRef = useRef<(() => void) | null>(null)
  const activeRef = useRef<ActiveTerminalSession | null>(null)
  const leaseRef = useRef<LocalTerminalLease | null>(null)
  const dimensionsRef = useRef({ cols: 80, rows: 24 })
  const preMountOutputRef = useRef<Uint8Array[]>([])
  const queuedInputRef = useRef<Uint8Array[]>([])
  const queuedInputBytesRef = useRef(0)
  const inputChainRef = useRef(Promise.resolve())
  const resizeTimerRef = useRef<number | null>(null)
  const gapReplayTimerRef = useRef<number | null>(null)
  const connectionPromiseRef = useRef<Promise<TerminalConnection> | null>(null)
  const localCoordinatorRef = useRef<TerminalConnectionCoordinator | null>(null)
  if (!localCoordinatorRef.current) {
    localCoordinatorRef.current = createTerminalConnectionCoordinator()
  }
  const connectionCoordinator =
    'host' in options
      ? localCoordinatorRef.current
      : (options.connectionCoordinator ?? localCoordinatorRef.current)
  const exitNotifiedRef = useRef(false)
  const liveFramesRef = useRef(new Map<number, TerminalFrame>())

  const saveLease = useCallback(
    (lease: LocalTerminalLease) => {
      leaseRef.current = lease
      saveRecoverableTerminalLease(leaseStore, storageKey, lease)
    },
    [leaseStore, storageKey],
  )

  const removeLease = useCallback(() => {
    leaseRef.current = null
    removeRecoverableTerminalLease(leaseStore, storageKey, paneId)
  }, [leaseStore, paneId, storageKey])

  const appendOutput = useCallback((data: Uint8Array) => {
    const terminal = terminalRef.current
    if (!terminal) {
      preMountOutputRef.current.push(data)
      return
    }
    const viewport = terminal.buffer.active.viewportY
    const following = viewport >= terminal.buffer.active.baseY
    terminal.write(data, () => {
      if (following) {
        terminal.scrollToBottom()
      } else {
        terminal.scrollToLine(viewport)
      }
    })
  }, [])

  const applyFrame = useCallback(
    (frame: TerminalFrame) => {
      const active = activeRef.current
      if (!active || frame.sequence <= active.lastSequence) return
      try {
        appendOutput(decodeBase64(frame.data))
      } catch (error) {
        dispatch({
          type: 'failed',
          error: `Terminal output decode failed: ${errorMessage(error)}`,
        })
        return
      }
      active.lastSequence = frame.sequence
      try {
        saveLease({
          paneId,
          sessionId: active.sessionId,
          reconnectToken: active.reconnectToken,
          lastSequence: active.lastSequence,
        })
      } catch (error) {
        dispatch({ type: 'failed', error: errorMessage(error) })
      }
      dispatch({ type: 'frame-applied', sequence: frame.sequence })
    },
    [appendOutput, paneId, saveLease],
  )

  const scheduleReplay = useCallback(() => {
    if (gapReplayTimerRef.current !== null) return
    gapReplayTimerRef.current = window.setTimeout(() => {
      gapReplayTimerRef.current = null
      setRestartToken((token) => token + 1)
    }, 250)
  }, [])

  const applyExit = useCallback(
    (exit: {
      exit_code: number | null
      signal: string | null
      error: string | null
    }) => {
      if (!exitNotifiedRef.current) {
        let detail = `exit ${exit.exit_code ?? '?'}`
        if (exit.error) detail = `error: ${exit.error}`
        else if (exit.signal) detail = exit.signal
        appendOutput(new TextEncoder().encode(`\r\n[process ${detail}]\r\n`))
        exitNotifiedRef.current = true
      }
      dispatch({ type: 'exited', error: exit.error })
    },
    [appendOutput],
  )

  const applyOutputEvent = useCallback(
    (event: PtyOutputEvent) => {
      const active = activeRef.current
      if (!active || event.session_id !== active.sessionId) return
      if (event.data) {
        let frames: TerminalFrame[]
        try {
          frames = bufferTerminalFrame(
            liveFramesRef.current,
            { sequence: event.sequence, data: event.data },
            active.lastSequence,
          )
        } catch (error) {
          dispatch({ type: 'failed', error: errorMessage(error) })
          return
        }
        for (const frame of frames) applyFrame(frame)
        if (liveFramesRef.current.size > 0) {
          scheduleReplay()
        } else if (gapReplayTimerRef.current !== null) {
          window.clearTimeout(gapReplayTimerRef.current)
          gapReplayTimerRef.current = null
        }
      }
      if (event.eof) {
        applyExit({
          exit_code: event.exit_code,
          signal: event.signal,
          error: event.error,
        })
      }
    },
    [applyExit, applyFrame, scheduleReplay],
  )

  const writeInput = useCallback(
    (session: ActiveTerminalSession, data: Uint8Array) => {
      if (!host) return
      inputChainRef.current = inputChainRef.current
        .catch(() => undefined)
        .then(async () => {
          try {
            await host.iii.trigger('shell::pty::write', {
              session_id: session.sessionId,
              access_key: session.accessKey,
              data: encodeBase64(data),
            })
          } catch (error) {
            if (
              activeRef.current?.sessionId === session.sessionId &&
              activeRef.current.generation === session.generation
            ) {
              dispatch({ type: 'failed', error: errorMessage(error) })
            }
          }
        })
    },
    [host],
  )

  const sendInput = useCallback(
    (data: Uint8Array) => {
      if (data.byteLength === 0) return
      const active = activeRef.current
      if (active) {
        writeInput(active, data)
        return
      }
      if (
        queuedInputBytesRef.current + data.byteLength >
        MAX_QUEUED_INPUT_BYTES
      ) {
        dispatch({
          type: 'failed',
          error: `Terminal input queue exceeds ${MAX_QUEUED_INPUT_BYTES} bytes`,
        })
        return
      }
      queuedInputRef.current.push(data)
      queuedInputBytesRef.current += data.byteLength
    },
    [writeInput],
  )

  const sendResize = useCallback(
    (cols: number, rows: number) => {
      const dimensions = normalizeTerminalDimensions(cols, rows)
      dimensionsRef.current = dimensions
      const active = activeRef.current
      if (!active || !host) return
      if (resizeTimerRef.current !== null) {
        window.clearTimeout(resizeTimerRef.current)
      }
      resizeTimerRef.current = window.setTimeout(() => {
        resizeTimerRef.current = null
        void host.iii
          .trigger('shell::pty::resize', {
            session_id: active.sessionId,
            access_key: active.accessKey,
            cols: dimensions.cols,
            rows: dimensions.rows,
          })
          .catch((error) => {
            if (
              activeRef.current?.sessionId === active.sessionId &&
              activeRef.current.generation === active.generation
            ) {
              dispatch({ type: 'failed', error: errorMessage(error) })
            }
          })
      }, 60)
    },
    [host],
  )

  useEffect(() => {
    if (!visible || !container) return
    const readTheme = () => {
      const styles = window.getComputedStyle(container)
      const color = (name: string, fallback: string) =>
        styles.getPropertyValue(name).trim() || fallback
      const background = color('--color-bg', styles.backgroundColor || '#111111')
      return {
        background,
        foreground: color('--color-ink', styles.color || '#e5e5e5'),
        cursor: color('--color-ink', styles.color || '#e5e5e5'),
        cursorAccent: background,
        selectionBackground: color('--color-surface-active', '#3a3a3a'),
        ...terminalAnsiPalette(background),
      }
    }
    const styles = window.getComputedStyle(container)
    const color = (name: string, fallback: string) =>
      styles.getPropertyValue(name).trim() || fallback
    const terminal = new Terminal({
      cursorBlink: true,
      cursorStyle: 'block',
      fontFamily: color(
        '--font-mono',
        'ui-monospace, SFMono-Regular, Menlo, monospace',
      ),
      fontSize: fontSizeRef.current,
      lineHeight: 1.2,
      scrollback: 10_000,
      scrollOnUserInput: true,
      theme: readTheme(),
    })
    const fitAddon = new FitAddon()
    const terminalHost = document.createElement('div')
    terminalHost.className = 'shui-xterm-host'
    terminal.loadAddon(fitAddon)
    container.appendChild(terminalHost)
    terminal.open(terminalHost)
    terminalRef.current = terminal
    fitAddonRef.current = fitAddon

    const input = terminal.onData((data) =>
      sendInput(new TextEncoder().encode(data)),
    )
    const binary = terminal.onBinary((data) =>
      sendInput(binaryStringToBytes(data)),
    )
    const resized = terminal.onResize(({ cols, rows }) =>
      sendResize(cols, rows),
    )
    const scrolled = terminal.onScroll((viewportY) => {
      setAtBottom(viewportY >= terminal.buffer.active.baseY)
    })
    for (const chunk of preMountOutputRef.current) terminal.write(chunk)
    preMountOutputRef.current = []

    let fitFrame = 0
    const fitTerminal = () => {
      // A pane mid-layout (docking, splitting, a collapsed sidebar) measures
      // as a sliver. Fitting against that hands the PTY a 1-column terminal,
      // and the shell redraws its prompt at that width — the stray "%" marks
      // and clipped prompts that survive the pane growing back.
      const rect = container.getBoundingClientRect()
      if (rect.width < 40 || rect.height < 24) return
      try {
        fitAddon.fit()
      } catch {
        return
      }
      dimensionsRef.current = normalizeTerminalDimensions(
        terminal.cols,
        terminal.rows,
      )
    }
    const scheduleFit = () => {
      if (fitFrame) window.cancelAnimationFrame(fitFrame)
      fitFrame = window.requestAnimationFrame(() => {
        fitFrame = 0
        fitTerminal()
      })
    }
    const observer = new ResizeObserver(scheduleFit)
    observer.observe(container)
    const themeObserver = new MutationObserver(() => {
      terminal.options.theme = readTheme()
    })
    themeObserver.observe(document.documentElement, {
      attributeFilter: ['data-theme', 'class'],
    })
    const frame = window.requestAnimationFrame(() => {
      fitTerminal()
      terminal.focus()
    })

    terminalCleanupRef.current = () => {
      window.cancelAnimationFrame(frame)
      if (fitFrame) window.cancelAnimationFrame(fitFrame)
      themeObserver.disconnect()
      observer.disconnect()
      scrolled.dispose()
      resized.dispose()
      binary.dispose()
      input.dispose()
      terminal.dispose()
      if (terminalRef.current === terminal) terminalRef.current = null
      if (fitAddonRef.current === fitAddon) fitAddonRef.current = null
    }
    return () => {
      terminalCleanupRef.current?.()
      terminalCleanupRef.current = null
    }
  }, [container, sendInput, sendResize, visible])

  // New type means new cell metrics: the pane refits, and the PTY learns the
  // new geometry through the onResize path the session already forwards.
  useEffect(() => {
    const terminal = terminalRef.current
    if (!terminal) return
    terminal.options.fontSize = fontSize
    try {
      fitAddonRef.current?.fit()
    } catch {
      // Mid-layout a pane measures as a sliver; the ResizeObserver refits.
    }
  }, [fontSize])

  useEffect(() => {
    void restartToken
    if (!visible || !router || !host || !root) return
    let cancelled = false
    const attempt = connectionCoordinator.begin()
    const { generation } = attempt
    const { cols, rows } = dimensionsRef.current
    const lease = findPaneLease(leaseStore, storageKey, paneId)
    leaseRef.current = lease
    exitNotifiedRef.current = false
    preMountOutputRef.current = []
    queuedInputRef.current = []
    queuedInputBytesRef.current = 0
    inputChainRef.current = Promise.resolve()
    activeRef.current = null
    liveFramesRef.current.clear()
    dispatch({ type: 'connecting', cwd: root })
    terminalRef.current?.reset()

    const connectionPromise = connectTerminalSession({
      host,
      router,
      root,
      lease,
      requestId: attempt.requestId,
      cols,
      rows,
    })
    connectionPromiseRef.current = connectionPromise
    void connectionPromise
      .then(async (connection) => {
        let credentialsPersisted = false
        try {
          if (cancelled || !connectionCoordinator.isCurrent(generation)) {
            try {
              saveLease({
                paneId,
                sessionId: connection.sessionId,
                reconnectToken: connection.reconnectToken,
                lastSequence: lease?.lastSequence ?? 0,
              })
            } catch {
              await closeTerminalConnection(host, router, connection)
              removeLease()
              return
            }
            if (cancelled && connectionCoordinator.isCurrent(generation)) {
              await disposeTerminalConnection(host, router, connection)
            } else {
              connection.unsubscribe()
            }
            return
          }
          const nextLease = reduceTerminalLease(lease, {
            type: 'restarted',
            lease: {
              paneId,
              sessionId: connection.sessionId,
              reconnectToken: connection.reconnectToken,
              lastSequence: 0,
            },
          })
          if (!nextLease) {
            throw new Error('terminal connection did not produce a lease')
          }
          saveLease(nextLease)
          credentialsPersisted = true
          if (cancelled) {
            await disposeTerminalConnection(host, router, connection)
            return
          }

          const active: ActiveTerminalSession = {
            generation,
            sessionId: connection.sessionId,
            accessKey: connection.accessKey,
            reconnectToken: connection.reconnectToken,
            lastSequence: nextLease.lastSequence,
            unsubscribe: connection.unsubscribe,
          }
          activeRef.current = active
          dispatch({
            type: 'connected',
            sessionId: connection.sessionId,
            cwd: connection.cwd,
          })
          const activated = connection.activate(applyOutputEvent)
          if (connection.truncated) {
            dispatch({ type: 'replay-truncated' })
            appendOutput(new TextEncoder().encode(REPLAY_TRUNCATION_NOTICE))
          }
          if (activated.requiresReplay) {
            scheduleReplay()
          } else {
            for (const frame of activated.frames) {
              if (frame.sequence <= activated.replayThrough) {
                applyFrame(frame)
                continue
              }
              const contiguous = bufferTerminalFrame(
                liveFramesRef.current,
                frame,
                active.lastSequence,
              )
              for (const pendingFrame of contiguous) applyFrame(pendingFrame)
            }
            if (liveFramesRef.current.size > 0) scheduleReplay()
          }
          for (const event of activated.events) applyOutputEvent(event)

          const exited = exitedStatus(connection.status)
          if (exited) {
            applyExit(exited)
          } else if (connection.status === 'detached') {
            dispatch({
              type: 'disconnected',
              error: 'terminal session detached during attach',
            })
          }
          connectionCoordinator.complete(generation)

          const queuedInput = queuedInputRef.current
          queuedInputRef.current = []
          queuedInputBytesRef.current = 0
          for (const data of queuedInput) writeInput(active, data)
          const latest = dimensionsRef.current
          if (latest.cols !== cols || latest.rows !== rows) {
            sendResize(latest.cols, latest.rows)
          }
        } catch (error) {
          if (
            activeRef.current?.sessionId === connection.sessionId &&
            activeRef.current.generation === generation
          ) {
            activeRef.current = null
          }
          try {
            if (credentialsPersisted) {
              await disposeTerminalConnection(host, router, connection)
            } else {
              await closeTerminalConnection(host, router, connection)
              removeLease()
            }
          } catch (cleanupError) {
            throw new Error(
              `${errorMessage(error)}; terminal cleanup failed: ${errorMessage(cleanupError)}`,
            )
          }
          throw error
        }
      })
      .catch((error) => {
        if (cancelled) return
        if (lease && backendConfirmedSessionMissing(error)) {
          removeLease()
          setRestartToken((token) => token + 1)
          return
        }
        if (
          lease &&
          !errorMessage(error).includes('reconnect token is invalid')
        ) {
          dispatch({
            type: 'reconnecting',
            error: errorMessage(error),
          })
          scheduleReplay()
          return
        }
        dispatch({
          type: lease ? 'disconnected' : 'failed',
          error: errorMessage(error),
        })
      })
      .finally(() => {
        if (connectionPromiseRef.current === connectionPromise) {
          connectionPromiseRef.current = null
        }
      })

    return () => {
      cancelled = true
      const active = activeRef.current
      activeRef.current = null
      queuedInputRef.current = []
      queuedInputBytesRef.current = 0
      if (!active) return
      void detachTerminalSessionForUnmount(
        host,
        router,
        storageKey,
        paneId,
        active,
      ).catch(() => undefined)
    }
  }, [
    appendOutput,
    applyExit,
    applyFrame,
    applyOutputEvent,
    connectionCoordinator,
    host,
    leaseStore,
    paneId,
    removeLease,
    restartToken,
    root,
    router,
    saveLease,
    scheduleReplay,
    sendResize,
    storageKey,
    visible,
    writeInput,
  ])

  useEffect(() => {
    if (!visible || !host) return
    let interrupted = false
    let retryScheduled = false
    return host.iii.addConnectionStateListener((connectionState) => {
      if (connectionState === 'connected') {
        if (interrupted && !retryScheduled) {
          interrupted = false
          retryScheduled = true
          connectionCoordinator.invalidate()
          setRestartToken((token) => token + 1)
          retryScheduled = false
        }
        return
      }
      interrupted = true
      if (activeRef.current || connectionPromiseRef.current) {
        dispatch({
          type: 'reconnecting',
          error: 'Terminal transport disconnected; reconnecting',
        })
      }
    })
  }, [connectionCoordinator, host, visible])

  useEffect(() => {
    if (!visible || !host) return
    const heartbeat = window.setInterval(() => {
      const active = activeRef.current
      if (!active) return
      const { cols, rows } = dimensionsRef.current
      void host.iii
        .trigger('shell::pty::resize', {
          session_id: active.sessionId,
          access_key: active.accessKey,
          cols,
          rows,
        })
        .catch((error) => {
          if (
            activeRef.current?.sessionId === active.sessionId &&
            activeRef.current.generation === active.generation
          ) {
            dispatch({ type: 'failed', error: errorMessage(error) })
          }
        })
    }, HEARTBEAT_MS)
    return () => window.clearInterval(heartbeat)
  }, [host, visible])

  useEffect(
    () => () => {
      if (resizeTimerRef.current !== null) {
        window.clearTimeout(resizeTimerRef.current)
      }
      if (gapReplayTimerRef.current !== null) {
        window.clearTimeout(gapReplayTimerRef.current)
      }
    },
    [],
  )

  const close = useCallback(async () => {
    if (connectionPromiseRef.current) {
      try {
        await connectionPromiseRef.current
      } catch {
        connectionPromiseRef.current = null
      }
    }
    const active = activeRef.current
    if (active && host) {
      try {
        await host.iii.trigger('shell::pty::close', {
          session_id: active.sessionId,
          access_key: active.accessKey,
        })
      } catch (error) {
        if (!backendConfirmedSessionMissing(error)) {
          dispatch({ type: 'failed', error: errorMessage(error) })
          throw error
        }
      }
      active.unsubscribe()
      router?.drain(active.sessionId)
      activeRef.current = null
      let warning: string | null = null
      try {
        removeLease()
      } catch (error) {
        warning = errorMessage(error)
      }
      dispatch({ type: 'closed' })
      return warning
    }

    const lease =
      leaseRef.current ?? findPaneLease(leaseStore, storageKey, paneId)
    if (lease && host && router) {
      try {
        const warning = await reclaimTerminalLease(host, router, {
          ...lease,
          update: saveLease,
          remove: removeLease,
        })
        dispatch({ type: 'closed' })
        return warning
      } catch (error) {
        dispatch({ type: 'failed', error: errorMessage(error) })
        throw error
      }
    }
    try {
      removeLease()
    } catch (error) {
      dispatch({ type: 'closed' })
      return errorMessage(error)
    }
    dispatch({ type: 'closed' })
    return null
  }, [host, leaseStore, paneId, removeLease, router, saveLease, storageKey])

  const restart = useCallback(() => {
    void close()
      .then(() => {
        terminalRef.current?.reset()
        preMountOutputRef.current = []
        setRestartToken((token) => token + 1)
      })
      .catch(() => undefined)
  }, [close])

  const forget = useCallback(() => {
    const active = activeRef.current
    activeRef.current = null
    if (active) {
      active.unsubscribe()
      router?.drain(active.sessionId)
      if (host) {
        void host.iii
          .trigger('shell::pty::detach', {
            session_id: active.sessionId,
            access_key: active.accessKey,
          })
          .catch(() => undefined)
      }
    }
    liveFramesRef.current.clear()
    removeLease()
    dispatch({ type: 'closed' })
  }, [host, removeLease, router])

  const startFresh = useCallback(() => {
    forget()
    terminalRef.current?.reset()
    preMountOutputRef.current = []
    setRestartToken((token) => token + 1)
  }, [forget])

  const focus = useCallback(() => terminalRef.current?.focus(), [])
  const jumpToLatest = useCallback(() => {
    terminalRef.current?.scrollToBottom()
    terminalRef.current?.focus()
    setAtBottom(true)
  }, [])

  return {
    atBottom,
    cwd: state.cwd,
    error: state.error,
    focus,
    jumpToLatest,
    restart,
    startFresh,
    forget,
    close,
    setContainer,
    status: state.status,
  }
}
