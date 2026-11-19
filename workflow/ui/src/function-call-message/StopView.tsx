import { Badge, MetaRow } from '@iii-dev/console-ui'
import { safeParseRequest, safeParseResponse, stopRequestSchema, stopResponseSchema } from '../lib/parsers'
import { RunIdRow, RunStatusPill } from './shared'

interface StopViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * `workflow::stop` — request cancellation of a run. Confirms the run handle
 * and the resulting status (the worker flips it to `cancelled`).
 */
export function StopView({ input, output, running }: StopViewProps) {
  const req = safeParseRequest(stopRequestSchema, input)
  if (!req) return null

  const resp = !running ? safeParseResponse(stopResponseSchema, output) : null

  return (
    <div className="wf-card">
      <MetaRow>
        {running ? (
          <Badge className="wf-pill">stopping…</Badge>
        ) : resp?.status ? (
          <RunStatusPill status={resp.status} />
        ) : resp?.stopped ? (
          <Badge className="wf-pill" variant="warn">
            stopped
          </Badge>
        ) : (
          <Badge className="wf-pill">stop requested</Badge>
        )}
      </MetaRow>
      <RunIdRow runId={resp?.run_id ?? req.run_id} />
    </div>
  )
}
