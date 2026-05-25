/**
 * Subscriber-presence cache for durable pub/sub topics.
 *
 * The after-function-call hook publishes to a durable topic and then blocks
 * waiting for subscriber replies. When no worker subscribes to that topic the
 * wait is pure dead time on the turn's critical path (the collector only exits
 * on its deadline). This module answers "does anyone subscribe to <topic>?" by
 * querying `engine::triggers::list` for a `durable:subscriber` trigger bound to
 * the topic (the registration shape consumers use — see iii queue worker).
 *
 * The answer is cached with a short TTL so the engine isn't queried per call.
 * Hook subscribers are a deploy-time concern (workers register at startup), so
 * a coarse TTL is fine; a newly-registered subscriber is picked up within one
 * TTL window, which is consistent with the hook's existing "late arrivals may
 * be dropped" contract. TTL is preferred over an `engine::functions-available`
 * invalidation trigger to avoid adding another always-on trigger.
 */

import { TriggerInfo } from 'iii-sdk';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

/** How long a subscriber-presence answer stays fresh before re-querying. */
export const SUBSCRIBER_CACHE_TTL_MS = 30_000;

const DURABLE_SUBSCRIBER_TYPE = 'durable:subscriber';

type CacheEntry = { has: boolean; at: number };
const cache = new Map<string, CacheEntry>();

/** Clear the presence cache. Test seam; not used in production. */
export function resetSubscriberCache(): void {
  cache.clear();
}

function topicOf(config: unknown): unknown {
  if (config && typeof config === 'object') {
    return (config as Record<string, unknown>).topic;
  }
  return undefined;
}

/**
 * True if any worker subscribes to `topic` via a durable (queue) subscriber.
 *
 * Result is cached per topic for `SUBSCRIBER_CACHE_TTL_MS`. On query failure
 * this returns `true` (fail-safe): callers fall back to their normal
 * publish/collect behavior rather than silently dropping a hook.
 *
 * `now` is injectable for testing the TTL; production callers omit it.
 */
export async function hasDurableSubscriber(
  iii: ISdk,
  topic: string,
  now: number = Date.now(),
): Promise<boolean> {
  const cached = cache.get(topic);
  if (cached && now - cached.at < SUBSCRIBER_CACHE_TTL_MS) {
    return cached.has;
  }

  try {
    const resp = await iii.trigger<unknown, { triggers?: TriggerInfo[] }>({
      function_id: 'engine::triggers::list',
      payload: {},
    });
    const has = (resp.triggers ?? []).some(
      (t) => t.trigger_type === DURABLE_SUBSCRIBER_TYPE && topicOf(t.config) === topic,
    );
    cache.set(topic, { has, at: now });
    return has;
  } catch (err) {
    logger.warn('subscriber presence check failed; assuming subscribers exist', {
      topic,
      err: String(err),
    });
    return true;
  }
}
