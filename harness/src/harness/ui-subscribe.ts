/**
 * In-memory model-catalog subscriber registry plus
 * `ui::models::subscribe` / `ui::models::unsubscribe` function registrations.
 */

import { unwrapBody } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';

export class FanoutState {
  private readonly modelSubs = new Set<string>();

  subscribeModels(browser_id: string): void {
    this.modelSubs.add(browser_id);
  }

  unsubscribeModels(browser_id: string): void {
    this.modelSubs.delete(browser_id);
  }

  /** Browsers subscribed to model-catalog changes. */
  modelSubscribers(): string[] {
    return [...this.modelSubs];
  }
}

export function registerSubscriptions(iii: ISdk, state: FanoutState): void {
  iii.registerFunction(
    'ui::models::subscribe',
    async (input: unknown) => {
      const body = unwrapBody(input);
      const browser_id = typeof body.browser_id === 'string' ? body.browser_id : null;
      if (!browser_id) throw new Error('missing browser_id');
      state.subscribeModels(browser_id);
      return { ok: true };
    },
    {
      description:
        "Register a browser's interest in model-catalog changes (ui::models::changed pushes).",
    },
  );

  iii.registerFunction(
    'ui::models::unsubscribe',
    async (input: unknown) => {
      const body = unwrapBody(input);
      const browser_id = typeof body.browser_id === 'string' ? body.browser_id : null;
      if (!browser_id) throw new Error('missing browser_id');
      state.unsubscribeModels(browser_id);
      return { ok: true };
    },
    {
      description: "Remove a browser's model-catalog change subscription.",
    },
  );
}
