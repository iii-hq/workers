/**
 * Model capability types. Wire-identical to
 * `models-catalog/src/lib.rs::{Model, Pricing, …}`.
 */

import { z } from 'zod';

export type ThinkingLevel = 'off' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh';

export type Transport = 'sse' | 'websocket' | 'auto';

export type CacheRetention = 'none' | 'short' | 'long';

export type ThinkingBudgets = {
  minimal?: number;
  low?: number;
  medium?: number;
  high?: number;
};

export type Pricing = {
  input_per_1m: number;
  output_per_1m: number;
  cache_read_per_1m?: number;
  cache_write_per_1m?: number;
};

export type Model = {
  id: string;
  provider: string;
  api: string;
  display_name: string;
  context_window: number;
  max_output_tokens?: number;
  supports_thinking?: boolean;
  supports_xhigh?: boolean;
  supports_tools?: boolean;
  supports_vision?: boolean;
  supports_cache?: boolean;
  thinking_budgets?: ThinkingBudgets;
  transports?: Transport[];
  default_cache_retention?: CacheRetention;
  pricing?: Pricing;
};

/**
 * Zod mirror of {@link Model}, used to validate model metadata that crosses a
 * worker boundary (`ProviderStreamInput.model_meta`). `.passthrough()` keeps
 * any future catalog fields intact. Kept field-for-field in sync with the
 * `Model` type above; the compile-time guard below fails the build on drift.
 */
export const ModelSchema = z
  .object({
    id: z.string(),
    provider: z.string(),
    api: z.string(),
    display_name: z.string(),
    context_window: z.number(),
    max_output_tokens: z.number().optional(),
    supports_thinking: z.boolean().optional(),
    supports_xhigh: z.boolean().optional(),
    supports_tools: z.boolean().optional(),
    supports_vision: z.boolean().optional(),
    supports_cache: z.boolean().optional(),
    thinking_budgets: z
      .object({
        minimal: z.number().optional(),
        low: z.number().optional(),
        medium: z.number().optional(),
        high: z.number().optional(),
      })
      .passthrough()
      .optional(),
    transports: z.array(z.enum(['sse', 'websocket', 'auto'])).optional(),
    default_cache_retention: z.enum(['none', 'short', 'long']).optional(),
    pricing: z
      .object({
        input_per_1m: z.number(),
        output_per_1m: z.number(),
        cache_read_per_1m: z.number().optional(),
        cache_write_per_1m: z.number().optional(),
      })
      .passthrough()
      .optional(),
  })
  .passthrough();

// Compile-time guard: every value ModelSchema parses is assignable to Model.
// If a field drifts out of sync, this line fails to typecheck.
const _modelSchemaSatisfiesType = (m: z.infer<typeof ModelSchema>): Model => m;
void _modelSchemaSatisfiesType;

export const MODELS_SCOPE = 'models';

export type Capability =
  | { type: 'thinking' }
  | { type: 'thinking_level'; level: ThinkingLevel }
  | { type: 'tools' }
  | { type: 'vision' }
  | { type: 'cache' };

/** Filter for `models::list`, applied against the provider-registered catalog. */
export type ListFilter = {
  provider?: string;
  capability?: Capability;
};

export function parseCapability(s: string): Capability | null {
  switch (s) {
    case 'thinking':
      return { type: 'thinking' };
    case 'thinking:low':
      return { type: 'thinking_level', level: 'low' };
    case 'thinking:medium':
      return { type: 'thinking_level', level: 'medium' };
    case 'thinking:high':
      return { type: 'thinking_level', level: 'high' };
    case 'thinking:xhigh':
      return { type: 'thinking_level', level: 'xhigh' };
    case 'tools':
      return { type: 'tools' };
    case 'vision':
      return { type: 'vision' };
    case 'cache':
      return { type: 'cache' };
    default:
      return null;
  }
}

export function supportsModel(m: Model, capability: Capability): boolean {
  switch (capability.type) {
    case 'thinking':
      return Boolean(m.supports_thinking);
    case 'thinking_level':
      return capability.level === 'xhigh'
        ? Boolean(m.supports_xhigh)
        : Boolean(m.supports_thinking);
    case 'tools':
      return Boolean(m.supports_tools);
    case 'vision':
      return Boolean(m.supports_vision);
    case 'cache':
      return Boolean(m.supports_cache);
  }
}
