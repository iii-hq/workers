import { MarkdownPreview } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { formatCount } from '../lib/format'
import {
  ActionLine,
  Card,
  Chip,
  EmptyRow,
  KvChip,
  MetaRow,
  PulseLine,
  SectionHead,
  StatusPill,
  SubHead,
} from '../lib/widgets'
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

/* ---------------- directory::registry::workers::list ---------------- */

export function RegistryWorkersListView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(registryWorkersListRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="searching…" variant="default" />
          {req?.search ? <KvChip label="search">{req.search}</KvChip> : null}
          {req?.cursor ? <KvChip label="cursor">·next page·</KvChip> : null}
        </MetaRow>
        <PulseLine label="querying registry…" />
      </Card>
    )
  }

  const resp = safeParseResponse(registryWorkersListResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow>
        <StatusPill
          label={`${resp.workers.length} ${
            resp.workers.length === 1 ? 'worker' : 'workers'
          }${resp.pagination.has_more ? ' · more' : ''}`}
          variant={resp.workers.length === 0 ? 'warn' : 'accent'}
        />
        {req?.search ? <KvChip label="search">{req.search}</KvChip> : null}
        {resp.pagination.page_size ? (
          <KvChip label="page">{resp.pagination.page_size}</KvChip>
        ) : null}
      </MetaRow>
      {resp.workers.length === 0 ? (
        <EmptyRow label="no published workers match" />
      ) : (
        <ul className="dir-ui-list">
          {resp.workers.map((w) => (
            <li key={`${w.name}@${w.version ?? ''}`} className="dir-ui-row">
              <div className="dir-ui-row-head">
                <span className="dir-ui-id">{w.name}</span>
                {w.version ? (
                  <Chip>
                    <span className="v">v{w.version}</span>
                  </Chip>
                ) : null}
                {w.type ? <KvChip label="type">{w.type}</KvChip> : null}
                {typeof w.total_downloads === 'number' &&
                w.total_downloads > 0 ? (
                  <KvChip label="downloads">
                    {formatCount(w.total_downloads)}
                  </KvChip>
                ) : null}
                {w.author?.verified ? (
                  <Chip className="verified">verified</Chip>
                ) : null}
              </div>
              {w.description ? (
                <div className="dir-ui-desc">{w.description}</div>
              ) : null}
              <div className="dir-ui-fine wrap">
                {w.author?.name ? <span>by {w.author.name}</span> : null}
                {w.repo ? <span>{w.repo}</span> : null}
                {w.image ? <span>image: {w.image}</span> : null}
                {w.supported_targets && w.supported_targets.length > 0 ? (
                  <span>targets: {w.supported_targets.join(', ')}</span>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
      )}
      {resp.pagination.has_more && resp.pagination.next_cursor ? (
        <div className="dir-ui-next-note">
          next cursor available — pass back to fetch more
        </div>
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
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? <KvChip label="worker">{req.name}</KvChip> : null}
          {req?.version ? <KvChip label="version">{req.version}</KvChip> : null}
          {req?.tag ? <KvChip label="tag">{req.tag}</KvChip> : null}
        </MetaRow>
        <PulseLine label="fetching worker manifest…" />
      </Card>
    )
  }

  const resp = safeParseResponse(registryWorkerInfoResponseSchema, output)
  if (!resp) return null
  const { worker, readme, api_reference, skills_tree } = resp

  return (
    <Card>
      <MetaRow>
        <StatusPill label="worker" variant="accent" />
        {worker.version ? (
          <KvChip label="version">v{worker.version}</KvChip>
        ) : null}
        {worker.type ? <KvChip label="type">{worker.type}</KvChip> : null}
        {typeof worker.total_downloads === 'number' &&
        worker.total_downloads > 0 ? (
          <KvChip label="downloads">
            {formatCount(worker.total_downloads)}
          </KvChip>
        ) : null}
        {worker.author?.verified ? (
          <Chip className="verified">verified</Chip>
        ) : null}
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <div className="dir-ui-stack">
          <span className="dir-ui-id lg">{worker.name}</span>
          {worker.description ? (
            <span className="dir-ui-desc">{worker.description}</span>
          ) : null}
        </div>
      </ActionLine>
      <IdentityRow worker={worker} />
      <ApiReferenceSection api={api_reference} />
      <SkillsTreeSection tree={skills_tree} />
      {readme ? (
        <div className="dir-ui-section">
          <SectionHead>readme</SectionHead>
          <MarkdownPreview markdown={readme} />
        </div>
      ) : null}
    </Card>
  )
}

function IdentityRow({
  worker,
}: {
  worker: {
    repo?: string | null
    image?: string | null
    supported_targets?: string[]
    dependencies?: { name: string; version: string }[]
    author?: { name?: string | null; verified?: boolean } | null
  }
}) {
  const bits: string[] = []
  if (worker.author?.name) bits.push(`by ${worker.author.name}`)
  if (worker.repo) bits.push(worker.repo)
  if (worker.image) bits.push(`image: ${worker.image}`)
  if (worker.supported_targets && worker.supported_targets.length > 0) {
    bits.push(`targets: ${worker.supported_targets.join(', ')}`)
  }
  if (worker.dependencies && worker.dependencies.length > 0) {
    bits.push(
      `deps: ${worker.dependencies
        .map((d) => `${d.name}@${d.version}`)
        .join(', ')}`,
    )
  }
  if (bits.length === 0) return null
  return (
    <div className="dir-ui-identity">
      {bits.map((b) => (
        <span key={b}>{b}</span>
      ))}
    </div>
  )
}

function ApiReferenceSection({ api }: { api: ApiReferenceShape }) {
  const fns = api.functions ?? []
  const triggers = api.triggers ?? []
  if (fns.length === 0 && triggers.length === 0) return null
  return (
    <div className="dir-ui-section">
      <SectionHead>
        api · {fns.length} fns · {triggers.length} triggers
      </SectionHead>
      {renderRefList('functions', fns)}
      {renderRefList('triggers', triggers)}
    </div>
  )
}

function renderRefList(
  label: string,
  items: { name: string; description?: string | null }[],
): ReactNode {
  if (items.length === 0) return null
  return (
    <div className="dir-ui-section">
      <SubHead>{label}</SubHead>
      <ul className="dir-ui-list">
        {items.map((it) => (
          <li key={it.name} className="dir-ui-row tight">
            <span className="dir-ui-id sm">{it.name}</span>
            {it.description ? (
              <span className="dir-ui-desc">{it.description}</span>
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  )
}

function SkillsTreeSection({ tree }: { tree: SkillsTreeShape }) {
  const skills = tree.skills ?? []
  const prompts = tree.prompts ?? []
  if (skills.length === 0 && prompts.length === 0) return null
  return (
    <div className="dir-ui-section">
      <SectionHead>
        skills tree · {skills.length} skills · {prompts.length} prompts
      </SectionHead>
      {skills.length > 0 ? (
        <ul className="dir-ui-list">
          {skills.map((s) => (
            <li key={s.path} className="dir-ui-item">
              {s.path}
            </li>
          ))}
        </ul>
      ) : null}
      {prompts.length > 0 ? (
        <div>
          <SubHead>prompts</SubHead>
          <ul className="dir-ui-list">
            {prompts.map((p) => (
              <li key={p.name} className="dir-ui-row tight">
                <span className="dir-ui-id sm">{p.name}</span>
                {p.description ? (
                  <span className="dir-ui-desc">{p.description}</span>
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  )
}
