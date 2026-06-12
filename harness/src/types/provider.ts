/**
 * Provider streaming contract, consumed by both the orchestrator (caller) and
 * provider workers (handler).
 */

import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';
import type { Model } from '../models-catalog/types.js';

/**
 * Lax `StreamChannelRef` — the SDK's canonical type lives in `iii-sdk`,
 * but we only need the wire shape here.
 */
export const StreamChannelRefSchema = z.object({
  channel_id: z.string(),
  access_key: z.string(),
  direction: z.enum(['read', 'write']),
});
export type StreamChannelRef = z.infer<typeof StreamChannelRefSchema>;

export const AgentFunctionSchema = z.object({
  name: z.string(),
  description: z.string(),
  parameters: z.unknown().default({}),
  label: z.string().optional(),
  execution_mode: z.enum(['parallel', 'sequential']).optional(),
  prepare_arguments_supported: z.boolean().optional(),
});

export const ProviderStreamInputSchema = z.object({
  /**
   * Writer end of the channel the orchestrator opened. Provider writes
   * AssistantMessageEvent JSON text messages here, then closes.
   */
  writer_ref: StreamChannelRefSchema,
  system_prompt: z.string().nullable().optional(),
  model: z.string(),
  /** Pass-through; the providers serialize this themselves. */
  messages: z.array(z.unknown()),
  /** Nullish-tolerant: the router omits absent options, but null must not fail a stream. */
  tools: z
    .array(AgentFunctionSchema)
    .nullish()
    .transform((v) => v ?? []),
  /**
   * Optional reasoning/thinking level. Providers that support it map this
   * onto their native parameter (Anthropic `thinking`, OpenAI
   * `reasoning_effort`); others ignore it. Absent = off.
   */
  thinking_level: z
    .string()
    .nullish()
    .transform((v) => v ?? undefined),
  /**
   * Effective output-token budget, resolved + clamped by the router
   * (override > model ceiling > 32k soft cap). When present it is
   * authoritative; providers feed it through their own model-ceiling clamp
   * as the user override.
   */
  max_output_tokens: z
    .number()
    .int()
    .positive()
    .nullish()
    .transform((v) => v ?? undefined),
  /** Structured-output request, passed through to providers that support it. */
  response_format: z.unknown().optional(),
  /** This provider's namespaced options slice, verbatim from the caller. */
  provider_options: z.unknown().optional(),
  /**
   * Optional pre-resolved catalog entry for `model`, threaded from the
   * orchestrator so the provider does not re-fetch `models::get` it already
   * resolved at turn start. An optimization, never a source of truth: validated
   * leniently (any object passes; anything else coerces to absent) so a sparse
   * or partial catalog entry falls back to a live fetch instead of failing the
   * whole stream. Providers read its fields defensively.
   */
  model_meta: z
    .unknown()
    .optional()
    .transform((v) => (v && typeof v === 'object' ? (v as Model) : undefined)),
  /**
   * Optional stable id for the turn (the router sends its request_id, stable
   * across the retry attempts of one request). Providers may use it to dedupe
   * per-stream credential resolution within a turn; a new turn carries a new
   * key. Purely an optimization key — never a source of truth.
   */
  resolution_key: z
    .union([z.string(), z.number()])
    .nullish()
    .transform((v) => v ?? undefined),
});
export type ProviderStreamInput = z.infer<typeof ProviderStreamInputSchema>;

/**
 * Runtime-side schema used by the *receiving* provider handler. The
 * iii-sdk auto-hydrates any value matching {@link StreamChannelRefSchema}
 * (`{channel_id, access_key, direction}`) into a `ChannelWriter` /
 * `ChannelReader` *instance* via `resolveChannelValue` before the
 * handler runs (see iii-sdk/dist/index.mjs `resolveChannelValue`). So by
 * the time the handler sees the payload, `writer_ref` is no longer the
 * wire shape — it's a class instance with `sendMessage` / `close`. We
 * therefore relax just that one field for runtime validation; the
 * documented wire shape is still {@link ProviderStreamInputSchema}.
 */
export const ProviderStreamRuntimeInputSchema = ProviderStreamInputSchema.extend({
  writer_ref: z.unknown(),
});

export const ProviderStreamOutputSchema = z.object({
  ok: z.boolean(),
  status: z.string().optional(),
});
export type ProviderStreamOutput = z.infer<typeof ProviderStreamOutputSchema>;

/** Auto-derived JSON schemas, exposed via `engine::functions::info`. */
export const ProviderStreamInputJsonSchema = zodToJsonSchema(ProviderStreamInputSchema, {
  name: 'ProviderStreamInput',
});
export const ProviderStreamOutputJsonSchema = zodToJsonSchema(ProviderStreamOutputSchema, {
  name: 'ProviderStreamOutput',
});
