/**
 * Poll `state::list scope=agent prefix=session/` once per second and
 * push diffs to subscribers via `ui::sessions::changed::<browser_id>`.
 * Mirrors `harness/src/fanout.rs::spawn_sessions_changed_poll` (simplified).
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { stateList } from '../../runtime/state.js';
import type { FanoutState } from '../ui-subscribe.js';

const POLL_INTERVAL_MS = 1_000;

export function spawnSessionsPoll(iii: ISdk, state: FanoutState): () => void {
  let stopped = false;
  let prev: Set<string> = new Set();
  const tick = async () => {
    if (stopped) return;
    try {
      const items = await stateList(iii, 'agent', 'session/');
      const ids = new Set<string>();
      for (const item of items) {
        if (item && typeof item === 'object') {
          const sid = (item as Record<string, unknown>).session_id;
          if (typeof sid === 'string') ids.add(sid);
        }
      }
      const added = [...ids].filter((id) => !prev.has(id));
      const removed = [...prev].filter((id) => !ids.has(id));
      if (added.length > 0 || removed.length > 0) {
        const total = ids.size;
        const payload = { added, removed, total };
        for (const browser_id of state.allSubscribers()) {
          iii
            .trigger<unknown, unknown>({
              function_id: `ui::sessions::changed::${browser_id}`,
              payload,
              timeoutMs: 2_000,
            })
            .catch((err) => logger.debug('ui::sessions::changed failed', { err: String(err) }));
        }
        prev = ids;
      }
    } catch (err) {
      logger.debug('sessions poll failed', { err: String(err) });
    } finally {
      if (!stopped) setTimeout(tick, POLL_INTERVAL_MS);
    }
  };
  setTimeout(tick, POLL_INTERVAL_MS);
  return () => {
    stopped = true;
  };
}
