import type { ISdk, Trigger } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { FanoutState } from '../ui-subscribe.js';

export const MODELS_CHANGED_HANDLER_FN_ID = 'harness::fanout::models_changed';

/**
 * Trailing coalesce window. Each models-scope state write resets the timer so
 * a multi-provider refresh wave collapses into one UI push after quiescence.
 */
const DEBOUNCE_MS = 250;

/**
 * Push `ui::models::changed::<browser_id>` to every subscribed browser.
 * Called by the debounced `router::models::changed` handler after catalog
 * writes.
 */
export function emitModelsCatalogChanged(iii: ISdk, state: FanoutState): void {
  for (const browser_id of state.modelSubscribers()) {
    iii
      .trigger<unknown, unknown>({
        function_id: `ui::models::changed::${browser_id}`,
        payload: { reason: 'models-changed' },
        timeoutMs: 2_000,
      })
      .catch((err) => {
        logger.debug('ui::models::changed failed', { browser_id, err: String(err) });
        if (/function_not_found/.test(String(err))) state.unsubscribeModels(browser_id);
      });
  }
}

/**
 * The model catalog lives in the llm-router worker, which publishes
 * `router::models::changed` on every reconcile. A subscribe trigger notifies
 * browsers after writes; trailing debounce collapses overlapping provider
 * reconciles into one push.
 */
export function spawnModelsChanged(iii: ISdk, state: FanoutState): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;

  const flush = (): void => {
    timer = null;
    emitModelsCatalogChanged(iii, state);
  };

  const scheduleFlush = (): void => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(flush, DEBOUNCE_MS);
  };

  const handlerRef = iii.registerFunction(
    MODELS_CHANGED_HANDLER_FN_ID,
    async () => {
      scheduleFlush();
      return null;
    },
    {
      description:
        'Internal: coalesces router::models::changed events into ui::models::changed::<browser_id> pushes.',
    },
  );

  let trigger: Trigger | null = null;
  try {
    trigger = iii.registerTrigger({
      type: 'subscribe',
      function_id: MODELS_CHANGED_HANDLER_FN_ID,
      config: { topic: 'router::models::changed' },
    });
  } catch (err) {
    logger.warn('router::models::changed trigger registration failed', { err: String(err) });
  }

  return () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    try {
      trigger?.unregister();
    } catch {}
    try {
      handlerRef.unregister();
    } catch {}
  };
}
