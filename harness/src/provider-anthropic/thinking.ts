/**
 * Anthropic extended-thinking configuration. Maps an optional
 * `thinking_level` onto the Messages API `thinking` request field, with
 * budgets from the model catalog (`thinking_budgets`) when present and a
 * formula on the model's max output tokens as fallback.
 *
 * Invariant: Anthropic requires `max_tokens > thinking.budget_tokens`
 * (the budget counts toward max_tokens) and a minimum budget of 1024.
 * When the clamped request budget leaves no room, thinking is dropped
 * instead of sending a request the API would reject.
 */

import { logger } from '../runtime/otel.js';
import type { Model, ThinkingBudgets } from '../models-catalog/types.js';

export type ThinkingConfig = { type: 'enabled'; budget_tokens: number };

/** Anthropic's documented minimum thinking budget. */
export const MIN_THINKING_BUDGET = 1024;

/**
 * Tokens reserved for the visible answer. The thinking budget counts toward
 * max_tokens; without a reserve an `xhigh` budget can consume the whole
 * request budget and silently return an empty completion.
 */
export const OUTPUT_RESERVE_TOKENS = 1024;

function budgetFromCatalog(
  level: string,
  budgets: ThinkingBudgets | undefined,
): number | undefined {
  if (!budgets) return undefined;
  switch (level) {
    case 'minimal':
      return budgets.minimal;
    case 'low':
      return budgets.low;
    case 'medium':
      return budgets.medium;
    case 'high':
      return budgets.high;
    default:
      return undefined;
  }
}

/** Budget formula from the model's max output tokens `output`. */
function budgetFromFormula(level: string, output: number): number | undefined {
  switch (level) {
    case 'high':
      return Math.min(16_000, Math.floor(output / 2 - 1));
    case 'max':
    case 'xhigh':
      return Math.min(31_999, output - 1);
    case 'medium':
      return Math.min(8_000, Math.floor(output / 4));
    case 'low':
    case 'minimal':
      return Math.min(4_000, Math.floor(output / 8));
    default:
      return undefined;
  }
}

export function buildThinkingConfig(
  level: string | undefined,
  maxTokens: number,
  catalog?: Model,
): ThinkingConfig | undefined {
  if (!level || level === 'off') return undefined;
  // An explicit `supports_thinking: false` would 400; unknown stays permissive.
  if (catalog?.supports_thinking === false) return undefined;
  // Degrade xhigh to the high tier when the catalog says xhigh is unsupported.
  const effective =
    (level === 'xhigh' || level === 'max') && catalog?.supports_xhigh === false ? 'high' : level;

  let budget = budgetFromCatalog(effective, catalog?.thinking_budgets);
  if (budget === undefined || budget <= 0) {
    const output = catalog?.max_output_tokens;
    // The formula needs the model's output ceiling; unknown model -> no thinking.
    if (typeof output !== 'number' || output <= 0) return undefined;
    budget = budgetFromFormula(effective, output);
  }
  if (budget === undefined || budget <= 0) return undefined;

  // Keep budget below max_tokens with room for the visible answer; drop
  // thinking (logged) when less than the API minimum remains.
  budget = Math.min(budget, maxTokens - OUTPUT_RESERVE_TOKENS);
  if (budget < MIN_THINKING_BUDGET) {
    logger.debug('thinking dropped: budget below API minimum after max_tokens clamp', {
      level,
      maxTokens,
      budget,
    });
    return undefined;
  }

  return { type: 'enabled', budget_tokens: budget };
}
