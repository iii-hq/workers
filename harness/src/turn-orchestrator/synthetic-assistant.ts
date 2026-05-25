/**
 * Synthetic assistant messages the orchestrator injects when there is no real
 * provider turn: stream/transition errors, aborts, and the max_turns stop.
 * Builds on `emptyAssistant` so the message scaffolding lives in one place.
 */

import { type AssistantMessage, emptyAssistant } from '../types/agent-message.js';
import { text } from '../types/content.js';
import type { StopReason } from '../types/stream-event.js';

export type SyntheticAssistantOptions = {
  stop_reason: StopReason;
  /** Body text; omitted produces empty content (e.g. an aborted turn). */
  text?: string;
  provider?: string;
  model?: string;
};

/**
 * Build an assistant message with no provider behind it. `error`/`aborted`
 * stop reasons are flagged `error_kind: 'transient'` and carry an
 * `error_message` (the body text, or the stop reason when there is no text).
 */
export function syntheticAssistant(opts: SyntheticAssistantOptions): AssistantMessage {
  const isError = opts.stop_reason === 'error' || opts.stop_reason === 'aborted';
  return {
    ...emptyAssistant(opts.provider ?? '', opts.model ?? ''),
    stop_reason: opts.stop_reason,
    content: opts.text ? [text(opts.text)] : [],
    error_message: isError ? (opts.text ?? opts.stop_reason) : null,
    error_kind: isError ? 'transient' : null,
  };
}
