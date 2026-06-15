/**
 * Per-model identity prompts, selected by the run's ROUTED provider.
 * Routing authority lives in the llm-router worker: provisioning resolves the
 * provider once via `router::route` and persists it on the run request; this
 * module is a pure provider → family lookup with no routing logic of its own.
 */

import { PROMPT_ANTHROPIC } from './anthropic.js';
import { PROMPT_DEFAULT } from './default.js';
import { PROMPT_GPT } from './gpt.js';
import { PROMPT_KIMI } from './kimi.js';

export type PromptFamily = 'anthropic' | 'gpt' | 'kimi' | 'default';

const FAMILY_PROMPTS: Record<PromptFamily, string> = {
  anthropic: PROMPT_ANTHROPIC,
  gpt: PROMPT_GPT,
  kimi: PROMPT_KIMI,
  default: PROMPT_DEFAULT,
};

export function promptFamily(provider: string): PromptFamily {
  switch (provider) {
    case 'anthropic':
      return 'anthropic';
    case 'openai':
      return 'gpt';
    case 'kimi':
      return 'kimi';
    // No routed provider (router unreachable during provisioning): mirror the
    // router's seeded default_provider so the un-routed prompt matches what
    // the routed turn would have served.
    case '':
      return 'anthropic';
    default:
      return 'default';
  }
}

export function selectIdentityPrompt(provider: string): string {
  return FAMILY_PROMPTS[promptFamily(provider)];
}
