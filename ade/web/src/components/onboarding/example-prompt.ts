import { agentProfileFromEntry } from '@/components/chat/agent-defaults'
import { effortOptionsFor } from '@/components/chat/ModelPicker'
import type { ConversationsApi } from '@/hooks/use-conversations'
import { type AgentEntry, listAgents } from '@/lib/backend/directory-prompts'
import { sendDraftWhenReady } from '@/lib/composer-insert'
import { getIiiClient } from '@/lib/iii-client'
import {
  type ExamplePrompt,
  resolvePromptModel,
} from '@/lib/onboarding/prompts'
import { fetchNewChatWorkingDir } from '@/lib/working-dir'
import { type ModelOption, THINKING_LOWEST, type ThinkingLevel } from '@/types/chat'

/** What opening an example prompt needs from the conversations store. */
export type PromptChatApi = Pick<
  ConversationsApi,
  | 'createNew'
  | 'prefillWorkingDir'
  | 'setAgentProfile'
  | 'setModel'
  | 'setThinkingLevel'
> & {
  /** Select the chat and bring the chat pane on screen. */
  openConversation: (id: string) => void
  /** The live model catalog (`provider::id` keys). */
  modelOptions: readonly ModelOption[]
}

/** The model offers this effort, and it is not only shown as unconfirmed;
 * `lowest` is the harness's to resolve, so any model takes it. */
function effortSupported(
  model: ModelOption | undefined,
  effort: ThinkingLevel,
): boolean {
  if (effort === THINKING_LOWEST) return true
  return effortOptionsFor(model).some(
    (option) => option.effort === effort && !option.disabled,
  )
}

/**
 * A new chat for an example prompt, sent as soon as the chat can send it
 * (with a model): it works in `workingDir` (the folder a new chat starts
 * in, set here because the send does not wait for the chat's own lookup),
 * the prompt's agent profile is selected, and the first model in its
 * priority list that this machine has is chosen with its effort. With none
 * of them available the chat keeps its usual default model.
 *
 * The prompt waits in the composer instead of going when sending it now
 * could put it in the wrong place: its agent profile is not served (or
 * could not be read), so it would run under another agent, or the folder
 * lookup did not finish (`workingDir` undefined; `null` is a lookup that
 * found none), so it would start before the chat's own lookup sets one.
 * Returns the chat's id.
 */
export function openExamplePrompt(
  api: PromptChatApi,
  prompt: ExamplePrompt,
  agents: readonly AgentEntry[],
  workingDir: string | null | undefined,
): string {
  const id = api.createNew({ text: prompt.prompt })
  if (workingDir) api.prefillWorkingDir(id, workingDir)
  const agent = agents.find((entry) => entry.id === prompt.agent)
  // An explicit pick, like a click on the gallery card.
  if (agent) api.setAgentProfile(id, agentProfileFromEntry(agent))
  const pick = resolvePromptModel(
    prompt.models,
    api.modelOptions.map((option) => option.id),
  )
  if (pick) api.setModel(id, pick.model)
  // Without a pick the chat's model is unknown here, so only `lowest`
  // (resolved by the harness against it) can be taken.
  const effort = pick?.effort ?? prompt.effort
  const model = pick ? api.modelOptions.find((option) => option.id === pick.model) : undefined
  if (effort && effortSupported(model, effort)) api.setThinkingLevel(id, effort)
  if (agent && workingDir !== undefined) sendDraftWhenReady(id, prompt.prompt)
  api.openConversation(id)
  return id
}

/**
 * The longest an example waits for its profile or folder lookup. Past it
 * the chat opens with what is known, and the prompt waits for a manual send
 * (see `openExamplePrompt`) while the chat looks up its folder itself.
 */
const LOOKUP_WAIT_MS = 3000

function withinLookupWait<T>(lookup: Promise<T>, fallback: T): Promise<T> {
  return Promise.race([
    lookup,
    new Promise<T>((resolve) =>
      window.setTimeout(() => resolve(fallback), LOOKUP_WAIT_MS),
    ),
  ])
}

/**
 * `openExamplePrompt` with its lookups: the agent profiles (when the caller
 * has none in hand) and the folder a new chat starts in. The setup wizard's
 * Ready step and a worker page's `host.chat.startChat` share this, so the
 * learn-iii tour starts a chat exactly the way an example prompt does.
 * Resolves to the chat's id.
 */
export async function startExamplePrompt(
  api: PromptChatApi,
  prompt: ExamplePrompt,
  agents: readonly AgentEntry[] | null = null,
): Promise<string> {
  const [profiles, workingDir] = await Promise.all([
    agents ??
      withinLookupWait(
        getIiiClient()
          .then(listAgents)
          .catch(() => []),
        [],
      ),
    // `undefined` = the lookup did not finish, timed out or failed (not:
    // it found none).
    withinLookupWait<string | null | undefined>(
      fetchNewChatWorkingDir().catch(() => undefined),
      undefined,
    ),
  ])
  return openExamplePrompt(api, prompt, profiles, workingDir)
}
