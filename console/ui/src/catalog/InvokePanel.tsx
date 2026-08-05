/**
 * Call one function with a JSON body and show what came back.
 *
 * The body opens on a template generated from the function's registered
 * `request_schema` (../catalog/schema.ts) so the operator edits real field
 * names rather than typing the shape from memory. Editing is the console's
 * Monaco `CodeEditor` — the one editor, never a bundled second one.
 *
 * A call goes out as a plain `host.iii.trigger`, which is exactly what any
 * bus client can already do; there is no separate privilege here. Failures
 * render as the worker's own error text, never a swallowed empty result.
 */

import {
  Button,
  CodeEditor,
  type Host,
  JsonHighlight,
} from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { type InvokeOutcome, invoke } from './engine'
import { pretty, templateFromSchema } from './schema'

export function InvokePanel({
  host,
  functionId,
  requestSchema,
  label = 'invoke',
  runningLabel = 'invoking…',
  hint,
}: {
  host: Host
  functionId: string
  requestSchema: unknown
  /** Verb on the button — the triggers page fires a target function. */
  label?: string
  runningLabel?: string
  hint?: string
}) {
  const [body, setBody] = useState('{}')
  const [running, setRunning] = useState(false)
  const [outcome, setOutcome] = useState<InvokeOutcome | null>(null)
  const [invalid, setInvalid] = useState<string | null>(null)

  // A new selection resets the editor to that function's own template; an
  // in-flight result from the previous selection is dropped with it.
  useEffect(() => {
    setBody(templateFromSchema(requestSchema))
    setOutcome(null)
    setInvalid(null)
  }, [functionId, requestSchema])

  const run = async () => {
    let payload: unknown
    try {
      payload = JSON.parse(body)
    } catch (err) {
      setInvalid(err instanceof Error ? err.message : 'invalid JSON')
      setOutcome(null)
      return
    }
    if (
      payload === null ||
      typeof payload !== 'object' ||
      Array.isArray(payload)
    ) {
      setInvalid('the request body must be a JSON object')
      setOutcome(null)
      return
    }
    setInvalid(null)
    setRunning(true)
    const result = await invoke(
      host,
      functionId,
      payload as Record<string, unknown>,
    )
    setRunning(false)
    setOutcome(result)
  }

  return (
    <div className="console-catalog-invoke">
      {hint ? <div className="console-catalog-note">{hint}</div> : null}
      <CodeEditor
        value={body}
        onChange={setBody}
        language="json"
        className="console-catalog-editor"
        aria-label={`request body for ${functionId}`}
      />
      <div className="console-catalog-invoke-foot">
        <Button size="sm" onClick={run} disabled={running}>
          {running ? runningLabel : label}
        </Button>
        {invalid ? (
          <span className="console-catalog-invalid">{invalid}</span>
        ) : null}
        {outcome ? (
          <span
            className={
              outcome.ok ? 'console-catalog-ok' : 'console-catalog-invalid'
            }
          >
            {outcome.ok ? 'ok' : 'error'} · {Math.round(outcome.durationMs)}ms
          </span>
        ) : null}
      </div>
      {outcome?.error ? (
        <div className="console-catalog-error">{outcome.error}</div>
      ) : null}
      {outcome?.ok ? (
        <JsonHighlight
          code={pretty(outcome.data) || 'null'}
          className="console-catalog-result"
          wrap
        />
      ) : null}
    </div>
  )
}
