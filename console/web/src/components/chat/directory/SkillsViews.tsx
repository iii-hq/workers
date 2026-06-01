import type { ReactNode } from 'react'
import {
  ActionLine,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  type SkillsListRequest,
  safeParseRequest,
  safeParseResponse,
  skillsGetRequestSchema,
  skillsGetResponseSchema,
  skillsIndexResponseSchema,
  skillsListRequestSchema,
  skillsListResponseSchema,
} from './parsers'
import { formatBytes, formatRelativeTime, KvChip, MarkdownPane } from './shared'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* ---------------- directory::skills::list ---------------- */

export function SkillsListView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(skillsListRequestSchema, input)

  if (running) {
    return (
      <ListShell
        count={null}
        noun="skills"
        filters={<RequestFilters req={req ?? undefined} />}
        running
      />
    )
  }

  const resp = safeParseResponse(skillsListResponseSchema, output)
  if (!resp) return null

  return (
    <ListShell
      count={resp.skills.length}
      noun="skills"
      filters={<RequestFilters req={req ?? undefined} />}
    >
      {resp.skills.length === 0 ? (
        <EmptyRow label="no skills match" />
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.skills.map((s) => (
            <li key={s.id} className="px-3 py-2 flex flex-col gap-0.5">
              <div className="flex items-baseline gap-2 flex-wrap">
                <span className="font-mono text-[12.5px] text-accent break-all">
                  {s.id}
                </span>
                {s.type ? <KvChip label="type">{s.type}</KvChip> : null}
                {s.function_id ? (
                  <KvChip label="fn">{s.function_id}</KvChip>
                ) : null}
              </div>
              <div className="font-mono text-[12px] text-ink leading-[1.55]">
                {s.title}
              </div>
              {s.description ? (
                <div className="font-mono text-[11.5px] text-ink-faint leading-[1.55]">
                  {s.description}
                </div>
              ) : null}
              <div className="font-mono text-[11px] text-ink-ghost tabular-nums">
                {formatBytes(s.bytes)} · {formatRelativeTime(s.modified_at)}
              </div>
            </li>
          ))}
        </ul>
      )}
    </ListShell>
  )
}

/* ---------------- directory::skills::get ---------------- */

export function SkillsGetView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(skillsGetRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? <KvChip label="id">{req.id}</KvChip> : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · fetching skill…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(skillsGetResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="skill" variant="accent" />
        {resp.type ? <KvChip label="type">{resp.type}</KvChip> : null}
        {resp.function_id ? (
          <KvChip label="fn">{resp.function_id}</KvChip>
        ) : null}
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <div className="flex flex-col">
          <span className="font-mono text-[13px] text-accent break-all">
            {resp.id}
          </span>
          <span className="font-mono text-[11.5px] text-ink-faint">
            {resp.title}
          </span>
        </div>
      </ActionLine>
      <MarkdownPane body={resp.body} />
    </div>
  )
}

/* ---------------- directory::skills::index ---------------- */

export function SkillsIndexView({ output, running }: ViewProps) {
  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="indexing…" variant="default" />
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · building index…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(skillsIndexResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={`${resp.workers_count} ${
            resp.workers_count === 1 ? 'worker' : 'workers'
          }`}
          variant={resp.workers_count === 0 ? 'warn' : 'accent'}
        />
      </MetaRow>
      {resp.workers_count === 0 ? (
        <EmptyRow label="no workers indexed" />
      ) : (
        <MarkdownPane body={resp.body} />
      )}
    </div>
  )
}

/* ---------------- shared bits ---------------- */

interface ListShellProps {
  count: number | null
  noun: string
  filters?: ReactNode
  running?: boolean
  children?: ReactNode
}

function ListShell({
  count,
  noun,
  filters,
  running,
  children,
}: ListShellProps) {
  const label =
    running || count === null
      ? `listing ${noun}…`
      : count === 0
        ? `no ${noun} match`
        : `${count} ${count === 1 ? noun.replace(/s$/, '') : noun}`
  const pillVariant: 'accent' | 'warn' | 'default' = running
    ? 'default'
    : count === 0
      ? 'warn'
      : 'accent'
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label={label} variant={pillVariant} />
        {filters ? (
          <span className="flex flex-wrap items-center gap-1.5">{filters}</span>
        ) : null}
      </MetaRow>
      {running ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · scanning skills folder…
        </div>
      ) : (
        children
      )}
    </div>
  )
}

function EmptyRow({ label }: { label: string }) {
  return (
    <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
      · {label}
    </div>
  )
}

function RequestFilters({ req }: { req?: SkillsListRequest }) {
  if (!req) return null
  const chips: ReactNode[] = []
  if (req.prefix) {
    chips.push(
      <KvChip key="prefix" label="prefix">
        {req.prefix}
      </KvChip>,
    )
  }
  if (req.type) {
    chips.push(
      <KvChip key="type" label="type">
        {req.type}
      </KvChip>,
    )
  }
  if (req.search) {
    chips.push(
      <KvChip key="search" label="search">
        {req.search}
      </KvChip>,
    )
  }
  if (req.include_description === false) {
    chips.push(
      <KvChip key="no-desc" label="no description">
        on
      </KvChip>,
    )
  }
  return chips.length > 0 ? chips : null
}
