import type { ISdk, Trigger } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { TURN_STATE_SCOPE } from '../../turn-orchestrator/state.js';
import type { FanoutState } from '../ui-subscribe.js';

export const SESSION_CREATED_HANDLER_FN_ID = 'harness::fanout::session_created';

/**
 * A new session is signalled by the first `state:created` write on scope
 * `turn_state` (key = session id). The state trigger matches that scope in
 * engine — no `condition_function_id` RPC per turn_state update — so this
 * handler is the sole gate: it acts only on `state:created`.
 */
function sessionCreatedId(event: unknown): string | null {
  const obj = (event ?? {}) as Record<string, unknown>;
  if (obj.event_type !== 'state:created') return null;
  const key = typeof obj.key === 'string' ? obj.key : '';
  return key.length > 0 ? key : null;
}

export function spawnSessionsPoll(iii: ISdk, state: FanoutState): () => void {
  const handlerRef = iii.registerFunction(
    SESSION_CREATED_HANDLER_FN_ID,
    async (event: unknown) => {
      const session_id = sessionCreatedId(event);
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
        'Internal: fans out a newly-created session id to ui::sessions::changed::<browser_id>.',
    },
  );

  let trigger: Trigger | null = null;
  try {
    trigger = iii.registerTrigger({
      type: 'state',
      function_id: SESSION_CREATED_HANDLER_FN_ID,
      config: { scope: TURN_STATE_SCOPE },
    });
  } catch (err) {
    logger.warn('sessions state trigger registration failed', { err: String(err) });
  }

  return () => {
    try {
      trigger?.unregister();
    } catch {}
    try {
      handlerRef.unregister();
    } catch {}
  };
}
