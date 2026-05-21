import {
  MAX_PRESERVE_RECENT_TOKENS,
  MIN_PRESERVE_RECENT_TOKENS,
  deprecatedTriggerTokensCap,
  preserveRecentTokensOverride,
  reservedTokens,
} from './config.js';

export type ModelLimit = {
  context: number;
  input: number;
  output: number;
};

export type ModelLike = {
  id: string;
  limit: ModelLimit;
};

export type TokensLike = {
  input?: number;
  output?: number;
  cache_read?: number;
  cache_write?: number;
  total?: number;
};

export function usable(input: { model: ModelLike; reserved?: number }): number {
  const { model } = input;
  if (model.limit.context === 0) return 0;
  const reserved = input.reserved ?? reservedTokens();
  const base =
    model.limit.input > 0
      ? Math.max(0, model.limit.input - reserved)
      : Math.max(0, model.limit.context - model.limit.output);
  const cap = deprecatedTriggerTokensCap();
  return cap !== undefined ? Math.min(base, cap) : base;
}

export function isOverflow(input: {
  tokens: TokensLike;
  model: ModelLike;
  reserved?: number;
  autoDisabled?: boolean;
}): boolean {
  if (input.autoDisabled) return false;
  if (input.model.limit.context === 0) return false;
  const count =
    typeof input.tokens.total === 'number' && input.tokens.total > 0
      ? input.tokens.total
      : (input.tokens.input ?? 0) +
        (input.tokens.output ?? 0) +
        (input.tokens.cache_read ?? 0) +
        (input.tokens.cache_write ?? 0);
  return count >= usable({ model: input.model, reserved: input.reserved });
}

export function preserveRecentBudget(input: {
  model: ModelLike;
  reserved?: number;
  override?: number;
}): number {
  const ovr = input.override ?? preserveRecentTokensOverride();
  if (ovr !== undefined) return ovr;
  const u = usable({ model: input.model, reserved: input.reserved });
  return Math.min(
    MAX_PRESERVE_RECENT_TOKENS,
    Math.max(MIN_PRESERVE_RECENT_TOKENS, Math.floor(u * 0.25)),
  );
}
