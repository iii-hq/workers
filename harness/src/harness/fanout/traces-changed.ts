/**
 * Subscribe to `agent::turn_end` and fan-out a coalesced, empty
 * `ui::traces::changed::<browser_id>` signal to every all-sessions
 * subscriber. Lets the Traces devtools view refetch on a real "a turn's
 * spans just landed" beat instead of polling `engine::traces::*` on a timer.
 *
 * Mirrors `agent-events.ts` (stream trigger + `function_not_found` eviction)
 * and `sessions-poll.ts` (`allSubscribers()` fan-out, `() => void` shutdown).
 * The handler ignores the frame body — it is a pure "something changed" tick;
 * `agent::turn_end` already carries one frame per turn, so the debounce is a
 * cheap second line of defence rather than the primary load reducer.
 */

import type { ISdk, Trigger } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { FanoutState } from '../ui-subscribe.js';

const FN_ID = 'harness::fanout::traces_changed_handler';
/**
 * Dedicated turn-end stream (mirrored by the turn-orchestrator producer; see
 * `turn-orchestrator/events.ts` `TURN_END_STREAM`). Re-declared locally to
 * keep this pump from pulling in the orchestrator module graph, matching the
 * convention in `context-compaction/register.ts`.
 */
const TURN_END_STREAM = 'agent::turn_end';
/** Trailing-edge debounce window: collapse a burst of frames into one push. */
const COALESCE_MS = 400;

export function spawnTracesChangedPump(iii: ISdk, state: FanoutState): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;

  const flush = () => {
    timer = null;
    const subscribers = state.allSubscribers();
    logger.info('traces-changed: fanout ui::traces::changed', {
      subscribers: subscribers.length,
    });
    for (const browser_id of subscribers) {
      iii
        .trigger<unknown, unknown>({
          function_id: `ui::traces::changed::${browser_id}`,
          payload: {},
          timeoutMs: 2_000,
        })
        .catch((err) => {
          const msg = String(err);
          if (/function_not_found/.test(msg)) {
            state.evictBrowser(browser_id);
          } else {
            logger.debug('ui::traces::changed push failed', { browser_id, err: msg });
          }
        });
    }
  };

  const handler = iii.registerFunction(
    FN_ID,
    async () => {
      logger.debug('traces-changed: agent::turn_end received');
      if (timer) clearTimeout(timer);
      timer = setTimeout(flush, COALESCE_MS);
      return null;
    },
    { description: 'Internal: agent::turn_end -> coalesced ui::traces::changed fanout.' },
  );

  let trigger: Trigger | null = null;
  try {
    trigger = iii.registerTrigger({
      type: 'stream',
      function_id: FN_ID,
      config: { stream_name: TURN_END_STREAM },
    });
    logger.info('traces-changed pump active: agent::turn_end -> ui::traces::changed');
  } catch (err) {
    logger.warn('traces-changed stream subscriber registration failed', {
      err: String(err),
    });
  }

  return () => {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    try {
      trigger?.unregister();
    } catch {}
    try {
      handler.unregister();
    } catch {}
  };
}
