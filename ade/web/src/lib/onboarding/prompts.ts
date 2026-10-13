/**
 * The example prompts the setup wizard's last step offers. The project's
 * template declares them in `onboarding.yaml` at the project root (the
 * harness template ships four); `console::onboarding::prompts` reads and
 * validates that file on every call, and answers `[]` when it is missing or
 * broken, so a project without one simply shows none.
 *
 * Each prompt names the agent profile it runs with and a priority list of
 * models: the first one this machine's catalog holds wins.
 */

import { parseCatalogModelKey } from '@/lib/catalog-model-key'
import { getIiiClient } from '@/lib/iii-client'
import { isMissingFunction } from '@/lib/secrets'

/** One `models` entry: a provider's model, and the reasoning effort to use. */
export interface PromptModel {
  /** Router provider id (`claude-code`, `anthropic`, `openai-codex`). */
  provider: string
  /**
   * The model's id, bare (`claude-sonnet-5-5`) or as the provider lists it
   * (`claude-code/claude-sonnet-5-5`).
   */
  model: string
  /** An ADE thinking level (`minimal` … `xhigh`, `off`); absent keeps the default. */
  effort?: string
}

export interface ExamplePrompt {
  title: string
  description?: string
  /** Agent profile id (`ade-worker-builder`, `default`). */
  agent: string
  /** The message the new chat sends. */
  prompt: string
  models: PromptModel[]
}

const PROMPTS_FUNCTION = 'console::onboarding::prompts'

function text(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

function promptModel(value: unknown): PromptModel | null {
  if (!value || typeof value !== 'object') return null
  const row = value as Record<string, unknown>
  const provider = text(row.provider)
  const model = text(row.model)
  if (!provider || !model) return null
  const effort = text(row.effort)
  return { provider, model, ...(effort ? { effort } : {}) }
}

/**
 * The backend's answer, read defensively: it validates already, but an
 * entry the wizard cannot use is dropped rather than rendered half.
 */
export function parseExamplePrompts(value: unknown): ExamplePrompt[] {
  const list =
    value && typeof value === 'object'
      ? (value as { prompts?: unknown }).prompts
      : undefined
  if (!Array.isArray(list)) return []
  const out: ExamplePrompt[] = []
  for (const raw of list) {
    if (!raw || typeof raw !== 'object') continue
    const row = raw as Record<string, unknown>
    const title = text(row.title)
    const prompt = text(row.prompt)
    const agent = text(row.agent)
    if (!title || !prompt || !agent) continue
    const description = text(row.description)
    const models = Array.isArray(row.models)
      ? row.models
          .map(promptModel)
          .filter((entry): entry is PromptModel => entry !== null)
      : []
    out.push({
      title,
      ...(description ? { description } : {}),
      agent,
      prompt,
      models,
    })
  }
  return out
}

/**
 * The project's example prompts; `[]` when the ADE backend predates them or
 * the project declares none.
 */
export async function fetchExamplePrompts(): Promise<ExamplePrompt[]> {
  const client = await getIiiClient()
  try {
    const result = await client.trigger<unknown>(
      PROMPTS_FUNCTION,
      {},
      { timeoutMs: 5_000 },
    )
    return parseExamplePrompts(result)
  } catch (error) {
    if (isMissingFunction(error)) return []
    throw error
  }
}

/** Whether a catalog row (`provider` + its model `id`) is this entry's model. */
export function matchesPromptModel(
  entry: PromptModel,
  row: { provider: string; id: string },
): boolean {
  return (
    row.provider === entry.provider &&
    (row.id === entry.model || row.id.endsWith(`/${entry.model}`))
  )
}

/**
 * The first entry in `entries` this machine can run, as the composer's model
 * (a `provider::id` catalog key) and the entry's effort; `null` when none is
 * in the catalog, so the chat keeps its usual default.
 */
export function resolvePromptModel(
  entries: readonly PromptModel[],
  catalogKeys: readonly string[],
): { model: string; effort?: string } | null {
  const rows = catalogKeys.flatMap((key) => {
    const row = parseCatalogModelKey(key)
    return row ? [{ key, ...row }] : []
  })
  for (const entry of entries) {
    const row = rows.find((candidate) => matchesPromptModel(entry, candidate))
    if (row) {
      return {
        model: row.key,
        ...(entry.effort ? { effort: entry.effort } : {}),
      }
    }
  }
  return null
}
