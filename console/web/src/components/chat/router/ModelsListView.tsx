import type { ReactNode } from 'react'
import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import {
  formatTokens,
  modelsListRequestSchema,
  modelsListResponseSchema,
  type RouterModel,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ModelsListViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/** The capability flags worth surfacing, in display order. Only the `true`
 *  ones render, so a sparse model stays quiet. */
const CAPABILITIES: Array<{ key: keyof RouterModel; label: string }> = [
  { key: 'supports_thinking', label: 'thinking' },
  { key: 'supports_xhigh', label: 'xhigh' },
  { key: 'supports_tools', label: 'tools' },
  { key: 'supports_vision', label: 'vision' },
  { key: 'supports_cache', label: 'cache' },
  { key: 'supports_structured_output', label: 'json' },
]

/**
 * `router::models::list` — the model catalog. Read-only, so it leads with a
 * count, then groups models by provider with per-model context window, output
 * cap, capability flags, and pricing. Far more scannable than the raw
 * `{ models: [ … ] }` array.
 */
export function ModelsListView({
  input,
  output,
  running,
}: ModelsListViewProps) {
  const req = safeParseRequest(modelsListRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            listing models…
          </span>
          <RequestFilters
            provider={req?.provider}
            capability={req?.capability}
          />
        </MetaRow>
      </div>
    )
  }

  const resp = safeParseResponse(modelsListResponseSchema, output)
  if (!resp) return null

  const groups = groupByProvider(resp.models)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={
            resp.models.length === 0
              ? 'no models'
              : `${resp.models.length} ${resp.models.length === 1 ? 'model' : 'models'}`
          }
          variant={resp.models.length === 0 ? 'warn' : 'accent'}
        />
        <RequestFilters provider={req?.provider} capability={req?.capability} />
      </MetaRow>
      {groups.length === 0 ? (
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · catalog is empty
        </div>
      ) : (
        groups.map(([provider, models]) => (
          <div
            key={provider}
            className="border-b border-rule-2 last:border-b-0"
          >
            <div className="flex items-baseline gap-2 px-3 py-1.5 bg-paper-2 border-b border-rule-2">
              <span className="font-mono text-[11px] text-ink-faint uppercase tracking-[0.06em]">
                {provider}
              </span>
              <span className="font-mono text-[11px] text-ink-ghost tabular-nums">
                {models.length}
              </span>
            </div>
            <ul className="divide-y divide-rule-2">
              {models.map((m) => (
                <ModelRow key={`${m.provider}/${m.id}`} model={m} />
              ))}
            </ul>
          </div>
        ))
      )}
    </div>
  )
}

function ModelRow({ model }: { model: RouterModel }) {
  const ctx = formatTokens(model.context_window)
  const out = formatTokens(model.max_output_tokens)
  return (
    <li className="px-3 py-2 flex flex-col gap-1">
      <div className="flex items-baseline gap-2 flex-wrap">
        <span className="font-mono text-[12.5px] text-accent break-all">
          {model.display_name ?? model.id}
        </span>
        {model.display_name ? (
          <span className="font-mono text-[11px] text-ink-faint break-all">
            {model.id}
          </span>
        ) : null}
      </div>
      <div className="flex items-baseline gap-2 flex-wrap font-mono text-[11px] text-ink-faint">
        {ctx ? <span>ctx {ctx}</span> : null}
        {out ? <span>· out {out}</span> : null}
        {CAPABILITIES.map((c) =>
          model[c.key] === true ? <Chip key={c.label}>{c.label}</Chip> : null,
        )}
      </div>
      <Pricing model={model} />
    </li>
  )
}

/** Per-Mtok pricing line — only renders the figures the provider actually set. */
function Pricing({ model }: { model: RouterModel }) {
  const p = model.pricing
  if (!p) return null
  const parts: string[] = []
  if (typeof p.input === 'number') parts.push(`$${p.input} in`)
  if (typeof p.output === 'number') parts.push(`$${p.output} out`)
  const cache: string[] = []
  if (typeof p.cache_read === 'number') cache.push(`$${p.cache_read} r`)
  if (typeof p.cache_write === 'number') cache.push(`$${p.cache_write} w`)
  if (parts.length === 0 && cache.length === 0) return null
  return (
    <div className="font-mono text-[11px] text-ink-ghost">
      {parts.join(' · ')}
      {parts.length > 0 ? ' ' : ''}
      <span className="text-ink-faint">per Mtok</span>
      {cache.length > 0 ? ` · cache ${cache.join(' / ')}` : null}
    </div>
  )
}

function RequestFilters({
  provider,
  capability,
}: {
  provider?: string | null
  capability?: string | null
}): ReactNode {
  const chips: ReactNode[] = []
  if (provider)
    chips.push(<FilterChip key="provider" label="provider" value={provider} />)
  if (capability)
    chips.push(
      <FilterChip key="capability" label="capability" value={capability} />,
    )
  return chips.length > 0 ? (
    <span className="flex flex-wrap items-center gap-1.5">{chips}</span>
  ) : null
}

function FilterChip({ label, value }: { label: string; value: ReactNode }) {
  return (
    <Chip>
      <span className="text-ink-faint uppercase tracking-[0.06em]">
        {label}
      </span>
      <span className="ml-1 text-ink">{value}</span>
    </Chip>
  )
}

/** Group models by provider, providers sorted alphabetically and each group's
 *  models sorted by display name (falling back to id). */
function groupByProvider(
  models: RouterModel[],
): Array<[string, RouterModel[]]> {
  const map = new Map<string, RouterModel[]>()
  for (const m of models) {
    const list = map.get(m.provider)
    if (list) list.push(m)
    else map.set(m.provider, [m])
  }
  return [...map.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([provider, list]) => [
      provider,
      [...list].sort((a, b) =>
        (a.display_name ?? a.id).localeCompare(b.display_name ?? b.id),
      ),
    ])
}
