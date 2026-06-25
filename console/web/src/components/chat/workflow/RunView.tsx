import {
  ActionLine,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  runResponseSchema,
  safeParseRequest,
  safeParseResponse,
  startRequestSchema,
} from './parsers'
import { DagSummary, ResultPane, RunIdRow, RunStatusPill } from './shared'

interface RunViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * `workflow::run` — blocks until the run is terminal, then returns
 * `{ run_id, status, result | result_error }`. Same request shape as start, so
 * we render the DAG, then the terminal verdict + the deliverable (or error).
 */
export function RunView({ input, output, running }: RunViewProps) {
  const req = safeParseRequest(startRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="running…" variant="default" />
        </MetaRow>
        <DagSummary def={req.definition} />
        <div className="px-3 py-2 font-mono text-[12px] text-ink-ghost animate-pulse">
          · driving the dag to completion…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(runResponseSchema, output)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        {resp?.status ? (
          <RunStatusPill status={resp.status} />
        ) : (
          <StatusPill label="terminal" variant="default" />
        )}
      </MetaRow>
      {resp ? <RunIdRow runId={resp.run_id} /> : null}
      <DagSummary def={req.definition} />
      {resp?.result_error ? (
        <ActionLine symbol="✕" tone="warn">
          <span className="font-mono text-[12.5px] text-warn break-words">
            {resp.result_error}
          </span>
        </ActionLine>
      ) : null}
      {resp?.result != null ? (
        <ResultPane label="result" value={resp.result} />
      ) : null}
    </div>
  )
}
