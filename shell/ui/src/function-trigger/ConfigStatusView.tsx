import { Chip, FooterPill, StatusPill } from '../lib/terminal'
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
      <div className={`shui-slab ${applied ? 'accent' : 'alert'}`}>
        <div className="shui-row">
          <StatusPill
            label={resp.last_outcome}
            variant={applied ? 'accent' : 'alert'}
          />
          {resp.rejected_reloads > 0 ? (
            <FooterPill tone="warn">
              {`rejected reloads ${resp.rejected_reloads}`}
            </FooterPill>
          ) : (
            <Chip label="rejected reloads">{resp.rejected_reloads}</Chip>
          )}
        </div>
        {resp.last_error != null ? (
          <pre className="shui-pre err">
            <code>{resp.last_error}</code>
          </pre>
        ) : null}
      </div>
    </div>
  )
}
