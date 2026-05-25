/**
 * Registered-function I/O contracts for turn-orchestrator. Every payload schema
 * and payload/result type for the worker's `iii.registerFunction` handlers lives
 * here, so the contract surface is readable in one place. Handlers import the
 * schema (to `.parse` at the boundary) and the inferred types from this file.
 */

import { z } from 'zod';
import type { AgentMessage } from '../types/agent-message.js';
import type { TurnState, TurnStateRecord } from './state.js';
import type { Mode } from './system-prompt.js';

/** Shared `{ session_id }` payload — `turn::{state}` steps and `turn::get_state`. */
export const SessionIdPayloadSchema = z.object({
  session_id: z.string().min(1),
});

// --- run::start ---
export const RunStartPayloadSchema = SessionIdPayloadSchema.extend({
  message_id: z.string().optional(),
  provider: z.string(),
  model: z.string(),
  mode: z.enum(['plan', 'ask', 'agent'] satisfies [Mode, Mode, Mode]).optional(),
  messages: z.custom<AgentMessage[]>((v) => Array.isArray(v)).default([]),
  max_turns: z.number().optional(),
  system_prompt: z.string().default(''),
});
export type RunStartPayload = z.infer<typeof RunStartPayloadSchema>;
export type RunStartResult = { session_id: string };

// --- turn::{state} durable step ---
export const TurnStepPayloadSchema = SessionIdPayloadSchema;
export type TurnStepPayload = z.infer<typeof TurnStepPayloadSchema>;
export type TurnStepResult =
  | { ok: true; from_state: TurnState; to_state: TurnState }
  | { ok: true; skipped: true; reason: 'stale' };

// --- turn::get_state ---
export const GetStatePayloadSchema = SessionIdPayloadSchema;
export type GetStatePayload = z.infer<typeof GetStatePayloadSchema>;
export type GetStateResult = TurnStateRecord | null;

// --- turn::is_abort_signal_set / turn::on_abort_signal (agent-scope state event) ---
const AgentAbortSignalWriteEventSchema = z.object({
  type: z.literal('state').optional(),
  scope: z.literal('agent').optional(),
  event_type: z.enum(['state:created', 'state:updated']),
  key: z.string().regex(/^session\/[^/]+\/abort_signal$/),
  new_value: z.literal(true),
  old_value: z.union([z.literal(true), z.literal(false), z.null()]).optional(),
});

export const AbortSignalWriteEventSchema = AgentAbortSignalWriteEventSchema.transform((data) => {
  const session_id = data.key.slice('session/'.length, -'/abort_signal'.length);
  return { session_id };
});
export type ParsedAbortSignalWrite = z.infer<typeof AbortSignalWriteEventSchema>;
