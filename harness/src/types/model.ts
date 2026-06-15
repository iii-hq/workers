/**
 * Catalog model types, wire-aligned with the llm-router worker
 * (`llm-router/src/types/model.rs`). The router's catalog is the source of
 * truth: models are written via `router::models::reconcile` and read back via
 * `router::models::get`/`list`. The router only persists the fields its
 * `Model` struct declares — provider-side extras (`api`, `transports`,
 * `default_cache_retention`) are accepted in a reconcile payload but dropped
 * on the round-trip, so every field beyond the identity/limit core is
 * optional and consumers read them defensively.
 */

export type ThinkingLevel = 'off' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh';

export type Transport = 'sse' | 'websocket' | 'auto';

export type CacheRetention = 'none' | 'short' | 'long';

export type ThinkingBudgets = {
  minimal?: number;
  low?: number;
  medium?: number;
  high?: number;
};

/** Router pricing shape (per-1M-token rates; all optional). */
export type Pricing = {
  input?: number;
  output?: number;
  cache_read?: number;
  cache_write?: number;
};

export type Model = {
  id: string;
  provider: string;
  display_name?: string;
  context_window: number;
  max_output_tokens?: number;
  input_limit?: number;
  supports_thinking?: boolean;
  supports_xhigh?: boolean;
  supports_tools?: boolean;
  supports_vision?: boolean;
  supports_cache?: boolean;
  supports_structured_output?: boolean;
  thinking_budgets?: ThinkingBudgets;
  pricing?: Pricing;
  /** Provider-side hints; not persisted by the router catalog. */
  api?: string;
  transports?: Transport[];
  default_cache_retention?: CacheRetention;
};
