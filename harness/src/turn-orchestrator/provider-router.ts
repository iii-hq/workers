/**
 * Provider-router as a pure library (per PHASE-2-PLAN.md §4). The
 * orchestrator imports it directly; there is no `router::*` bus surface.
 */

import type { AgentMessage } from '../types/agent-message.js';
import type { AgentFunction } from '../types/function.js';
import type { ProviderStreamInput, StreamChannelRef } from '../types/provider.js';

export type RouteDecision =
  | { provider: 'anthropic'; model: string }
  | { provider: 'openai'; model: string }
  | { provider: 'kimi'; model: string }
  | { provider: 'lmstudio'; model: string };

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
  // Heuristic for missing provider: model name disambiguates.
  // LM Studio is intentionally excluded — model IDs are user-controlled
  // (e.g. `qwen/qwen3-4b-2507`) and overlap with HF-style IDs from other
  // services; require explicit provider='lmstudio'.
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
  }
}

export function buildInput(
  d: RouteDecision,
  writer_ref: StreamChannelRef,
  system_prompt: string | null | undefined,
  messages: AgentMessage[],
  tools: AgentFunction[],
): ProviderStreamInput {
  return {
    writer_ref,
    system_prompt: system_prompt ?? null,
    model: d.model,
    messages: messages as unknown[],
    tools,
  };
}
