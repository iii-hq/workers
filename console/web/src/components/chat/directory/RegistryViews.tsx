import type { ReactNode } from 'react'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
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
import { KvChip, MarkdownPane } from './shared'

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
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="searching…" variant="default" />
          {req?.search ? <KvChip label="search">{req.search}</KvChip> : null}
          {req?.cursor ? <KvChip label="cursor">·next page·</KvChip> : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · querying registry…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(registryWorkersListResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
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
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · no published workers match
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.workers.map((w) => (
            <li
              key={`${w.name}@${w.version ?? ''}`}
              className="px-3 py-2 flex flex-col gap-0.5"
            >
              <div className="flex items-baseline gap-2 flex-wrap">
                <span className="font-mono text-[12.5px] text-accent break-all">
                  {w.name}
                </span>
                {w.version ? (
                  <Chip>
                    <span className="text-ink">v{w.version}</span>
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
                  <Chip className="text-accent border-accent/40">
                    <span className="uppercase tracking-[0.06em]">
                      verified
                    </span>
                  </Chip>
                ) : null}
              </div>
              {w.description ? (
                <div className="font-mono text-[12px] text-ink-faint leading-[1.55]">
                  {w.description}
                </div>
              ) : null}
              <div className="font-mono text-[11px] text-ink-ghost flex flex-wrap gap-x-3 gap-y-0.5">
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
        <div className="px-3 py-1.5 bg-paper-2 border-t border-rule-2 font-mono text-[11px] text-ink-faint">
          next cursor available — pass back to fetch more
        </div>
      ) : null}
    </div>
  )
}

/* ---------------- directory::registry::workers::info ---------------- */

export function RegistryWorkerInfoView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(registryWorkerInfoRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? <KvChip label="worker">{req.name}</KvChip> : null}
          {req?.version ? <KvChip label="version">{req.version}</KvChip> : null}
          {req?.tag ? <KvChip label="tag">{req.tag}</KvChip> : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · fetching worker manifest…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(registryWorkerInfoResponseSchema, output)
  if (!resp) return null
  const { worker, readme, api_reference, skills_tree } = resp

  return (
    <div className="border-t border-rule-2 bg-bg">
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
          <Chip className="text-accent border-accent/40">
            <span className="uppercase tracking-[0.06em]">verified</span>
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <div className="flex flex-col">
          <span className="font-mono text-[13px] text-accent break-all">
            {worker.name}
          </span>
          {worker.description ? (
            <span className="font-mono text-[11.5px] text-ink-faint">
              {worker.description}
            </span>
          ) : null}
        </div>
      </ActionLine>
      <IdentityRow worker={worker} />
      <ApiReferenceSection api={api_reference} />
      <SkillsTreeSection tree={skills_tree} />
      {readme ? (
        <div className="border-b border-rule-2">
          <div className="px-3 py-1.5 bg-paper-2 border-b border-rule-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
            readme
          </div>
          <MarkdownPane body={readme} />
        </div>
      ) : null}
    </div>
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
    <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[11px] text-ink-faint flex flex-wrap gap-x-3 gap-y-0.5">
      {bits.map((b) => (
        <span key={b} className="break-all">
          {b}
        </span>
      ))}
    </div>
  )
}

function ApiReferenceSection({ api }: { api: ApiReferenceShape }) {
  const fns = api.functions ?? []
  const triggers = api.triggers ?? []
  if (fns.length === 0 && triggers.length === 0) return null
  return (
    <div className="border-b border-rule-2">
      <div className="px-3 py-1.5 bg-paper-2 border-b border-rule-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        api · {fns.length} fns · {triggers.length} triggers
      </div>
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
    <div className="border-b border-rule-2 last:border-b-0">
      <div className="px-3 py-1 bg-bg font-mono text-[10px] uppercase tracking-[0.06em] text-ink-ghost">
        {label}
      </div>
      <ul className="divide-y divide-rule-2">
        {items.map((it) => (
          <li key={it.name} className="px-3 py-1.5 flex flex-col gap-0.5">
            <span className="font-mono text-[12px] text-accent break-all">
              {it.name}
            </span>
            {it.description ? (
              <span className="font-mono text-[11.5px] text-ink-faint leading-[1.5]">
                {it.description}
              </span>
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
    <div className="border-b border-rule-2">
      <div className="px-3 py-1.5 bg-paper-2 border-b border-rule-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        skills tree · {skills.length} skills · {prompts.length} prompts
      </div>
      {skills.length > 0 ? (
        <ul className="divide-y divide-rule-2">
          {skills.map((s) => (
            <li
              key={s.path}
              className="px-3 py-1 font-mono text-[11.5px] text-ink break-all"
            >
              {s.path}
            </li>
          ))}
        </ul>
      ) : null}
      {prompts.length > 0 ? (
        <div>
          <div className="px-3 py-1 bg-bg font-mono text-[10px] uppercase tracking-[0.06em] text-ink-ghost">
            prompts
          </div>
          <ul className="divide-y divide-rule-2">
            {prompts.map((p) => (
              <li key={p.name} className="px-3 py-1 flex flex-col gap-0.5">
                <span className="font-mono text-[11.5px] text-accent break-all">
                  {p.name}
                </span>
                {p.description ? (
                  <span className="font-mono text-[11px] text-ink-faint leading-[1.5]">
                    {p.description}
                  </span>
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  )
}

function formatCount(n: number): string {
  if (n < 1000) return `${n}`
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`
  return `${(n / 1_000_000).toFixed(1)}M`
}
