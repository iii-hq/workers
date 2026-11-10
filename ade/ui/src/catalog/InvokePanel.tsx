/**
 * Call one function with a JSON body and show what came back.
 *
 * The body opens on a template generated from the function's registered
 * `request_schema` (./schema.ts), and the editor is the console's Monaco
 * `CodeEditor` fed the schema's field names as completions — the same editor
 * the rest of the console uses, never a bundled second one.
 *
 * Two things the old console did not do:
 *
 * - required fields are checked against the schema BEFORE the call, so an
 *   obvious mistake reads as "scope is required" instead of a worker-side
 *   serialization error
 * - every call this panel makes is kept for the session and can be replayed,
 *   so tuning a payload is a loop rather than a retype
 */

import { Button, CodeEditor, type Host, JsonHighlight, StatusDot, StatusPanel } from '@iii-dev/console-ui'
import { Play } from 'lucide-react'
import { type MutableRefObject, useEffect, useMemo, useRef, useState } from 'react'
import { formatDuration } from './ActivityFeed'
import { asCliCommand } from './cli-command'
import { type InvokeOutcome, invoke } from './engine'
import { schemaFieldNames } from './SchemaTable'
import { pretty, templateFromSchema } from './schema'
import { CopyButton, CopyIconButton, ShortcutHint } from './widgets'

interface Attempt {
  id: number
  atMs: number
  body: string
  outcome: InvokeOutcome
}

let attemptSeq = 0

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Required top-level fields the body is missing, by the schema's own list. */
function missingRequired(schema: unknown, payload: unknown): string[] {
  if (!isRecord(schema) || !isRecord(payload)) return []
  const required = Array.isArray(schema.required)
    ? schema.required.filter((k): k is string => typeof k === 'string')
    : []
  return required.filter((key) => payload[key] === undefined || payload[key] === '')
}

export function InvokePanel({
  host,
  functionId,
  requestSchema,
  label = 'Call',
  runningLabel = 'Calling…',
  hint,
  prefill,
  runRef,
}: {
  host: Host
  functionId: string
  requestSchema: unknown
  /** Verb on the button — the observers page calls an observer's target. */
  label?: string
  runningLabel?: string
  hint?: string
  /** A recorded input pushed in from the activity feed; changes replace the body. */
  prefill?: { value: unknown; nonce: number }
  /** Kept current with this panel's run() while mounted, so the page's
      Mod+Enter command can reach it without lifting this form up. */
  runRef?: MutableRefObject<(() => void) | null>
}) {
  const [body, setBody] = useState('{}')
  const [running, setRunning] = useState(false)
  const [attempts, setAttempts] = useState<Attempt[]>([])
  const [invalid, setInvalid] = useState<string | null>(null)

  // A new selection resets the editor to that function's own template and
  // drops the previous function's attempts with it. Keyed on the schema's
  // CONTENT, not its identity — live catalog refreshes rebuild the object
  // every tick, and resetting on identity would wipe a body mid-edit.
  const schemaKey = JSON.stringify(requestSchema) ?? 'none'
  useEffect(() => {
    setBody(templateFromSchema(requestSchema))
    setAttempts([])
    setInvalid(null)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [functionId, schemaKey])

  useEffect(() => {
    if (!prefill) return
    setBody(pretty(prefill.value) || '{}')
    setInvalid(null)
  }, [prefill])

  const completions = useMemo(() => schemaFieldNames(requestSchema), [requestSchema])

  const latest = attempts[0] ?? null
  const outcome = latest?.outcome ?? null

  const reset = () => {
    setBody(templateFromSchema(requestSchema))
    setInvalid(null)
  }

  const inFlightRef = useRef(false)
  const run = async () => {
    let payload: unknown
    try {
      payload = JSON.parse(body)
    } catch (err) {
      setInvalid(err instanceof Error ? err.message : 'Invalid JSON')
      return
    }
    if (!isRecord(payload)) {
      setInvalid('The request body must be a JSON object')
      return
    }
    const missing = missingRequired(requestSchema, payload)
    if (missing.length > 0) {
      setInvalid(`${missing.join(', ')} ${missing.length > 1 ? 'are' : 'is'} required by the schema`)
      return
    }
    if (inFlightRef.current) return
    inFlightRef.current = true
    setInvalid(null)
    setRunning(true)
    let result: Awaited<ReturnType<typeof invoke>>
    try {
      result = await invoke(host, functionId, payload)
    } finally {
      inFlightRef.current = false
      setRunning(false)
    }
    attemptSeq += 1
    setAttempts((prev) => [{ id: attemptSeq, atMs: Date.now(), body, outcome: result }, ...prev.slice(0, 9)])
  }

  if (runRef) runRef.current = run
  useEffect(() => {
    return () => {
      if (runRef) runRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const request = (
    <>
      <div className="console-catalog-invoke-head">
        <div>
          <h3>Request</h3>
          <p className="invoke-description">A JSON body; required fields are checked before the call.</p>
        </div>
        <Button
          className="console-catalog-btn"
          variant="ghost"
          size="sm"
          type="button"
          onClick={reset}
          disabled={running}
        >
          Reset
        </Button>
      </div>
      {hint ? <div className="console-catalog-note">{hint}</div> : null}
      {/* Monaco binds ⌘↵ to "insert line below" and swallows it, so the run
          shortcut is caught on the way down, before the editor sees it. */}
      <div
        className="console-catalog-editor-wrap"
        onKeyDownCapture={(event) => {
          if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
            event.preventDefault()
            event.stopPropagation()
            void run()
          }
        }}
      >
        <CodeEditor
          fontSize={13}
          value={body}
          onChange={setBody}
          language="json"
          className="console-catalog-editor"
          completions={completions}
          aria-label={`Request body for ${functionId}`}
        />
      </div>
      <div className="console-catalog-invoke-foot">
        <Button
          className="console-catalog-btn"
          type="button"
          variant="primary"
          size="sm"
          onClick={run}
          disabled={running}
        >
          <Play aria-hidden />
          {running ? runningLabel : label}
          <ShortcutHint keys="enter" />
        </Button>
        <CopyButton
          value={asCliCommand(functionId, safeJson(body))}
          label="Copy as command"
          title="Copies this exact call as an `iii call …` line for the terminal"
        />
        {invalid ? <span className="console-catalog-invalid">{invalid}</span> : null}
      </div>
    </>
  )

  // Keyed by attempt, so each result enters once instead of mutating in place.
  const response = outcome ? (
    <div key={latest?.id} className="console-catalog-result-shell" data-ok={outcome.ok}>
      <div className="console-catalog-result-head">
        <span className={outcome.ok ? 'console-catalog-ok' : 'console-catalog-invalid'}>
          <StatusDot tone={outcome.ok ? 'ok' : 'alert'} />
          {outcome.ok ? 'Success' : 'Error'}
        </span>
        <span className="result-meta">
          {latest ? clockTime(latest.atMs) : null}
          <span>{formatDuration(outcome.durationMs)}</span>
          {outcome.ok ? <CopyIconButton value={pretty(outcome.data) || 'null'} label="Copy response" /> : null}
        </span>
      </div>
      {outcome.error ? (
        <StatusPanel variant="alert" headline={outcome.error} />
      ) : (
        <JsonHighlight code={pretty(outcome.data) || 'null'} className="console-catalog-result" wrap />
      )}
    </div>
  ) : null

  const history =
    attempts.length > 0 ? (
      <div className="console-catalog-attempts">
        <h4 className="console-catalog-field-label">This session</h4>
        {attempts.map((attempt) => (
          <button
            key={attempt.id}
            type="button"
            className="console-catalog-attempt"
            data-latest={attempt.id === latest?.id}
            onClick={() => setBody(attempt.body)}
            title="Put this body back in the editor"
          >
            <StatusDot tone={attempt.outcome.ok ? 'ok' : 'alert'} />
            <span className="time">{clockTime(attempt.atMs)}</span>
            <span className="body">{oneLine(attempt.body)}</span>
            <span className="duration">{formatDuration(attempt.outcome.durationMs)}</span>
          </button>
        ))}
      </div>
    ) : null

  return (
    <div className="console-catalog-invoke">
      <section className="invoke-column" aria-label="Request">
        {request}
        {history}
      </section>
      <section className="invoke-column" aria-label="Response">
        <div className="console-catalog-invoke-head">
          <div>
            <h3>Response</h3>
          </div>
        </div>
        {response ?? (
          <div className="console-catalog-response-empty">
            Run it to see what comes back. Each run stays in this session's list, so you can load an earlier body back.
          </div>
        )}
      </section>
    </div>
  )
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour12: false })
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return {}
  }
}

function oneLine(body: string): string {
  const flat = body.replace(/\s+/g, ' ').trim()
  return flat.length > 64 ? `${flat.slice(0, 61)}…` : flat
}
