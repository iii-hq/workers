/**
 * `hook-fanout::publish_collect` handler. A single `stream` trigger on
 * `agent::hook_reply` routes every reply into an in-process `pending` map
 * keyed by `event_id`. Each call installs a Collector, publishes the
 * envelope, and resolves on whichever fires first: expected_replies met,
 * quiescence_ms elapsed since the last reply, or effective_timeout.
 */

import { randomUUID } from 'node:crypto';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { ExitReason } from './exit.js';
import { mergeFieldMerge, mergeFirstBlockWins, mergePipelineLastWins } from './merge.js';
import { FUNCTION_ID, HOOK_REPLY_STREAM, REPLY_HANDLER_FN_ID, parseMergeRule } from './types.js';

export type PublishCollectConfig = {
  default_timeout_ms: number;
  min_timeout_ms: number;
  /**
   * Retained for back-compat in config wiring; unused in the reactive
   * implementation. The waiter is event-driven, not interval-driven.
   */
  poll_interval_ms: number;
  quiescence_ms: number;
};

export const DEFAULT_CONFIG: PublishCollectConfig = {
  default_timeout_ms: 10_000,
  min_timeout_ms: 50,
  poll_interval_ms: 0,
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

type Collector = {
  replies: unknown[];
  expected_replies: number | undefined;
  quiescence_ms: number;
  resolve: ((reason: ExitReason) => void) | null;
  quiescenceTimer: ReturnType<typeof setTimeout> | null;
  deadlineTimer: ReturnType<typeof setTimeout> | null;
};

const pending = new Map<string, Collector>();

function clearTimers(c: Collector): void {
  if (c.quiescenceTimer) {
    clearTimeout(c.quiescenceTimer);
    c.quiescenceTimer = null;
  }
  if (c.deadlineTimer) {
    clearTimeout(c.deadlineTimer);
    c.deadlineTimer = null;
  }
}

function settle(c: Collector, reason: ExitReason): void {
  if (!c.resolve) return;
  const resolve = c.resolve;
  c.resolve = null;
  clearTimers(c);
  resolve(reason);
}

function deliverReply(c: Collector, payload: unknown): void {
  c.replies.push(payload);
  if (c.expected_replies !== undefined && c.replies.length >= c.expected_replies) {
    settle(c, 'expected_replies');
    return;
  }
  if (c.quiescence_ms > 0) {
    if (c.quiescenceTimer) clearTimeout(c.quiescenceTimer);
    c.quiescenceTimer = setTimeout(() => {
      settle(c, 'quiescence');
    }, c.quiescence_ms);
  }
}

/**
 * Stream trigger handler. Fires on every `stream::set` to
 * `agent::hook_reply`. Dispatches the reply to the pending Collector for
 * this `group_id`; ignores frames for unknown event_ids (e.g. late
 * arrivals after a publish_collect already exited).
 *
 * Exported for tests; production wiring goes through `register()`.
 */
export async function handleStreamReply(frame: unknown): Promise<unknown> {
  if (!frame || typeof frame !== 'object') return null;
  const obj = frame as Record<string, unknown>;
  const group_id =
    (typeof obj.groupId === 'string' && obj.groupId) ||
    (typeof obj.group_id === 'string' && obj.group_id) ||
    null;
  if (!group_id) return null;
  const inner =
    obj.event && typeof obj.event === 'object' && 'data' in (obj.event as Record<string, unknown>)
      ? (obj.event as Record<string, unknown>).data
      : (obj.data ?? obj);
  const c = pending.get(group_id);
  if (c) deliverReply(c, inner);
  return null;
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
  const started_at = Date.now();
  const effective_timeout = Math.max(timeout_ms, cfg.min_timeout_ms);

  const collector: Collector = {
    replies: [],
    expected_replies,
    quiescence_ms,
    resolve: null,
    quiescenceTimer: null,
    deadlineTimer: null,
  };
  pending.set(event_id, collector);

  let publish_error: string | undefined;
  try {
    const exit_reason = await new Promise<ExitReason>((resolve) => {
      collector.resolve = resolve;
      collector.deadlineTimer = setTimeout(() => {
        settle(collector, collector.replies.length === 0 ? 'deadline_no_replies' : 'deadline');
      }, effective_timeout);

      // Publish AFTER the collector is installed so an early reply can't
      // miss the Map lookup. iii::durable::publish enqueues; dispatch is
      // async on the bus, so there is no real race either way — but
      // installed-first is unambiguously correct.
      (async () => {
        try {
          const envelope = buildPublishEnvelope(topic, event_id, inner);
          await iii.trigger<unknown, unknown>({
            function_id: 'iii::durable::publish',
            payload: envelope,
          });
        } catch (err) {
          logger.warn('hook-fanout::publish_collect: publish trigger failed', {
            topic,
            err: String(err),
          });
          publish_error = err instanceof Error ? err.message : String(err);
        }
      })();
    });

    const elapsed_ms = Date.now() - started_at;
    logger.info('hook-fanout::publish_collect completed', {
      topic,
      replies: collector.replies.length,
      elapsed_ms,
      exit_reason,
      publish_failed: publish_error !== undefined,
    });

    let merged: unknown;
    switch (merge_rule) {
      case 'first_block_wins':
        merged = mergeFirstBlockWins(collector.replies);
        break;
      case 'field_merge':
        merged = mergeFieldMerge(inner, collector.replies);
        break;
      case 'pipeline_last_wins':
        merged = mergePipelineLastWins(inner, collector.replies);
        break;
    }

    return buildResponse(
      event_id,
      collector.replies,
      merged,
      publish_error !== undefined,
      publish_error,
    );
  } finally {
    clearTimers(collector);
    pending.delete(event_id);
  }
}

export function register(iii: ISdk, cfg: PublishCollectConfig): void {
  iii.registerFunction(REPLY_HANDLER_FN_ID, handleStreamReply, {
    description:
      'Internal: routes agent::hook_reply stream events to pending publish_collect calls.',
  });
  try {
    iii.registerTrigger({
      type: 'stream',
      function_id: REPLY_HANDLER_FN_ID,
      config: { stream_name: HOOK_REPLY_STREAM },
    });
  } catch (err) {
    logger.warn('hook-fanout reply stream trigger registration failed', {
      err: String(err),
    });
  }
  iii.registerFunction(FUNCTION_ID, async (payload: unknown) => execute(iii, cfg, payload), {
    description: 'Publish a topic, collect subscriber replies until timeout, apply merge_rule.',
  });
}
