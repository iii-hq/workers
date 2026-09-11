import { MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import {
  safeParseRequest,
  safeParseResponse,
  startRequestSchema,
  startResponseSchema,
} from './parsers'
import { DagSummary, GhostRow, RunIdRow } from './shared'

interface StartViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * `workflow::start` — fire-and-forget launch of a durable DAG. The request
 * carries the whole definition; the value here is rendering it AS a DAG
 * (nodes, deps, joins, fan-outs, output) instead of a wall of JSON. The
 * response is just the run handle.
 */
export function StartView({ input, output, running }: StartViewProps) {
  const req = safeParseRequest(startRequestSchema, input)
  if (!req) return null

  const resp = !running ? safeParseResponse(startResponseSchema, output) : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={running ? 'launching…' : 'launched'}
          variant={running ? 'default' : 'accent'}
        />
        {req.notify ? (
          <StatusPill
            label={`notify ${req.notify.function_id}`}
            variant="default"
          />
        ) : null}
      </MetaRow>
      {resp ? <RunIdRow runId={resp.run_id} /> : null}
      <DagSummary def={req.definition} />
      {running ? (
        <div className="px-3 py-2 font-mono text-[12px] text-ink-ghost animate-pulse">
          · registering run…
        </div>
      ) : resp ? null : (
        <GhostRow label="run id not returned" />
      )}
    </div>
  )
}
