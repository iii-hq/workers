/**
 * `hook-fanout::publish_collect` handler. Mirrors
 * `hook-fanout/src/handler.rs::execute`.
 *
 * Flow:
 * 1. mint event_id, build envelope, publish via `iii::durable::publish`.
 * 2. poll `stream::list HOOK_REPLY_STREAM group_id=event_id` every
 *    `poll_interval_ms` until `decideExit` returns a reason.
 * 3. apply the merge rule and return `{ event_id, replies, merged }`.
 */

import { randomUUID } from 'node:crypto';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { type ExitReason, decideExit } from './exit.js';
import { mergeFieldMerge, mergeFirstBlockWins, mergePipelineLastWins } from './merge.js';
import { FUNCTION_ID, HOOK_REPLY_STREAM, parseMergeRule } from './types.js';

export type PublishCollectConfig = {
  default_timeout_ms: number;
  min_timeout_ms: number;
  poll_interval_ms: number;
  quiescence_ms: number;
};

export const DEFAULT_CONFIG: PublishCollectConfig = {
  default_timeout_ms: 10_000,
  min_timeout_ms: 50,
  poll_interval_ms: 25,
  quiescence_ms: 200,
};

export function buildPublishEnvelope(
  topic: string,
  event_id: string,
  payload: unknown,
): Record<string, unknown> {
  return {
    topic,
    data: {
      event_id,
      reply_stream: HOOK_REPLY_STREAM,
      payload,
    },
  };
}

function collectStreamItems(
  raw: unknown,
  collected: unknown[],
  lastIndex: { value: number },
): void {
  let items: unknown[] | null = null;
  if (Array.isArray(raw)) items = raw;
  else if (raw && typeof raw === 'object') {
    const inner = (raw as Record<string, unknown>).items;
    if (Array.isArray(inner)) items = inner;
  }
  if (!items) return;
  for (let i = lastIndex.value; i < items.length; i++) {
    const item = items[i];
    const payload =
      item && typeof item === 'object' && 'data' in (item as Record<string, unknown>)
        ? (item as Record<string, unknown>).data
        : item;
    collected.push(payload);
  }
  lastIndex.value = items.length;
}

/**
 * Build the publish-collect response envelope. Mirrors
 * `hook-fanout/src/handler.rs::build_response` (PR #150).
 *
 * Adds `publish.{ok,error}` and `publish_failed:true` markers so callers can
 * fail closed when a publish errored or an expected subscriber didn't reply.
 */
export function buildResponse(
  event_id: string,
  replies: unknown[],
  merged: unknown,
  publishFailed: boolean,
  publishError?: string,
): Record<string, unknown> {
  const publish: Record<string, unknown> = publishFailed
    ? publishError !== undefined
      ? { ok: false, error: publishError }
      : { ok: false }
    : { ok: true };
  const out: Record<string, unknown> = {
    event_id,
    replies,
    merged,
    publish,
  };
  if (publishFailed) out.publish_failed = true;
  return out;
}

export async function execute(
  iii: ISdk,
  cfg: PublishCollectConfig,
  payload: unknown,
): Promise<Record<string, unknown>> {
  if (!payload || typeof payload !== 'object') {
    throw new Error('hook-fanout::publish_collect: payload must be an object');
  }
  const obj = payload as Record<string, unknown>;
  const topic = obj.topic;
  if (typeof topic !== 'string') {
    throw new Error('hook-fanout::publish_collect: missing required field: topic');
  }
  const inner = obj.payload ?? {};
  const merge_rule_str = obj.merge_rule;
  if (typeof merge_rule_str !== 'string') {
    throw new Error('hook-fanout::publish_collect: missing required field: merge_rule');
  }
  const merge_rule = parseMergeRule(merge_rule_str);
  if (!merge_rule) {
    throw new Error(`hook-fanout::publish_collect: unknown merge_rule: ${merge_rule_str}`);
  }
  const timeout_ms = typeof obj.timeout_ms === 'number' ? obj.timeout_ms : cfg.default_timeout_ms;
  const expected_replies =
    typeof obj.expected_replies === 'number' ? obj.expected_replies : undefined;
  const quiescence_ms =
    typeof obj.quiescence_ms === 'number' ? obj.quiescence_ms : cfg.quiescence_ms;

  const event_id = randomUUID();
  const envelope = buildPublishEnvelope(topic, event_id, inner);
  const started_at = Date.now();
  let publish_failed = false;
  let publish_error: string | undefined;
  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'iii::durable::publish',
      payload: envelope,
    });
  } catch (err) {
    logger.warn('hook-fanout::publish_collect: publish trigger failed', {
      topic,
      err: String(err),
    });
    publish_failed = true;
    publish_error = err instanceof Error ? err.message : String(err);
  }

  const effective_timeout = Math.max(timeout_ms, cfg.min_timeout_ms);
  const deadline_ms = started_at + effective_timeout;
  const replies: unknown[] = [];
  const lastIndex = { value: 0 };
  let last_reply_at_ms: number | undefined;
  let exit_reason: ExitReason = 'deadline_no_replies';

  // poll loop
  for (;;) {
    const before_len = replies.length;
    try {
      const value = await iii.trigger<unknown, unknown>({
        function_id: 'stream::list',
        payload: { stream_name: HOOK_REPLY_STREAM, group_id: event_id },
      });
      collectStreamItems(value, replies, lastIndex);
    } catch (err) {
      logger.debug('hook-fanout: stream::list poll failed', { err: String(err) });
    }
    const now_ms = Date.now();
    if (replies.length > before_len) last_reply_at_ms = now_ms;
    const reason = decideExit({
      replies_count: replies.length,
      expected_replies,
      quiescence_ms,
      last_reply_at_ms,
      now_ms,
      deadline_ms,
    });
    if (reason !== null) {
      exit_reason = reason;
      break;
    }
    await sleep(cfg.poll_interval_ms);
  }

  const elapsed_ms = Date.now() - started_at;
  logger.info('hook-fanout::publish_collect completed', {
    topic,
    replies: replies.length,
    elapsed_ms,
    exit_reason,
    publish_failed,
  });

  let merged: unknown;
  switch (merge_rule) {
    case 'first_block_wins':
      merged = mergeFirstBlockWins(replies);
      break;
    case 'field_merge':
      merged = mergeFieldMerge(inner, replies);
      break;
    case 'pipeline_last_wins':
      merged = mergePipelineLastWins(inner, replies);
      break;
  }

  return buildResponse(event_id, replies, merged, publish_failed, publish_error);
}

export function register(iii: ISdk, cfg: PublishCollectConfig): void {
  iii.registerFunction(FUNCTION_ID, async (payload: unknown) => execute(iii, cfg, payload), {
    description: 'Publish a topic, collect subscriber replies until timeout, apply merge_rule.',
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
