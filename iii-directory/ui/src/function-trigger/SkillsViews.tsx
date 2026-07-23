import { MarkdownPreview } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { formatBytes, formatRelativeTime } from '../lib/format'
import {
  ActionLine,
  Card,
  EmptyRow,
  KvChip,
  MetaRow,
  PulseLine,
  StatusPill,
} from '../lib/widgets'
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
        <ul className="dir-ui-list">
          {resp.skills.map((s) => (
            <li key={s.id} className="dir-ui-row">
              <div className="dir-ui-row-head">
                <span className="dir-ui-id">{s.id}</span>
                {s.type ? <KvChip label="type">{s.type}</KvChip> : null}
                {s.function_id ? (
                  <KvChip label="fn">{s.function_id}</KvChip>
                ) : null}
              </div>
              <div className="dir-ui-title">{s.title}</div>
              {s.description ? (
                <div className="dir-ui-desc">{s.description}</div>
              ) : null}
              <div className="dir-ui-fine">
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
      <Card>
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? <KvChip label="id">{req.id}</KvChip> : null}
        </MetaRow>
        <PulseLine label="fetching skill…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsGetResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow>
        <StatusPill label="skill" variant="accent" />
        {resp.type ? <KvChip label="type">{resp.type}</KvChip> : null}
        {resp.function_id ? (
          <KvChip label="fn">{resp.function_id}</KvChip>
        ) : null}
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <div className="dir-ui-stack">
          <span className="dir-ui-id lg">{resp.id}</span>
          <span className="dir-ui-desc">{resp.title}</span>
        </div>
      </ActionLine>
      <MarkdownPreview markdown={resp.body} />
    </Card>
  )
}

/* ---------------- directory::skills::index ---------------- */

export function SkillsIndexView({ output, running }: ViewProps) {
  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="indexing…" variant="default" />
        </MetaRow>
        <PulseLine label="building index…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsIndexResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
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
        <MarkdownPreview markdown={resp.body} />
      )}
    </Card>
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

function ListShell({ count, noun, filters, running, children }: ListShellProps) {
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
    <Card>
      <MetaRow>
        <StatusPill label={label} variant={pillVariant} />
        {filters ? <span className="dir-ui-filters">{filters}</span> : null}
      </MetaRow>
      {running ? <PulseLine label={`scanning ${noun} folder…`} /> : children}
    </Card>
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
