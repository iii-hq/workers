import { ActionLine, Chip, MetaRow } from '@/components/chat/sandbox/shared'
import {
  type NodeCheckpoint,
  safeParseRequest,
  safeParseResponse,
  statusRequestSchema,
  statusResponseSchema,
  tallyNodes,
} from './parsers'
import {
  GhostRow,
  NodeStateDot,
  ProgressBar,
  ResultPane,
  RunIdRow,
  RunStatusPill,
} from './shared'

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
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            reading run state…
          </span>
        </MetaRow>
        {runId ? <RunIdRow runId={runId} /> : null}
      </div>
    )
  }

  if (!resp) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        {runId ? <RunIdRow runId={runId} /> : null}
        <GhostRow label="status payload could not be read" />
      </div>
    )
  }

  const counts = tallyNodes(resp.nodes)
  const nodeEntries = Object.entries(resp.nodes).sort((a, b) =>
    compareUids(a[0], b[0]),
  )

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        {resp.status ? <RunStatusPill status={resp.status} /> : null}
        <Chip>
          <span className="text-ink tabular-nums">{counts.done}</span>
          <span className="text-ink-faint">/{counts.total} done</span>
        </Chip>
        {counts.running > 0 ? (
          <Chip>
            <span className="text-accent tabular-nums">{counts.running}</span>
            <span className="text-ink-faint ml-1">running</span>
          </Chip>
        ) : null}
        {counts.failed > 0 ? (
          <Chip className="text-alert">
            <span className="tabular-nums">{counts.failed}</span>
            <span className="ml-1">failed</span>
          </Chip>
        ) : null}
      </MetaRow>
      {runId ? <RunIdRow runId={runId} /> : null}
      <ProgressBar
        done={counts.done}
        running={counts.running}
        failed={counts.failed}
        total={counts.total}
      />
      {nodeEntries.length === 0 ? (
        <GhostRow label="no nodes materialized yet" />
      ) : (
        <ul className="divide-y divide-rule-2">
          {nodeEntries.map(([uid, cp]) => (
            <NodeRow key={uid} uid={uid} cp={cp} />
          ))}
        </ul>
      )}
      {resp.result_error ? (
        <ActionLine symbol="✕" tone="warn">
          <span className="font-mono text-[12.5px] text-warn break-words">
            {resp.result_error}
          </span>
        </ActionLine>
      ) : null}
      {resp.result != null ? (
        <ResultPane label="result" value={resp.result} />
      ) : null}
    </div>
  )
}

function NodeRow({ uid, cp }: { uid: string; cp: NodeCheckpoint }) {
  return (
    <li className="flex items-start gap-2 px-3 py-1.5">
      <NodeStateDot state={cp.state} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="font-mono text-[12.5px] text-ink break-all">
            {uid}
          </span>
          {typeof cp.retries === 'number' && cp.retries > 0 ? (
            <Chip className="text-warn">retry {cp.retries}</Chip>
          ) : null}
        </div>
        {cp.result_error ? (
          <div className="mt-0.5 font-mono text-[11px] text-warn break-words">
            {cp.result_error}
          </div>
        ) : null}
      </div>
    </li>
  )
}
