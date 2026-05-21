/**
 * Subscribe to `agent::events` and fan-out to per-browser
 * `ui::session::event::<browser_id>` triggers. Mirrors
 * `harness/src/fanout.rs::register_agent_event_pump`.
 */

import type { ISdk, Trigger } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { FanoutState } from '../ui-subscribe.js';

const FN_ID = 'harness::fanout::agent_event_handler';

export function spawnAgentEventsPump(iii: ISdk, state: FanoutState): Trigger | null {
  iii.registerFunction(
    FN_ID,
    async (frame: unknown) => {
      const obj = frame && typeof frame === 'object' ? (frame as Record<string, unknown>) : null;
      if (!obj) return null;
      const session_id =
        (typeof obj.groupId === 'string' && obj.groupId) ||
        (typeof obj.group_id === 'string' && obj.group_id) ||
        null;
      if (!session_id) return null;
      const inner =
        obj.event &&
        typeof obj.event === 'object' &&
        'data' in (obj.event as Record<string, unknown>)
          ? (obj.event as Record<string, unknown>).data
          : (obj.data ?? null);
      const payload = { session_id, event: inner };
      const browsers = state.subscribersFor(session_id);
      for (const browser_id of browsers) {
        iii
          .trigger<unknown, unknown>({
            function_id: `ui::session::event::${browser_id}`,
            payload,
            timeoutMs: 2_000,
          })
          .catch((err) => {
            logger.debug('ui::session::event push failed', {
              browser_id,
              err: String(err),
            });
            const msg = String(err);
            if (/function_not_found/.test(msg)) state.evictBrowser(browser_id);
          });
      }
      return null;
    },
    { description: 'Internal: agent::events fanout handler.' },
  );
  try {
    return iii.registerTrigger({
      type: 'stream',
      function_id: FN_ID,
      config: { stream_name: 'agent::events' },
    });
  } catch (err) {
    logger.warn('agent::events stream subscriber registration failed', {
      err: String(err),
    });
    return null;
  }
}
