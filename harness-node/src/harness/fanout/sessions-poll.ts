/**
 * Reactive sessions fanout via a native iii `state` trigger.
 *
 * Wiring:
 *   - A condition function (`harness::session::is_create_event`) filters
 *     state events to "session record created" — `event_type === 'created'`
 *     AND `key` matches `session/<id>/turn_state`. The engine evaluates the
 *     condition server-side and only fires the handler when it returns true.
 *   - A handler function (`harness::fanout::session_created`) receives the
 *     `StateEventData` (`{event_type, scope, key, old_value, new_value}`)
 *     and pushes `ui::sessions::changed::<browser_id>` to all-session
 *     subscribers.
 *   - A `state` trigger on `scope=agent` binds the two together.
 *
 * No polling, no hand-rolled lifecycle topic, no diff against a prev set
 * — the engine already knows when a session record was newly created and
 * delivers the value directly to the handler.
 */

import type { ISdk, Trigger } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { FanoutState } from '../ui-subscribe.js';

export const SESSION_CREATED_HANDLER_FN_ID = 'harness::fanout::session_created';
export const SESSION_CREATE_CONDITION_FN_ID = 'harness::session::is_create_event';
const SESSION_RECORD_KEY_RE = /^session\/[^/]+\/turn_state$/;

function extractSessionId(key: string): string | null {
  const m = SESSION_RECORD_KEY_RE.exec(key);
  if (!m) return null;
  return key.slice('session/'.length, key.length - '/turn_state'.length);
}

export function spawnSessionsPoll(iii: ISdk, state: FanoutState): () => void {
  const conditionRef = iii.registerFunction(
    SESSION_CREATE_CONDITION_FN_ID,
    async (event: unknown) => {
      const obj = (event ?? {}) as Record<string, unknown>;
      const event_type = typeof obj.event_type === 'string' ? obj.event_type : null;
      const key = typeof obj.key === 'string' ? obj.key : null;
      return event_type === 'state:created' && !!key && SESSION_RECORD_KEY_RE.test(key);
    },
    {
      description:
        'Condition: state event is a new session record (event_type=state:created, scope=agent, key=session/<id>/turn_state).',
    },
  );

  const handlerRef = iii.registerFunction(
    SESSION_CREATED_HANDLER_FN_ID,
    async (event: unknown) => {
      const obj = (event ?? {}) as Record<string, unknown>;
      const key = typeof obj.key === 'string' ? obj.key : '';
      const session_id = extractSessionId(key);
      if (!session_id) return null;
      const payload = { added: [session_id], removed: [] as string[] };
      for (const browser_id of state.allSubscribers()) {
        iii
          .trigger<unknown, unknown>({
            function_id: `ui::sessions::changed::${browser_id}`,
            payload,
            timeoutMs: 2_000,
          })
          .catch((err) => logger.debug('ui::sessions::changed failed', { err: String(err) }));
      }
      return null;
    },
    {
      description:
        'Internal: fans out a single newly-created session id to ui::sessions::changed::<browser_id>.',
    },
  );

  let trigger: Trigger | null = null;
  try {
    trigger = iii.registerTrigger({
      type: 'state',
      function_id: SESSION_CREATED_HANDLER_FN_ID,
      config: {
        scope: 'agent',
        condition_function_id: SESSION_CREATE_CONDITION_FN_ID,
      },
    });
  } catch (err) {
    logger.warn('sessions state trigger registration failed', { err: String(err) });
  }

  return () => {
    try {
      trigger?.unregister();
    } catch {
      // ignore
    }
    try {
      handlerRef.unregister();
    } catch {
      // ignore
    }
    try {
      conditionRef.unregister();
    } catch {
      // ignore
    }
  };
}
