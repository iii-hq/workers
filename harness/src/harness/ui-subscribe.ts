/**
 * In-memory subscription registry + `ui::subscribe` / `ui::unsubscribe`
 * function registrations. Mirrors the fanout-state half of
 * `harness/src/fanout.rs`.
 */

import { unwrapBody } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';

const ALL_SESSIONS = '__all__';

export class FanoutState {
  // browser_id -> set of session ids (or "__all__")
  private readonly subs = new Map<string, Set<string>>();
  // browser_id set interested in model-catalog changes. Kept separate from
  // session subs so the agent-events pump (which evicts on a missing
  // ui::session::event handler) can't tear down a browser's model
  // subscription, and so model pushes never fan out as session events.
  private readonly modelSubs = new Set<string>();

  subscribe(browser_id: string, session_id: string | null): void {
    const key = session_id ?? ALL_SESSIONS;
    let set = this.subs.get(browser_id);
    if (!set) {
      set = new Set();
      this.subs.set(browser_id, set);
    }
    set.add(key);
  }

  unsubscribe(browser_id: string, session_id: string | null): void {
    const key = session_id ?? ALL_SESSIONS;
    const set = this.subs.get(browser_id);
    if (!set) return;
    set.delete(key);
    if (set.size === 0) this.subs.delete(browser_id);
  }

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

  evictBrowser(browser_id: string): void {
    this.subs.delete(browser_id);
    this.modelSubs.delete(browser_id);
  }

  browserCount(): number {
    return this.subs.size;
  }

  /** Browsers subscribed to `session_id` (or to all sessions). */
  subscribersFor(session_id: string): string[] {
    const out: string[] = [];
    for (const [browser_id, set] of this.subs) {
      if (set.has(session_id) || set.has(ALL_SESSIONS)) out.push(browser_id);
    }
    return out;
  }

  /** Browsers subscribed to all sessions. */
  allSubscribers(): string[] {
    const out: string[] = [];
    for (const [browser_id, set] of this.subs) {
      if (set.has(ALL_SESSIONS)) out.push(browser_id);
    }
    return out;
  }
}

export function registerSubscriptions(iii: ISdk, state: FanoutState): void {
  iii.registerFunction(
    'ui::subscribe',
    async (input: unknown) => {
      const body = unwrapBody(input);
      const browser_id = typeof body.browser_id === 'string' ? body.browser_id : null;
      if (!browser_id) throw new Error('missing browser_id');
      const session_id = typeof body.session_id === 'string' ? body.session_id : null;
      state.subscribe(browser_id, session_id);
      return { ok: true, total_browsers: state.browserCount() };
    },
    {
      description:
        "Register a browser's interest in a session (or all sessions if session_id is null).",
    },
  );

  iii.registerFunction(
    'ui::unsubscribe',
    async (input: unknown) => {
      const body = unwrapBody(input);
      const browser_id = typeof body.browser_id === 'string' ? body.browser_id : null;
      if (!browser_id) throw new Error('missing browser_id');
      const session_id = typeof body.session_id === 'string' ? body.session_id : null;
      state.unsubscribe(browser_id, session_id);
      return { ok: true, total_browsers: state.browserCount() };
    },
    {
      description:
        "Remove a browser's subscription to a session (or its all-sessions sub if session_id is null).",
    },
  );

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
