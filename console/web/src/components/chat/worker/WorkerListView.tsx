import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import {
  safeParseRequest,
  safeParseResponse,
  type WorkerEntry,
  workerListRequestSchema,
  workerListResponseSchema,
} from './parsers'

interface WorkerListViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function WorkerListView({
  input,
  output,
  running,
}: WorkerListViewProps) {
  const req = safeParseRequest(workerListRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="listing…" variant="default" />
          {req?.running_only ? (
            <Chip>
              <span className="uppercase tracking-[0.06em] text-ink-faint">
                running only
              </span>
            </Chip>
          ) : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · enumerating workers…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(workerListResponseSchema, output)
  if (!resp) return null

  const runningCount = resp.workers.filter((w) => w.running).length
  const label =
    resp.workers.length === 0
      ? 'no workers'
      : `${resp.workers.length} ${
          resp.workers.length === 1 ? 'worker' : 'workers'
        } · ${runningCount} running`
  const pillVariant: 'accent' | 'warn' =
    resp.workers.length === 0 ? 'warn' : 'accent'

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label={label} variant={pillVariant} />
        {req?.running_only ? (
          <Chip>
            <span className="uppercase tracking-[0.06em] text-ink-faint">
              running only
            </span>
          </Chip>
        ) : null}
      </MetaRow>
      {resp.workers.length === 0 ? (
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · no workers found
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.workers.map((w) => (
            <WorkerRow key={w.name} entry={w} />
          ))}
        </ul>
      )}
    </div>
  )
}

function WorkerRow({ entry }: { entry: WorkerEntry }) {
  return (
    <li className="px-3 py-2 flex items-center justify-between gap-3 flex-wrap">
      <div className="flex items-baseline gap-2 flex-wrap min-w-0">
        <span className="font-mono text-[12.5px] text-accent break-all">
          {entry.name}
        </span>
        <RunningBadge running={entry.running} />
        {entry.version ? (
          <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint border border-rule-2 bg-paper-2 px-1.5 py-0.5">
            v{entry.version}
          </span>
        ) : null}
      </div>
      <div className="font-mono text-[11px] text-ink-faint tabular-nums">
        {typeof entry.pid === 'number'
          ? `pid ${entry.pid}`
          : entry.running
            ? 'pid —'
            : 'stopped'}
      </div>
    </li>
  )
}

function RunningBadge({ running }: { running: boolean }) {
  return (
    <span
      className={`font-mono text-[10px] uppercase tracking-[0.06em] border px-1.5 py-0.5 ${
        running
          ? 'text-accent border-accent/40 bg-paper-2'
          : 'text-ink-faint border-rule-2 bg-paper-2'
      }`}
    >
      {running ? 'running' : 'stopped'}
    </span>
  )
}
