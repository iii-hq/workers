import { Button, MetaRow, StatusPanel, TerminalCommandLine, TerminalStream, useConfirm } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { Api } from './api'
import { processFacts } from './presentation'
import { setDisabled } from './save'
import type { ProcessLogs, Server } from './types'

/** Column 3 for the Process row of a stdio server: the command, the process's facts, its stderr tail, Restart
 * now and Stop. */
export function ProcessView({ api, server, cwd }: { api: Api; server: Server; cwd: string | null }) {
  const { confirm, dialog } = useConfirm()
  const [logs, setLogs] = useState<ProcessLogs | null>(null)
  const [readError, setReadError] = useState<string | null>(null)
  const [failure, setFailure] = useState<{ headline: string; detail: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const seq = useRef(0)
  const tail = useRef<HTMLDivElement | null>(null)
  // The tail follows the newest line until the person scrolls up.
  const follow = useRef(true)

  // `server` is a new object after every `servers::list` read, which the worker's `stderr` and `process` events
  // cause: the tail follows them with no observer of its own.
  useEffect(() => {
    const mine = ++seq.current
    api.processLogs(server.name).then(
      (next) => {
        if (mine !== seq.current) return
        setLogs(next)
        setReadError(null)
      },
      (err) => {
        if (mine === seq.current) setReadError(errorMessage(err))
      },
    )
  }, [api, server])
  useLayoutEffect(() => {
    if (logs && tail.current && follow.current) tail.current.scrollTop = tail.current.scrollHeight
  }, [logs])

  const act = async (headline: string, run: () => Promise<unknown>) => {
    setBusy(true)
    setFailure(null)
    try {
      await run()
    } catch (err) {
      setFailure({ headline, detail: errorMessage(err) })
    } finally {
      setBusy(false)
    }
  }
  const stop = async () => {
    const ok = await confirm({
      title: `Stop ${server.name}?`,
      description: 'The process ends and its functions go away until you turn the server on again.',
      confirmLabel: 'Stop',
    })
    if (ok) await act('Could not stop', () => setDisabled(api, server.name, true))
  }

  return (
    <div className="mcp-ui-detail">
      <TerminalCommandLine className="mcp-ui-cli" command={server.target} copy />
      <p className="mcp-ui-cwd">
        <span>Working directory</span>
        {cwd === null ? <span>The worker's working directory</span> : <code>{cwd}</code>}
      </p>
      {server.process ? <MetaRow className="mcp-ui-facts" items={processFacts(server.process, Date.now())} /> : null}
      {failure ? (
        <StatusPanel variant="alert" role="alert" headline={failure.headline} detail={failure.detail} />
      ) : null}
      <div className="mcp-ui-run">
        <Button
          size="sm"
          disabled={busy}
          onClick={() => void act('Could not restart', () => api.reconnect(server.name))}
        >
          Restart now
        </Button>
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => void stop()}>
          Stop
        </Button>
      </div>
      {readError ? <StatusPanel variant="alert" headline="Could not read stderr" detail={readError} /> : null}
      {logs && logs.dropped > 0 ? (
        <p className="mcp-ui-quiet-line">
          {logs.dropped} earlier {logs.dropped === 1 ? 'line' : 'lines'} dropped
        </p>
      ) : null}
      {logs && logs.lines.length === 0 ? <p className="mcp-ui-quiet-line">No output on stderr yet.</p> : null}
      {logs && logs.lines.length > 0 ? (
        <div
          className="mcp-ui-stderr"
          ref={tail}
          onScroll={(event) => {
            const el = event.currentTarget
            follow.current = el.scrollHeight - el.scrollTop - el.clientHeight <= 4
          }}
        >
          {/* Clamped above the worker's 500-line, 4 KiB ring, so the newest lines are never the hidden ones. */}
          <TerminalStream
            label="stderr"
            text={logs.lines.map((line) => line.text).join('\n')}
            clampLines={501}
            clampChars={2_100_000}
          />
        </div>
      ) : null}
      {dialog}
    </div>
  )
}
