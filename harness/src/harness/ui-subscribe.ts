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

  evictBrowser(browser_id: string): void {
    this.subs.delete(browser_id);
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
}
