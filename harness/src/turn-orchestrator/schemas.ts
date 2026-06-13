/**
 * Registered-function I/O contracts for turn-orchestrator. Every payload schema
 * and payload/result type for the worker's `iii.registerFunction` handlers lives
 * here, so the contract surface is readable in one place. Handlers import the
 * schema (to `.parse` at the boundary) and the inferred types from this file.
 */

import { z } from 'zod';
import type { AssistantMessage, AgentMessage } from '../types/agent-message.js';
import { TurnStateInvariantError } from './errors.js';
import type { FunctionBatchWork } from './function-execute/types.js';
import {
  FUNCTION_BATCH_STATES,
  type AssistantStreamingTurnRecord,
  type AwaitingApprovalEntry,
  type FunctionBatchTurnRecord,
  type TurnState,
  type TurnStateRecord,
} from './state.js';
import type { Mode } from './system-prompt.js';

/** Shared `{ session_id }` payload — `turn::{state}` steps and `turn::get_state`. */
const SessionIdPayloadSchema = z.object({
  session_id: z.string().min(1),
});

export const RunStartPayloadSchema = SessionIdPayloadSchema.extend({
  message_id: z.string().optional(),
  provider: z.string(),
  model: z.string(),
  mode: z.enum(['plan', 'ask', 'agent'] satisfies [Mode, Mode, Mode]).optional(),
  /**
   * Optional reasoning/thinking level, persisted on the run request and
   * threaded to `router::chat` (omitted on the wire when 'off' or absent).
   * The provider degrades-with-warning when the model can't honor it.
   */
  thinking_level: z.enum(['off', 'minimal', 'low', 'medium', 'high', 'xhigh']).optional(),
  messages: z.custom<AgentMessage[]>((v) => Array.isArray(v)).default([]),
  max_turns: z.number().optional(),
  system_prompt: z.string().default(''),
});
export type RunStartPayload = z.infer<typeof RunStartPayloadSchema>;
/**
 * `started` is false when the session already had a turn in flight and this
 * call was ignored (no record reset, no message appended) — see run-start.ts.
 */
export type RunStartResult = {
  session_id: string;
  started: boolean;
  reason?: 'session_busy';
};

export const RunAbortPayloadSchema = SessionIdPayloadSchema;
export type RunAbortPayload = z.infer<typeof RunAbortPayloadSchema>;
/** `aborted` is false when no turn was running (or it was already finishing). */
export type RunAbortResult = {
  session_id: string;
  aborted: boolean;
  state: TurnState | null;
};

export const TurnStepPayloadSchema = SessionIdPayloadSchema;
export type TurnStepPayload = z.infer<typeof TurnStepPayloadSchema>;
export type TurnStepResult =
  | { ok: true; from_state: TurnState; to_state: TurnState }
  | { ok: true; skipped: true; reason: 'stale' };

const AwaitingApprovalEntrySchema = z.object({
  function_call_id: z.string().min(1),
  function_id: z.string().min(1),
  args: z.unknown(),
});

const FunctionBatchWorkSchema = z.custom<FunctionBatchWork>(
  (v) =>
    v != null &&
    typeof v === 'object' &&
    Array.isArray((v as FunctionBatchWork).prepared) &&
    typeof (v as FunctionBatchWork).executed === 'object' &&
    (v as FunctionBatchWork).executed !== null,
  { message: 'work must include prepared and executed' },
);

const AssistantMessageSchema = z.custom<AssistantMessage>(
  (v) => v != null && typeof v === 'object' && (v as AssistantMessage).role === 'assistant',
  { message: 'last_assistant is required' },
);

/** Fields required before function_execute / function_awaiting_approval handlers run. */
export const FunctionBatchTurnRecordSchema = z
  .object({
    session_id: z.string().min(1),
    state: z.enum(FUNCTION_BATCH_STATES),
    turn_count: z.number(),
    function_results: z.array(z.unknown()),
    turn_end_emitted: z.boolean(),
    started_at_ms: z.number(),
    updated_at_ms: z.number(),
    last_assistant: AssistantMessageSchema,
    work: FunctionBatchWorkSchema,
    awaiting_approval: z.array(AwaitingApprovalEntrySchema),
  })
  .passthrough();

function formatZodIssues(error: z.ZodError): string {
  return error.issues.map((issue) => `${issue.path.join('.')}: ${issue.message}`).join('; ');
}

/** Validate persisted turn_state for function-batch handlers; throws {@link TurnStateInvariantError}. */
export function parseFunctionBatchRecord(rec: TurnStateRecord): FunctionBatchTurnRecord {
  const result = FunctionBatchTurnRecordSchema.safeParse(rec);
  if (!result.success) {
    throw new TurnStateInvariantError(
      `invalid function batch turn record: ${formatZodIssues(result.error)}`,
    );
  }
  return rec as FunctionBatchTurnRecord;
}

/** Fields required before assistant_streaming handlers run. */
export const AssistantStreamingTurnRecordSchema = z
  .object({
    session_id: z.string().min(1),
    state: z.literal('assistant_streaming'),
    turn_count: z.number(),
    function_results: z.array(z.unknown()),
    turn_end_emitted: z.boolean(),
    started_at_ms: z.number(),
    updated_at_ms: z.number(),
  })
  .passthrough();

/** Validate persisted turn_state for assistant_streaming; throws {@link TurnStateInvariantError}. */
export function parseAssistantStreamingRecord(rec: TurnStateRecord): AssistantStreamingTurnRecord {
  const result = AssistantStreamingTurnRecordSchema.safeParse(rec);
  if (!result.success) {
    throw new TurnStateInvariantError(
      `invalid assistant_streaming turn record: ${formatZodIssues(result.error)}`,
    );
  }
  return rec as AssistantStreamingTurnRecord;
}

export const GetStatePayloadSchema = SessionIdPayloadSchema;
export type GetStatePayload = z.infer<typeof GetStatePayloadSchema>;

/** Lean projection of TurnStateRecord sent to the UI and returned by turn::get_state.
 *  Excludes heavy internal fields (work, last_assistant) not needed by consumers. */
export type TurnStateView = {
  session_id: string;
  state: TurnState;
  turn_count: number;
  max_turns?: number;
  awaiting_approval?: AwaitingApprovalEntry[];
  error?: { kind: string; message: string };
};

export function toView(rec: TurnStateRecord): TurnStateView {
  return {
    session_id: rec.session_id,
    state: rec.state,
    turn_count: rec.turn_count,
    max_turns: rec.max_turns,
    awaiting_approval: rec.awaiting_approval,
    error: rec.error,
  };
}

export type GetStateResult = TurnStateView | null;

const slashFreeId = (label: string) =>
  z
    .string()
    .min(1)
    .refine((v) => !v.includes('/'), { message: `${label} must not contain "/"` });

const ContentBlockSchema = z
  .object({ type: z.string().min(1) })
  .passthrough()
  .array();

/**
 * `harness::function::resolve` payload — a sibling worker (the
 * approval-gate) settles one held call: `execute` releases it through the
 * normal dispatch pipeline; `deliver` answers it with the given content
 * without executing (user deny, sweep timeout).
 */
export const FunctionResolvePayloadSchema = z
  .object({
    session_id: slashFreeId('session_id'),
    turn_id: z.string().min(1),
    function_call_id: slashFreeId('function_call_id'),
    action: z.enum(['execute', 'deliver']),
    is_error: z.boolean().optional(),
    content: ContentBlockSchema.optional(),
    details: z.unknown().optional(),
  })
  .superRefine((v, ctx) => {
    if (v.action === 'deliver' && !v.content) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['content'],
        message: 'content is required when action is "deliver"',
      });
    }
  });
export type FunctionResolvePayload = z.infer<typeof FunctionResolvePayloadSchema>;

/** `resolved` is false for unknown/already-settled calls — never an error. */
export type FunctionResolveResult = {
  resolved: boolean;
  turn_resumed?: boolean;
};
