/**
 * Provider-router as a pure library (per PHASE-2-PLAN.md §4). The
 * orchestrator imports it directly; there is no `router::*` bus surface.
 */

import type { Model } from '../models-catalog/types.js';
import type { AgentMessage } from '../types/agent-message.js';
import type { AgentFunction } from '../types/function.js';
import type { ProviderStreamInput, StreamChannelRef } from '../types/provider.js';

export type RouteDecision =
  | { provider: 'anthropic'; model: string }
  | { provider: 'openai'; model: string }
  | { provider: 'kimi'; model: string }
  | { provider: 'lmstudio'; model: string }
  | { provider: 'llamacpp'; model: string };

export type RouteRequest = {
  provider?: string;
  model: string;
};

/** Pick a provider for a request. Defaults to Anthropic when ambiguous. */
export function decide(req: RouteRequest): RouteDecision {
  const p = (req.provider ?? '').toLowerCase();
  if (p === 'openai') return { provider: 'openai', model: req.model };
  if (p === 'kimi') return { provider: 'kimi', model: req.model };
  if (p === 'lmstudio') return { provider: 'lmstudio', model: req.model };
  if (p === 'llamacpp') return { provider: 'llamacpp', model: req.model };
  if (!p && /^gpt-|^o\d-/i.test(req.model)) {
    return { provider: 'openai', model: req.model };
  }
  if (!p && /^kimi-|^moonshot-v1-/i.test(req.model)) {
    return { provider: 'kimi', model: req.model };
  }
  return { provider: 'anthropic', model: req.model };
}

export function targetFunctionId(d: RouteDecision): string {
  switch (d.provider) {
    case 'anthropic':
      return 'provider::anthropic::stream';
    case 'openai':
      return 'provider::openai::stream';
    case 'kimi':
      return 'provider::kimi::stream';
    case 'lmstudio':
      return 'provider::lmstudio::stream';
    case 'llamacpp':
      return 'provider::llamacpp::stream';
  }
}

export function buildInput(
  d: RouteDecision,
  writer_ref: StreamChannelRef,
  system_prompt: string | null | undefined,
  messages: AgentMessage[],
  tools: AgentFunction[],
  thinking_level?: string,
  model_meta?: Model,
  resolution_key?: number,
): ProviderStreamInput {
  return {
    writer_ref,
    system_prompt: system_prompt ?? null,
    model: d.model,
    messages: messages as unknown[],
    tools,
    ...(thinking_level ? { thinking_level } : {}),
    ...(model_meta ? { model_meta } : {}),
    ...(resolution_key !== undefined ? { resolution_key } : {}),
  };
}
