import { ActionLine, MetaRow } from '@iii-dev/console-ui'
import { uiClasses } from '@iii-dev/console-ui/ui-classes'
import { X } from 'lucide-react'
import {
  type NodeState,
  safeParseRequest,
  safeParseResponse,
  statusRequestSchema,
  statusResponseSchema,
  tallyNodes,
} from '../lib/parsers'
import { GhostRow, NodeStateDot, ProgressBar, ResultPane, RunIdRow, RunStatusPill, WfChip } from './shared'

interface StatusViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/** Order node uids for the board: by base id alphabetically, then fanned `#i`
 *  numerically (so `read#2` precedes `read#10`, never lexical). */
function compareUids(a: string, b: string): number {
  const [ba, ia] = splitUid(a)
  const [bb, ib] = splitUid(b)
  if (ba !== bb) return ba < bb ? -1 : 1
  return ia - ib
}

function splitUid(uid: string): [string, number] {
  const hash = uid.indexOf('#')
  if (hash === -1) return [uid, -1]
  return [uid.slice(0, hash), Number(uid.slice(hash + 1)) || 0]
}

/**
 * `workflow::status` — the progress board. A run can be inspected at any point;
 * this is the most-watched workflow view, so it leads with the run verdict, a
 * completion bar, then a per-node state list with inline failures. Far more
 * legible than the raw `{ nodes: { … } }` map.
 */
export function StatusView({ input, output, running }: StatusViewProps) {
  const req = safeParseRequest(statusRequestSchema, input)
  const resp = !running ? safeParseResponse(statusResponseSchema, output) : null

  const runId = resp?.run_id ?? req?.run_id

  if (running) {
    return (
      <div className="wf-card">
        <MetaRow>
          <span className={`wf-meta wf-ghost ${uiClasses.pulse}`}>reading run state…</span>
        </MetaRow>
        {runId ? <RunIdRow runId={runId} /> : null}
      </div>
    )
  }

  if (!resp) {
    return (
      <div className="wf-card">
        {runId ? <RunIdRow runId={runId} /> : null}
        <GhostRow label="status payload could not be read" />
      </div>
    )
  }

  const counts = tallyNodes(resp.nodes)
  const nodeErrors = resp.node_errors ?? {}
  const nodeEntries = Object.entries(resp.nodes).sort((a, b) => compareUids(a[0], b[0]))

  return (
    <div className="wf-card">
      <MetaRow>
        {resp.status ? <RunStatusPill status={resp.status} /> : null}
        <WfChip>
          {/* One child: the chip's gap would split the count from its total. */}
          <span>
            <span className="wf-num wf-ink">{counts.done}</span>
            <span className="wf-faint">/{counts.total} done</span>
          </span>
        </WfChip>
        {counts.running > 0 ? (
          <WfChip>
            <span className="wf-num wf-accent">{counts.running}</span>
            <span className="wf-faint">Running</span>
          </WfChip>
        ) : null}
        {counts.failed > 0 ? (
          <WfChip className="wf-alert">
            <span className="wf-num">{counts.failed}</span>
            <span>Failed</span>
          </WfChip>
        ) : null}
      </MetaRow>
      {runId ? <RunIdRow runId={runId} /> : null}
      <ProgressBar done={counts.done} running={counts.running} failed={counts.failed} total={counts.total} />
      {nodeEntries.length === 0 ? (
        <GhostRow label="no nodes materialized yet" />
      ) : (
        <ul className="wf-rows">
          {nodeEntries.map(([uid, state]) => (
            <NodeRow key={uid} uid={uid} state={state} error={nodeErrors[uid]} />
          ))}
        </ul>
      )}
      {resp.result_error ? (
        <ActionLine icon={<X />} tone="warn">
          <span className="wf-error">{resp.result_error}</span>
        </ActionLine>
      ) : null}
      {resp.result != null ? <ResultPane label="result" value={resp.result} /> : null}
    </div>
  )
}

function NodeRow({ uid, state, error }: { uid: string; state: NodeState; error?: string }) {
  return (
    <li className="wf-row">
      <NodeStateDot state={state} />
      <div className="wf-row-body">
        <span className="wf-node-id">{uid}</span>
        {error ? <div className="wf-edges wf-node-error">{error}</div> : null}
      </div>
    </li>
  )
}
