/**
 * Thinking-related primitives. Mirrors
 * `harness/crates/harness-types/src/thinking.rs`.
 */

export type ThinkingLevel = 'off' | 'low' | 'medium' | 'high';

export type ThinkingBudgets = {
  input_tokens?: number;
  output_tokens?: number;
};

export type TextSignature = string;
export type TextPhase = 'start' | 'delta' | 'end';
