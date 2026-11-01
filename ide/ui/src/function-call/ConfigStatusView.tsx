import { Badge, TerminalStream } from '@iii-dev/console-ui'
import { safeParseResponse, shellConfigStatusResponseSchema } from './parsers'

interface ShellConfigStatusViewProps {
  output: unknown
  running?: boolean
}

/**
 * `shell::config-status` — the worker's config-reload health. A slab
 * with the last reload outcome, a rejected-reload counter (warn pill
 * when non-zero), and the last rejection error verbatim. The request
 * payload is ignored server-side, so nothing request-derived renders.
 */
export function ShellConfigStatusView({
  output,
  running,
}: ShellConfigStatusViewProps) {
  const resp =
    output != null
      ? safeParseResponse(shellConfigStatusResponseSchema, output)
      : null

  if (!resp) {
    if (!running) return null
    return <div className="shui-card shui-running">checking config…</div>
  }

  const applied = resp.last_outcome === 'applied'
  return (
    <div className="shui-card">
      <div className="shui-slab" data-tone={applied ? 'accent' : 'alert'}>
        <div className="shui-row">
          <Badge variant={applied ? 'accent' : 'alert'}>{resp.last_outcome}</Badge>
          <Badge variant={resp.rejected_reloads > 0 ? 'warn' : 'default'}>
            {`rejected reloads ${resp.rejected_reloads}`}
          </Badge>
        </div>
        {resp.last_error != null ? <TerminalStream label="last error" text={resp.last_error} tone="err" /> : null}
      </div>
    </div>
  )
}
