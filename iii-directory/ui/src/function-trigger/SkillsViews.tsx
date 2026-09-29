import { ActionLine, Badge, Card, EmptyState, MarkdownPreview, MetaRow } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { SquareFunction } from 'lucide-react'
import { ago } from '../lib/format'
import { Identity, kv, Loading, Row, Rows } from '../lib/widgets'
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

const requestFilters = (req: SkillsListRequest | null) =>
  kv([
    ['prefix', req?.prefix],
    ['type', req?.type],
    ['search', req?.search],
    ['no description', req?.include_description === false && 'on'],
  ])

export function SkillsListView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(skillsListRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow items={requestFilters(req)}>
          <Badge>listing skills…</Badge>
        </MetaRow>
        <Loading label="scanning skills folder…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsListResponseSchema, output)
  if (!resp) return null
  const n = resp.skills.length

  return (
    <Card>
      <MetaRow items={requestFilters(req)}>
        <Badge variant={n === 0 ? 'warn' : 'accent'}>
          {n === 0 ? 'no skills match' : `${n} ${n === 1 ? 'skill' : 'skills'}`}
        </Badge>
      </MetaRow>
      {n === 0 ? (
        <EmptyState title="No skills match" description="Nothing in the skills folder matches these filters." />
      ) : (
        <Rows>
          {resp.skills.map((s) => (
            <Row
              key={s.id}
              mono
              title={s.id}
              description={[s.title, s.description].filter(Boolean).join(' · ')}
              meta={
                <>
                  {s.type ? <span>type {s.type}</span> : null}
                  {s.function_id ? <span>fn {s.function_id}</span> : null}
                  <span>
                    {formatBytes(s.bytes)} · {ago(s.modified_at)}
                  </span>
                </>
              }
            />
          ))}
        </Rows>
      )}
    </Card>
  )
}

/* ---------------- directory::skills::get ---------------- */

export function SkillsGetView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(skillsGetRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow items={kv([['id', req?.id]])}>
          <Badge>loading…</Badge>
        </MetaRow>
        <Loading label="fetching skill…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsGetResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow
        items={kv([
          ['type', resp.type],
          ['fn', resp.function_id],
          ['modified', ago(resp.modified_at)],
        ])}
      >
        <Badge variant="accent">skill</Badge>
      </MetaRow>
      <ActionLine icon={<SquareFunction />}>
        <Identity name={resp.id} description={resp.title} />
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
          <Badge>indexing…</Badge>
        </MetaRow>
        <Loading label="building index…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsIndexResponseSchema, output)
  if (!resp) return null
  const n = resp.workers_count

  return (
    <Card>
      <MetaRow>
        <Badge variant={n === 0 ? 'warn' : 'accent'}>{`${n} ${n === 1 ? 'worker' : 'workers'}`}</Badge>
      </MetaRow>
      {n === 0 ? (
        <EmptyState title="No workers indexed" description="No registered worker contributed skills to the index." />
      ) : (
        <MarkdownPreview markdown={resp.body} />
      )}
    </Card>
  )
}
