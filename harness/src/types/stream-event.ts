/**
 * Streaming events emitted by providers. Wire-identical to
 * `harness/crates/harness-types/src/stream_event.rs`.
 *
 * Snake-case `type` tags because Rust serializes the variants as
 * `text_delta`, `thinking_delta`, etc.
 */

import type { AssistantMessage } from './agent-message.js';

export type StopReason = 'end' | 'length' | 'function_call' | 'aborted' | 'error';

export type ErrorKind =
  | 'auth_expired'
  | 'rate_limited'
  | 'context_overflow'
  | 'transient'
  | 'permanent';

export type Usage = {
  input?: number;
  output?: number;
  cache_read?: number;
  cache_write?: number;
  cost_usd?: number;
};

/**
 * Discriminated union streamed by providers over an iii channel. Each
 * non-terminal variant carries a `partial: AssistantMessage` accumulator;
 * `done` / `error` carry the final assembled message.
 */
export type AssistantMessageEvent =
  | { type: 'start'; partial: AssistantMessage }
  | { type: 'text_start'; partial: AssistantMessage }
  | { type: 'text_delta'; partial: AssistantMessage; delta: string }
  | { type: 'text_end'; partial: AssistantMessage }
  | { type: 'thinking_start'; partial: AssistantMessage }
  | { type: 'thinking_delta'; partial: AssistantMessage; delta: string }
  | { type: 'thinking_end'; partial: AssistantMessage }
  | { type: 'functioncall_start'; partial: AssistantMessage }
  | { type: 'functioncall_delta'; partial: AssistantMessage; delta: string }
  | { type: 'functioncall_end'; partial: AssistantMessage }
  | { type: 'usage'; usage: Usage }
  | {
      type: 'stop';
      stop_reason: StopReason;
      error_message?: string;
      error_kind?: ErrorKind;
    }
  | { type: 'done'; message: AssistantMessage }
  | { type: 'error'; error: AssistantMessage };

export function isTerminal(event: AssistantMessageEvent): boolean {
  return event.type === 'done' || event.type === 'error';
}
