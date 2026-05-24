/**
 * Function-related types. Mirrors
 * `harness/crates/harness-types/src/function.rs`.
 */

import type { ContentBlock } from './content.js';

export type ExecutionMode = 'parallel' | 'sequential';

export type Transport = 'sse' | 'websocket' | 'auto';

export type CacheRetention = 'none' | 'short' | 'long';

/**
 * Tool / function descriptor sent to providers. The Rust type is
 * `AgentFunction`; we keep that name on the wire but accept the legacy
 * `tools` alias on input where it appears.
 */
export type AgentFunction = {
  name: string;
  description: string;
  /** JSON Schema describing the function arguments (provider-side `parameters`). */
  parameters: unknown;
  label?: string;
  execution_mode?: ExecutionMode;
  prepare_arguments_supported?: boolean;
};

/** A single function call emitted by the assistant. */
export type FunctionCall = {
  id: string;
  function_id: string;
  arguments: unknown;
};

/**
 * Result of a function call.
 *
 * `details` is freeform JSON the provider sees in the tool result envelope.
 * When `details.status === "denied"`, providers prepend a
 * `[PERMISSION_DENIED]` marker per `formatFunctionResultContent`.
 */
export type FunctionResult = {
  content: ContentBlock[];
  details: unknown;
  terminate?: boolean;
};

/** Prepared call entry used during FSM function execution. */
export type PreparedFunctionCall =
  | { kind: 'prepared'; function_call: FunctionCall }
  | { kind: 'immediate'; result: FunctionResult; is_error: boolean };

/** Finalized call entry after function execution completes. */
export type FinalizedFunctionCall = {
  function_call: FunctionCall;
  result: FunctionResult;
  is_error: boolean;
};
