import { MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import {
  safeParseRequest,
  safeParseResponse,
  stopRequestSchema,
  stopResponseSchema,
} from './parsers'
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
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        {running ? (
          <StatusPill label="stopping…" variant="default" />
        ) : resp?.status ? (
          <RunStatusPill status={resp.status} />
        ) : resp?.stopped ? (
          <StatusPill label="stopped" variant="warn" />
        ) : (
          <StatusPill label="stop requested" variant="default" />
        )}
      </MetaRow>
      <RunIdRow runId={resp?.run_id ?? req.run_id} />
    </div>
  )
}
