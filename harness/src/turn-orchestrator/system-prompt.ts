/**
 * System-prompt assembly: picks the per-model identity prompt (prompt/*) for
 * the run's provider/model and prepends the mode paragraph. Every variant is
 * engine-grounded — the agent discovers capabilities from the live engine
 * (`engine::*` / `worker::*` / `directory::registry::workers::*`), installs
 * registry workers when nothing fits, routes code-file work through
 * `coder::*`, and fetches the iii.dev SDK reference before authoring workers.
 */

import { selectIdentityPrompt } from './prompt/index.js';

export type Mode = 'plan' | 'ask' | 'agent';

const MODE_PARAGRAPHS: Record<Mode, string> = {
  plan: `You are operating in plan mode: investigate first, then produce a concise numbered plan.
1. Investigate everything needed to fully plan — relevant functions, workers, and triggers — via \`agent_trigger\`.
2. Ask the user about any ambiguity or uncertain decision until they are confident in the plan, before finalizing it.
3. End the plan with a todo list of the actionable steps required to execute it.`,
  ask: 'You are operating in ask mode: answer the user directly and be concise (one or two paragraphs). Only call `agent_trigger` when strictly necessary to ground your answer.',
  agent:
    'You are operating in agent mode: use `agent_trigger` autonomously to satisfy the request. Stop when you have a final answer or hit an irrecoverable error.',
};

function isMode(value: unknown): value is Mode {
  return value === 'plan' || value === 'ask' || value === 'agent';
}

export type SystemPromptOptions = {
  /** Caller-supplied prompt; when non-empty it is returned verbatim. */
  override?: string | null;
  /** Operating mode; prepends a mode paragraph before the identity prompt. */
  mode?: Mode | null;
  /** Run's provider id (e.g. `anthropic`); selects the prompt family. */
  provider?: string | null;
  /** Run's model id (e.g. `gpt-5`); refines the family when provider is empty. */
  model?: string | null;
};

export function buildSystemPrompt(opts: SystemPromptOptions = {}): string {
  const { override, mode, provider, model } = opts;
  if (override && override.length > 0) return override;
  const identity = selectIdentityPrompt(provider ?? '', model ?? '');
  return isMode(mode) ? `${MODE_PARAGRAPHS[mode]}\n\n${identity}` : identity;
}
