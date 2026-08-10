/**
 * The console tab: an exec composer over `sandbox::exec` (command line,
 * workdir, timeout, env rows, `sh -c` and `setsid` toggles — argv shaping
 * lives in exec.ts, where its tests are), quick probes, and the persisted
 * timeline of ExecRecord cards (records.ts).
 *
 * ↑/↓ in the command input walk the command history (this timeline's
 * commands, deduped, newest first). The composer disables while the
 * sandbox has zero free exec slots — with a live note, since the poll
 * store keeps `exec_slots_free` current — and entirely once the sandbox
 * stopped.
 */

import { Button, type Host, Input } from '@iii-dev/console-ui'
import { type KeyboardEvent, useMemo, useRef, useState } from 'react'
import { AnsiText } from '../sandbox-family/ansi'
import {
  buildExecPayload,
  DETACH_TITLE,
  type ExecFormValues,
  exitPill,
  formatTriggerCommand,
  parseExecResult,
} from './exec'
import { formatMs } from './format'
import type { ExecRecord, ExecRecords } from './records'
import type { SandboxSummary } from './store'
import { EXEC_SLOTS } from './store'
import { CopyButton, type EnvRow, EnvRowsEditor } from './widgets'

const PROBES: { label: string; line: string }[] = [
  { label: 'ps', line: 'ps aux' },
  { label: 'env', line: 'env' },
  { label: 'df -h', line: 'df -h' },
]

let recordSeq = 0
const nextRecordId = () => `r-${Date.now()}-${recordSeq++}`

function payloadFromRecord(
  sandboxId: string,
  record: ExecRecord,
): Record<string, unknown> {
  return buildExecPayload(sandboxId, {
    line: record.cmd,
    shell: record.shell,
    detached: record.detached,
    workdir: record.workdir ?? '',
    timeoutMs: record.timeout_ms !== undefined ? String(record.timeout_ms) : '',
    env: Object.entries(record.env ?? {}).map(([key, value]) => ({ key, value })),
  })
}

function RecordCard({
  sandboxId,
  record,
}: {
  sandboxId: string
  record: ExecRecord
}) {
  const pill = exitPill(record)
  const copyCommand =
    record.source === 'run'
      ? null
      : formatTriggerCommand('sandbox::exec', payloadFromRecord(sandboxId, record))
  return (
    <article className="cr-page-record">
      <header className="cr-page-record-head">
        <code className="cr-page-record-cmd" title={record.cmd}>
          {record.cmd}
        </code>
        <span className={`cr-page-exit-pill ${pill.tone}`}>{pill.label}</span>
        {record.duration_ms !== null ? (
          <span className="cr-page-record-dur">{formatMs(record.duration_ms)}</span>
        ) : null}
        {copyCommand ? (
          <CopyButton
            text={copyCommand}
            title="copy as an `iii trigger sandbox::exec …` command"
          />
        ) : null}
      </header>
      <div className="cr-page-record-chips">
        {record.source === 'probe' ? <span className="cr-page-chip">probe</span> : null}
        {record.source === 'run' ? <span className="cr-page-chip">run code</span> : null}
        {record.shell && !record.detached ? (
          <span className="cr-page-chip">sh -c</span>
        ) : null}
        {record.detached ? <span className="cr-page-chip">setsid · detached</span> : null}
        {record.workdir ? (
          <span className="cr-page-chip" title={record.workdir}>
            wd {record.workdir}
          </span>
        ) : null}
        {record.timeout_ms !== undefined ? (
          <span className="cr-page-chip">timeout {formatMs(record.timeout_ms)}</span>
        ) : null}
        {record.env && Object.keys(record.env).length > 0 ? (
          <span className="cr-page-chip">
            {Object.keys(record.env).length} env
          </span>
        ) : null}
      </div>
      {record.error ? (
        <div className="cr-page-inline-error">{record.error}</div>
      ) : (
        <>
          {record.stdout ? (
            <div className="cr-page-stream out">
              <div className="cr-page-stream-label">stdout</div>
              <pre className="cr-page-stream-body">
                <AnsiText text={record.stdout} />
              </pre>
            </div>
          ) : null}
          {record.stderr ? (
            <div className="cr-page-stream err">
              <div className="cr-page-stream-label">stderr</div>
              <pre className="cr-page-stream-body">
                <AnsiText text={record.stderr} />
              </pre>
            </div>
          ) : null}
          {!record.stdout && !record.stderr ? (
            <div className="cr-page-faint cr-page-no-output">no output</div>
          ) : null}
        </>
      )}
    </article>
  )
}

export function ConsoleTab({
  host,
  sandbox,
  timeline,
}: {
  host: Host
  sandbox: SandboxSummary
  timeline: ExecRecords
}) {
  const [line, setLine] = useState('')
  const [workdir, setWorkdir] = useState('')
  const [timeoutMs, setTimeoutMs] = useState('')
  const [env, setEnv] = useState<EnvRow[]>([])
  const [shell, setShell] = useState(true)
  const [detached, setDetached] = useState(false)
  const [running, setRunning] = useState(false)
  // History cursor: -1 = composing; 0.. = index into `history` (newest first).
  const cursor = useRef(-1)
  const draft = useRef('')

  const history = useMemo(() => {
    const seen = new Set<string>()
    const out: string[] = []
    for (let i = timeline.records.length - 1; i >= 0; i--) {
      const cmd = timeline.records[i].cmd
      if (timeline.records[i].source === 'run' || seen.has(cmd)) continue
      seen.add(cmd)
      out.push(cmd)
    }
    return out
  }, [timeline.records])

  const slotsFull = sandbox.exec_slots_free === 0
  const disabled = sandbox.stopped || running

  const submit = (form: ExecFormValues, source: 'exec' | 'probe') => {
    if (!form.line.trim() || running) return
    setRunning(true)
    const payload = buildExecPayload(sandbox.sandbox_id, form)
    const requested = typeof payload.timeout_ms === 'number' ? payload.timeout_ms : 30_000
    const base: Omit<ExecRecord, 'stdout' | 'stderr' | 'exit_code' | 'timed_out' | 'duration_ms'> = {
      id: nextRecordId(),
      at: Date.now(),
      cmd: form.line,
      shell: form.shell,
      detached: form.detached,
      workdir: form.workdir.trim() || undefined,
      timeout_ms: typeof payload.timeout_ms === 'number' ? payload.timeout_ms : undefined,
      env:
        payload.env && typeof payload.env === 'object'
          ? (payload.env as Record<string, string>)
          : undefined,
      source,
    }
    host.iii
      // Give the transport headroom past the exec's own deadline, so the
      // daemon's timed_out verdict arrives instead of a client-side abort.
      .trigger('sandbox::exec', payload, { timeoutMs: Math.max(60_000, requested + 15_000) })
      .then((value) => {
        const result = parseExecResult(value)
        timeline.append({ ...base, ...result })
      })
      .catch((err: unknown) => {
        timeline.append({
          ...base,
          stdout: '',
          stderr: '',
          exit_code: null,
          timed_out: false,
          duration_ms: null,
          error: err instanceof Error ? err.message : String(err),
        })
      })
      .finally(() => setRunning(false))
  }

  const submitComposer = () => {
    submit({ line, workdir, timeoutMs, env, shell, detached }, 'exec')
    setLine('')
    cursor.current = -1
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      submitComposer()
      return
    }
    if (event.key === 'ArrowUp') {
      if (cursor.current + 1 >= history.length) return
      event.preventDefault()
      if (cursor.current === -1) draft.current = line
      cursor.current += 1
      setLine(history[cursor.current])
      return
    }
    if (event.key === 'ArrowDown') {
      if (cursor.current === -1) return
      event.preventDefault()
      cursor.current -= 1
      setLine(cursor.current === -1 ? draft.current : history[cursor.current])
    }
  }

  const ordered = useMemo(
    () => timeline.records.slice().reverse(),
    [timeline.records],
  )

  return (
    <div className="cr-page-console">
      {sandbox.stopped ? (
        <div className="cr-page-banner warn" role="status">
          sandbox ended — exec is gone; the timeline below is history.
        </div>
      ) : (
        <div className="cr-page-composer">
          <div className="cr-page-composer-line">
            <Input
              value={line}
              onChange={(next) => {
                cursor.current = -1
                setLine(next)
              }}
              onKeyDown={onKeyDown}
              placeholder="command — ↑/↓ for history, Enter to run"
              preserveCase
              spellCheck={false}
              aria-label="command"
              disabled={disabled || slotsFull}
            />
            <Button
              variant="primary"
              size="sm"
              onClick={submitComposer}
              disabled={disabled || slotsFull || !line.trim()}
            >
              {running ? 'running…' : 'exec'}
            </Button>
          </div>
          <div className="cr-page-composer-opts">
            <Input
              value={workdir}
              onChange={setWorkdir}
              placeholder="workdir (optional)"
              preserveCase
              spellCheck={false}
              aria-label="workdir"
              disabled={disabled}
            />
            <Input
              value={timeoutMs}
              onChange={setTimeoutMs}
              placeholder="timeout_ms"
              inputMode="numeric"
              aria-label="timeout in milliseconds"
              disabled={disabled}
            />
            <button
              type="button"
              className={`cr-page-toggle${shell && !detached ? ' on' : ''}`}
              onClick={() => setShell((v) => !v)}
              disabled={disabled || detached}
              title="wrap the line as sh -c '<cmd>' so pipes, globs and redirects work; off sends the line for the daemon to shlex-split"
              aria-pressed={shell && !detached}
            >
              sh -c
            </button>
            <button
              type="button"
              className={`cr-page-toggle${detached ? ' on' : ''}`}
              onClick={() => setDetached((v) => !v)}
              disabled={disabled}
              title={DETACH_TITLE}
              aria-pressed={detached}
            >
              setsid
            </button>
          </div>
          <EnvRowsEditor rows={env} onChange={setEnv} disabled={disabled} />
          <div className="cr-page-probes">
            {PROBES.map((probe) => (
              <button
                key={probe.label}
                type="button"
                className="cr-page-probe"
                title={`${probe.line} — uses 1 exec slot`}
                disabled={disabled || slotsFull}
                onClick={() =>
                  submit(
                    {
                      line: probe.line,
                      workdir: '',
                      timeoutMs: '',
                      env: [],
                      shell: true,
                      detached: false,
                    },
                    'probe',
                  )
                }
              >
                {probe.label}
              </button>
            ))}
            <span className="cr-page-faint">probes use 1 exec slot</span>
          </div>
          {slotsFull ? (
            <div className="cr-page-banner warn" role="status">
              all {EXEC_SLOTS} exec slots are busy — the composer re-enables
              the moment one frees (this page polls live).
            </div>
          ) : null}
        </div>
      )}

      <div className="cr-page-timeline">
        {!timeline.loaded && ordered.length === 0 ? (
          <div className="cr-page-faint">loading timeline…</div>
        ) : ordered.length === 0 ? (
          <div className="cr-page-faint">
            nothing run yet — records persist per sandbox, newest first.
          </div>
        ) : (
          ordered.map((record) => (
            <RecordCard
              key={record.id}
              sandboxId={sandbox.sandbox_id}
              record={record}
            />
          ))
        )}
      </div>
    </div>
  )
}
