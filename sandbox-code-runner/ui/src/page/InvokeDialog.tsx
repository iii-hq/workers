/**
 * Invoke a runtime-registered bus function by hand: a JSON payload editor,
 * one call, and the raw result (JsonHighlight) or the failure. The
 * function id is invoked verbatim over `host.iii.trigger` — these are the
 * ids guest code registered through the planted SDK, so whatever the guest
 * handler returns renders as-is.
 */

import {
  Button,
  CodeEditor,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type Host,
  JsonHighlight,
} from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'

export function InvokeDialog({
  host,
  functionId,
  onClose,
}: {
  host: Host
  /** `null` renders the dialog closed. */
  functionId: string | null
  onClose(): void
}) {
  const [payload, setPayload] = useState('{}')
  const [parseError, setParseError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<string | null>(null)
  const [callError, setCallError] = useState<string | null>(null)
  // Bumped per function — a still-in-flight invoke from the previous
  // function must not write its response under the new title.
  const generation = useRef(0)

  // Fresh slate per function — a result from the previous invoke must not
  // sit under a different function's title.
  useEffect(() => {
    generation.current += 1
    setPayload('{}')
    setParseError(null)
    setResult(null)
    setCallError(null)
    setBusy(false)
  }, [functionId])

  const invoke = () => {
    let parsed: unknown
    try {
      parsed = payload.trim() === '' ? {} : JSON.parse(payload)
    } catch (err) {
      setParseError(err instanceof Error ? err.message : String(err))
      return
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      setParseError('payload must be a JSON object')
      return
    }
    if (!functionId) return
    setParseError(null)
    setCallError(null)
    setResult(null)
    setBusy(true)
    const gen = generation.current
    host.iii
      // Guest handlers run real code — give them the same transport
      // headroom as the run dialog instead of the default deadline.
      .trigger(functionId, parsed as Record<string, unknown>, { timeoutMs: 120_000 })
      .then((value) => {
        if (gen === generation.current)
          setResult(JSON.stringify(value, null, 2) ?? 'null')
      })
      .catch((err: unknown) => {
        if (gen === generation.current)
          setCallError(err instanceof Error ? err.message : String(err))
      })
      .finally(() => {
        if (gen === generation.current) setBusy(false)
      })
  }

  return (
    <Dialog open={functionId !== null} onOpenChange={(open) => !open && onClose()}>
      {functionId === null ? null : (
        <DialogContent className="cr-page-dialog">
          <DialogTitle>trigger {functionId}</DialogTitle>
          <DialogDescription>
            triggers the guest-registered function over the bus with the input
            payload below — this runs real code in its runtime.
          </DialogDescription>
          <div className="cr-page-editor-pane">
            <CodeEditor
              value={payload}
              onChange={setPayload}
              language="json"
              className="cr-page-payload-editor"
              aria-label="input payload (json)"
              autoFocus
            />
          </div>
          {parseError ? (
            <div className="cr-page-inline-error" role="alert">
              payload is not valid JSON: {parseError}
            </div>
          ) : null}
          <div className="cr-page-dialog-actions">
            <Button variant="ghost" size="sm" onClick={onClose}>
              close
            </Button>
            <Button variant="primary" size="sm" onClick={invoke} disabled={busy}>
              {busy ? 'triggering…' : 'trigger'}
            </Button>
          </div>
          {callError ? (
            <div className="cr-page-errcard" role="alert">
              <div className="cr-page-errcard-msg">{callError}</div>
            </div>
          ) : null}
          {result !== null ? (
            <JsonHighlight code={result} className="cr-page-invoke-result" />
          ) : null}
        </DialogContent>
      )}
    </Dialog>
  )
}
