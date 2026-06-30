import { MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import {
  nodeResultRequestSchema,
  nodeResultResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'
import { GhostRow, ResultPane } from './shared'

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
export function NodeResultView({
  input,
  output,
  running,
}: NodeResultViewProps) {
  const req = safeParseRequest(nodeResultRequestSchema, input)
  if (!req) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={running ? 'fetching…' : 'node result'}
          variant="default"
        />
        <span className="font-mono text-[11px] text-ink-faint">
          <span className="text-ink-ghost">node</span>{' '}
          <span className="text-accent break-all">{req.node_uid}</span>
        </span>
      </MetaRow>
      <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[11px] text-ink-faint break-all">
        <span className="uppercase tracking-[0.06em] text-[10px] mr-1">
          run
        </span>
        <span className="text-ink select-all">{req.run_id}</span>
      </div>
      {running ? (
        <div className="px-3 py-2 font-mono text-[12px] text-ink-ghost animate-pulse">
          {`· fetching ${req.node_uid}…`}
        </div>
      ) : (
        <NodeResultBody output={output} />
      )}
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
