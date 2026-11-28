/**
 * The model and the provider a conversation runs on, as the Chat shows them.
 * Discovered sessions (sub-agents, turns other clients sent through
 * `harness::send`) carry no `metadata.model`: their last reply's model and
 * provider stand in. The account chip, the limit card and Conversation
 * details' account row all read `effectiveProvider`, so the chip never opens
 * Details on a conversation the row doesn't recognise.
 */
import { parseCatalogModelKey } from '@/lib/catalog-model-key'
import {
  type AssistantMessage,
  CATALOG_MODEL_KEY_SEP,
  type Conversation,
  type Message,
  type ModelId,
} from '@/types/chat'

export type ConversationModelFields = Partial<
  Pick<Conversation, 'model' | 'messages' | 'sessionMetadata'>
>

/** Read from another tool (`external_source`): its replies don't choose a model here. */
export function importedConversation(c: ConversationModelFields): boolean {
  return typeof c.sessionMetadata?.external_source === 'string'
}

function lastReply(messages: readonly Message[]): AssistantMessage | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]
    if (m.role === 'assistant' && (m.model || m.provider)) return m
  }
  return null
}

/**
 * A model id as the catalog keys it: the reply's own provider first, then
 * any catalog key for that model id; the id itself when the catalog doesn't
 * have it.
 */
function toCatalog(
  model: string,
  catalog: readonly { id: string }[],
  provider?: string,
): ModelId {
  if (provider) {
    const own = `${provider}${CATALOG_MODEL_KEY_SEP}${model}`
    if (catalog.some((o) => o.id === own)) return own
  }
  return (
    catalog.find(
      (o) =>
        o.id === model || o.id.endsWith(`${CATALOG_MODEL_KEY_SEP}${model}`),
    )?.id ?? model
  )
}

/**
 * The model a conversation runs on: its own choice, else (not for an
 * imported one) the model its latest reply used, resolved against the
 * catalog's composite ids so the picker preselects when it can. An agent
 * profile may name a plain router id (`claude-opus-5-5`) rather than a
 * catalog key; it resolves the same way.
 */
export function effectiveModel(
  c: ConversationModelFields,
  catalog: readonly { id: string }[] = [],
): ModelId | null {
  if (c.model) return toCatalog(c.model, catalog)
  if (importedConversation(c)) return null
  const last = lastReply(c.messages ?? [])
  if (!last?.model) return null
  return toCatalog(last.model, catalog, last.provider)
}

/**
 * The provider of a conversation's next turn: its model's, else the
 * provider of its latest reply (which the transcript names even when the
 * catalog doesn't know the model), else the locked model's (a side chat).
 */
export function effectiveProvider(
  c: ConversationModelFields,
  {
    catalog = [],
    lockedModel,
  }: { catalog?: readonly { id: string }[]; lockedModel?: string | null } = {},
): string | null {
  if (!c.model && !importedConversation(c)) {
    const provider = lastReply(c.messages ?? [])?.provider
    if (provider) return provider
  }
  const model = effectiveModel(c, catalog) ?? lockedModel ?? null
  return model ? (parseCatalogModelKey(model)?.provider ?? null) : null
}
