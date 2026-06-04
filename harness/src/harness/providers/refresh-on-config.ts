/**
 * Bridge harness-config edits to model re-discovery.
 *
 * Editing the `harness` configuration entry (adding/removing a provider api
 * key in the console Workers tab, or via `configuration::set` from a script)
 * should make the model picker reflect the change without a manual refresh.
 * Only providers whose discovery-relevant settings changed are refreshed.
 * A single `ui::models::changed` push runs after the refresh wave completes.
 */

import { emitModelsCatalogChanged } from '../fanout/models-changed.js';
import {
  HARNESS_CONFIG_ID,
  normalizeHarnessConfig,
  parseConfigurationChangeEvent,
  providersAffectedByConfigChange,
} from '../../runtime/harness-config.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { FanoutState } from '../ui-subscribe.js';
import type { ProviderRegistry } from './registry.js';

const HANDLER_FN_ID = 'harness::providers::refresh_on_config';

/** Coalesce window for rapid successive config writes. */
const DEBOUNCE_MS = 500;

/** Per-provider refresh budget — discovery hits an upstream `/v1/models`. */
const REFRESH_TIMEOUT_MS = 30_000;

export function registerProviderRefreshOnConfig(
  iii: ISdk,
  registry: ProviderRegistry,
  fanoutState: FanoutState,
): void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pendingEvent: ReturnType<typeof parseConfigurationChangeEvent> | null = null;

  const refreshProviders = async (providerIds: readonly string[]): Promise<void> => {
    const listing = new Set(
      registry
        .list()
        .filter((p) => p.supports_model_listing)
        .map((p) => p.id),
    );
    const targets = providerIds.filter((id) => listing.has(id));
    if (targets.length === 0) return;

    await Promise.allSettled(
      targets.map((id) =>
        iii
          .trigger({
            function_id: `provider::${id}::refresh_models`,
            payload: {},
            timeoutMs: REFRESH_TIMEOUT_MS,
          })
          .catch((err) =>
            logger.debug('provider refresh-on-config failed', {
              provider: id,
              err: String(err),
            }),
          ),
      ),
    );
    emitModelsCatalogChanged(iii, fanoutState);
  };

  const runDebounced = async (): Promise<void> => {
    timer = null;
    const event = pendingEvent;
    pendingEvent = null;
    if (!event) return;

    const oldCfg = normalizeHarnessConfig(event.old_value);
    const newCfg = normalizeHarnessConfig(event.new_value);

    let targets: string[];
    if (event.old_value === null && event.new_value === null) {
      targets = registry
        .list()
        .filter((p) => p.supports_model_listing)
        .map((p) => p.id);
    } else {
      targets = providersAffectedByConfigChange(oldCfg, newCfg);
    }

    await refreshProviders(targets);
  };

  try {
    iii.registerFunction(HANDLER_FN_ID, async (payload: unknown) => {
      pendingEvent = parseConfigurationChangeEvent(payload);
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(() => void runDebounced(), DEBOUNCE_MS);
      return null;
    });
    iii.registerTrigger({
      type: 'configuration',
      function_id: HANDLER_FN_ID,
      config: { configuration_id: HARNESS_CONFIG_ID },
    });
  } catch (err) {
    logger.warn('harness: could not bind provider refresh-on-config trigger', {
      err: String(err),
    });
  }
}
