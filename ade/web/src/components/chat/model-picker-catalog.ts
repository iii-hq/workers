/**
 * The catalog as the pickers list it: models grouped under the provider
 * that serves them, in provider order, with the providers that have no
 * chat models yet kept in the list while they still need setup. Shared by
 * the composer's model menu (`PinnedModelMenu`, its "All models" view) and
 * the sheet page (`ModelPickerPanel`), so the two never disagree about
 * which providers show and why.
 */

import { useEffect, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import {
  discoveryBlocksModels,
  type ProviderListEntry,
} from '@/lib/models-catalog'
import type { ModelOption } from '@/types/chat'
import {
  formatModelLabel,
  formatProviderLabel,
  providerForModel,
} from './model-picker-presentation'

export interface ProviderGroup {
  /** The provider id (`anthropic`, `openai-codex`); `—` for a model without one. */
  label: string
  options: ModelOption[]
}

export function groupByProvider(
  options: readonly ModelOption[],
): ProviderGroup[] {
  const byProvider = new Map<string, ModelOption[]>()
  for (const opt of options) {
    const provider = providerForModel(opt.id) ?? '—'
    const bucket = byProvider.get(provider) ?? []
    bucket.push(opt)
    byProvider.set(provider, bucket)
  }
  return [...byProvider.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([label, opts]) => ({ label, options: opts }))
}

export function providerDisplayName(
  provider: ProviderListEntry | undefined,
  id: string,
): string {
  return provider?.display_name ?? formatProviderLabel(id) ?? id
}

/** Every word of the filter appears in the model's name, id or provider. */
export function modelMatchesWords(
  option: ModelOption,
  providerLabel: string,
  words: readonly string[],
): boolean {
  if (words.length === 0) return true
  const hay =
    `${formatModelLabel(option.label)} ${option.id} ${providerLabel}`.toLowerCase()
  return words.every((word) => hay.includes(word))
}

export function filterWordsOf(filter: string): string[] {
  return filter.toLowerCase().split(/\s+/).filter(Boolean)
}

/**
 * The groups a picker renders for `filter`: each provider's matching
 * models, plus the model-less providers that still need attention.
 *
 * A provider with no chat models is listed only while it still needs
 * setup: a configured one without chat models usually serves another
 * modality (speech, embeddings) and has nothing to offer this picker. One
 * that lists no models at all (`modelless`), or whose credential failed,
 * is broken rather than elsewhere — it stays, so its key can be fixed from
 * here. With a filter typed, only groups with a match survive (xAI keeps
 * its discovery notice while the filter spells its name).
 */
export function buildProviderGroups(args: {
  options: readonly ModelOption[]
  providers: readonly ProviderListEntry[]
  modelless: ReadonlySet<string>
  words: readonly string[]
}): ProviderGroup[] {
  const { options, providers, modelless, words } = args
  const providerById = new Map(providers.map((p) => [p.id, p]))
  const modelGroups = groupByProvider(options)
  const grouped = new Set(modelGroups.map((group) => group.label))
  return [
    ...modelGroups.map((group) => ({
      label: group.label,
      options: group.options.filter((option) =>
        modelMatchesWords(
          option,
          providerDisplayName(providerById.get(group.label), group.label),
          words,
        ),
      ),
    })),
    ...providers
      .map((provider) => provider.id)
      .filter((id) => !grouped.has(id))
      .filter(
        (id) =>
          providerById.get(id)?.configured !== true ||
          providerById.get(id)?.credential_error !== undefined ||
          (id === 'xai' && providerById.get(id)?.discovery !== undefined) ||
          modelless.has(id),
      )
      .map((id) => ({ label: id, options: [] })),
  ]
    .filter(
      (group) =>
        words.length === 0 ||
        group.options.length > 0 ||
        (group.label === 'xai' &&
          providerById.get('xai')?.discovery !== undefined &&
          words.every((word) => 'xai'.includes(word))),
    )
    .sort((a, b) => a.label.localeCompare(b.label))
}

export interface ProviderGroupState {
  providerLabel: string
  /** The worker is not loaded, or discovery blocks its models. */
  unavailable: boolean
  hasModels: boolean
  configured: boolean
  /** The models can be picked from (configured, or the provider owns its auth). */
  catalogIsUsable: boolean
}

/** What a group's header and body should say about its provider. */
export function providerGroupState(
  group: ProviderGroup,
  provider: ProviderListEntry | undefined,
): ProviderGroupState {
  const hasModels = group.options.length > 0
  const configured = provider?.configured ?? hasModels
  // OAuth/companion-app providers can be authenticated even though the
  // router slice has no API key and reports `configured: false`. A
  // successfully discovered catalog is authoritative for them.
  const providerOwnsAuthentication =
    provider !== undefined && provider.credential_env_var === undefined
  return {
    providerLabel: providerDisplayName(provider, group.label),
    unavailable:
      provider?.available === false || discoveryBlocksModels(provider),
    hasModels,
    configured,
    catalogIsUsable: hasModels && (configured || providerOwnsAuthentication),
  }
}

/**
 * Of `candidates` — configured providers with no chat models — the ones that
 * list no models of any modality. A speech or embedding provider lists its
 * own; one that lists nothing has a credential the upstream refused.
 */
export function useModellessProviders(
  candidates: string[],
): ReadonlySet<string> {
  const [modelless, setModelless] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const key = candidates.slice().sort().join(' ')
  useEffect(() => {
    const ids = key ? key.split(' ') : []
    if (ids.length === 0) {
      setModelless(new Set())
      return
    }
    let cancelled = false
    void (async () => {
      try {
        const client = await getIiiClient()
        const result = await client.trigger<{ models?: unknown }>(
          'router::models::list',
          { modality: 'any' },
        )
        const listed = new Set(
          (Array.isArray(result?.models) ? result.models : []).flatMap(
            (row) => {
              const provider = (row as { provider?: unknown } | null)?.provider
              return typeof provider === 'string' ? [provider] : []
            },
          ),
        )
        if (!cancelled) {
          setModelless(new Set(ids.filter((id) => !listed.has(id))))
        }
      } catch {
        if (!cancelled) setModelless(new Set())
      }
    })()
    return () => {
      cancelled = true
    }
  }, [key])
  return modelless
}

/** The configured providers that list no chat models — the modelless probe's input. */
export function modellessCandidates(
  providers: readonly ProviderListEntry[],
  options: readonly ModelOption[],
): string[] {
  const grouped = new Set(groupByProvider(options).map((group) => group.label))
  return providers
    .filter((provider) => provider.configured === true)
    .map((provider) => provider.id)
    .filter((id) => !grouped.has(id))
}
