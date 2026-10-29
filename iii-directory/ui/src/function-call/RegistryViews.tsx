import { ActionLine, Badge, Card, Chip, EmptyState, MarkdownPreview, MetaRow } from '@iii-dev/console-ui'
import { SquareFunction } from 'lucide-react'
import type { ReactNode } from 'react'
import { formatCount } from '../lib/format'
import { Identity, kv, Loading, Row, Rows, Section } from '../lib/widgets'
import {
  type ApiReferenceShape,
  registryWorkerInfoRequestSchema,
  registryWorkerInfoResponseSchema,
  registryWorkersListRequestSchema,
  registryWorkersListResponseSchema,
  type SkillsTreeShape,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

interface WorkerShape {
  version?: string | null
  type?: string | null
  total_downloads?: number | null
  repo?: string | null
  image?: string | null
  supported_targets?: string[]
  dependencies?: { name: string; version: string }[]
  author?: { name?: string | null; verified?: boolean } | null
}

/** Version, type, download count and the Verified mark. */
function workerChips(w: WorkerShape): ReactNode {
  return (
    <>
      {w.version ? <Chip>v{w.version}</Chip> : null}
      {w.type ? <Chip>{w.type}</Chip> : null}
      {typeof w.total_downloads === 'number' && w.total_downloads > 0 ? (
        <Chip>{formatCount(w.total_downloads)} downloads</Chip>
      ) : null}
      {w.author?.verified ? <Chip tone="accent">Verified</Chip> : null}
    </>
  )
}

/** Author, repo, image, targets and deps as label/value pairs, empties dropped. */
const workerFacts = (w: WorkerShape): [string, string][] =>
  (
    [
      ['by', w.author?.name],
      ['repo', w.repo],
      ['image', w.image],
      ['targets', w.supported_targets?.length ? w.supported_targets.join(', ') : null],
      ['deps', w.dependencies?.length ? w.dependencies.map((d) => `${d.name}@${d.version}`).join(', ') : null],
    ] as [string, string | null | undefined][]
  ).flatMap(([label, value]) => (value ? [[label, value]] : []))

/* ---------------- directory::registry::workers::list ---------------- */

export function RegistryWorkersListView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(registryWorkersListRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow
          items={kv([
            ['search', req?.search],
            ['cursor', req?.cursor && '·next page·'],
          ])}
        >
          <Badge>searching…</Badge>
        </MetaRow>
        <Loading label="querying registry…" />
      </Card>
    )
  }

  const resp = safeParseResponse(registryWorkersListResponseSchema, output)
  if (!resp) return null
  const n = resp.workers.length

  return (
    <Card>
      <MetaRow
        items={kv([
          ['search', req?.search],
          ['page', resp.pagination.page_size],
        ])}
      >
        <Badge variant={n === 0 ? 'warn' : 'accent'}>
          {`${n} ${n === 1 ? 'worker' : 'workers'}${resp.pagination.has_more ? ' · more' : ''}`}
        </Badge>
      </MetaRow>
      {n === 0 ? (
        <EmptyState title="No published workers match" description="The registry returned nothing for this search." />
      ) : (
        <Rows>
          {resp.workers.map((w) => (
            <Row
              key={`${w.name}@${w.version ?? ''}`}
              mono
              title={w.name}
              description={[w.description, ...workerFacts(w).map(([label, value]) => `${label} ${value}`)]
                .filter(Boolean)
                .join(' · ')}
              meta={workerChips(w)}
            />
          ))}
        </Rows>
      )}
      {resp.pagination.has_more && resp.pagination.next_cursor ? (
        <div className="dir-ui-empty">next cursor available — pass back to fetch more</div>
      ) : null}
    </Card>
  )
}

/* ---------------- directory::registry::workers::info ---------------- */

export function RegistryWorkerInfoView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(registryWorkerInfoRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow
          items={kv([
            ['worker', req?.name],
            ['version', req?.version],
            ['tag', req?.tag],
          ])}
        >
          <Badge>loading…</Badge>
        </MetaRow>
        <Loading label="fetching worker manifest…" />
      </Card>
    )
  }

  const resp = safeParseResponse(registryWorkerInfoResponseSchema, output)
  if (!resp) return null
  const { worker, readme, api_reference, skills_tree } = resp
  const facts = kv(workerFacts(worker))

  return (
    <Card>
      <MetaRow>
        <Badge variant="accent">worker</Badge>
        {workerChips(worker)}
      </MetaRow>
      <ActionLine icon={<SquareFunction />}>
        <Identity name={worker.name} description={worker.description} />
      </ActionLine>
      {facts.length > 0 ? <MetaRow items={facts} /> : null}
      <ApiReferenceSection api={api_reference} />
      <SkillsTreeSection tree={skills_tree} />
      {readme ? (
        <Section label="README">
          <MarkdownPreview markdown={readme} />
        </Section>
      ) : null}
    </Card>
  )
}

function ApiReferenceSection({ api }: { api: ApiReferenceShape }) {
  const fns = api.functions ?? []
  const triggers = api.triggers ?? []
  if (fns.length === 0 && triggers.length === 0) return null
  return (
    <Section label={`api · ${fns.length} fns · ${triggers.length} triggers`}>
      {renderRefList('functions', fns)}
      {renderRefList('triggers', triggers)}
    </Section>
  )
}

function renderRefList(label: string, items: { name: string; description?: string | null }[]): ReactNode {
  if (items.length === 0) return null
  return (
    <Section label={label}>
      <Rows>
        {items.map((it) => (
          <Row key={it.name} mono title={it.name} description={it.description || undefined} />
        ))}
      </Rows>
    </Section>
  )
}

function SkillsTreeSection({ tree }: { tree: SkillsTreeShape }) {
  const skills = tree.skills ?? []
  if (skills.length === 0) return null
  return (
    <Section label={`skills tree · ${skills.length} skills`}>
      <Rows>
        {skills.map((s) => (
          <Row key={s.path} mono title={s.path} />
        ))}
      </Rows>
    </Section>
  )
}
