import { Badge, MetaRow } from '@iii-dev/console-ui'
import { type StartRequest, safeParseResponse, startResponseSchema } from '../lib/parsers'
import { DagSummary, GhostRow, RunIdRow } from './shared'

interface StartViewProps {
  req: StartRequest
  output: unknown
  running?: boolean
}

/**
 * `workflow::start` — fire-and-forget launch of a durable DAG. The request
 * carries the whole definition; the value here is rendering it AS a DAG
 * (nodes, deps, joins, fan-outs, output) instead of a wall of JSON. The
 * response is just the run handle.
 */
export function StartView({ req, output, running }: StartViewProps) {
  const resp = !running ? safeParseResponse(startResponseSchema, output) : null

  return (
    <div className="wf-card">
      <MetaRow>
        <Badge className="wf-pill" variant={running ? 'default' : 'accent'}>
          {running ? 'launching…' : 'launched'}
        </Badge>
        {req.notify ? <Badge className="wf-pill">{`notify ${req.notify.function_id}`}</Badge> : null}
      </MetaRow>
      {resp ? <RunIdRow runId={resp.run_id} /> : null}
      <DagSummary def={req.definition} />
      {running ? <GhostRow label="registering run…" pulse /> : resp ? null : <GhostRow label="run id not returned" />}
    </div>
  )
}
