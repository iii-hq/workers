import { Badge, Eyebrow, MetaRow } from '@iii-dev/console-ui'
import { nodeResultRequestSchema, nodeResultResponseSchema, safeParseRequest, safeParseResponse } from '../lib/parsers'
import { GhostRow, ResultPane, RunIdRow } from './shared'

interface NodeResultViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * `workflow::node-result` — partial-result recovery: fetch one node's stored
 * output by uid. Lead with which node, then its result verbatim (string
 * outputs render as text/markdown; JSON pretty-prints).
 */
export function NodeResultView({ input, output, running }: NodeResultViewProps) {
  const req = safeParseRequest(nodeResultRequestSchema, input)
  if (!req) return null

  return (
    <div className="wf-card">
      <MetaRow>
        <Badge className="wf-pill">{running ? 'fetching…' : 'node result'}</Badge>
        <span className="wf-meta">
          <span className="wf-ghost">Node</span> <span className="wf-accent wf-break">{req.node_uid}</span>
        </span>
      </MetaRow>
      <RunIdRow runId={req.run_id} label={<Eyebrow>run</Eyebrow>} />
      {running ? <GhostRow label={`fetching ${req.node_uid}…`} pulse /> : <NodeResultBody output={output} />}
    </div>
  )
}

function NodeResultBody({ output }: { output: unknown }) {
  const resp = safeParseResponse(nodeResultResponseSchema, output)
  if (!resp || resp.result == null) {
    return <GhostRow label="no result stored for this node" />
  }
  return <ResultPane label="result" value={resp.result} />
}
