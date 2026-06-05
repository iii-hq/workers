/**
 * Per-model identity prompts, selected by the run's provider/model.
 * Routing reuses the provider-router's family heuristics.
 */

import { decide } from '../provider-router.js';
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

export function promptFamily(provider: string, model: string): PromptFamily {
  const route = decide({ provider, model });
  switch (route.provider) {
    case 'anthropic':
      return 'anthropic';
    case 'openai':
      return 'gpt';
    case 'kimi':
      return 'kimi';
    default:
      return 'default';
  }
}

export function selectIdentityPrompt(provider: string, model: string): string {
  return FAMILY_PROMPTS[promptFamily(provider, model)];
}
