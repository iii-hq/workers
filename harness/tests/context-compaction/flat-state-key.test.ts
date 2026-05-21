/**
 * Drift-guard: the context-compaction worker writes flat session messages
 * to a key it composes itself in `flat-state.ts::flatMessagesKey`, because
 * importing `turn-orchestrator/state.ts::messagesKey` directly would
 * create a package-layer dependency (the orchestrator depends on
 * context-compaction via preflight). The two functions MUST agree forever
 * — otherwise compaction silently writes the rewritten history to a
 * shadow key the orchestrator never reads from.
 *
 * If this test fails, fix the key shape in flat-state.ts to match
 * messagesKey, then take a hard look at whether the duplication should
 * be lifted into a shared `runtime/keys.ts` module.
 */

import { describe, expect, it } from 'vitest';
import { flatMessagesKey } from '../../src/context-compaction/flat-state.js';
import { messagesKey } from '../../src/turn-orchestrator/state.js';

describe('flatMessagesKey ↔ turn-orchestrator messagesKey', () => {
  it('produces the identical key for any session id', () => {
    for (const sid of ['s', 'console-abc', 'session-with-dashes-12345', 'x']) {
      expect(flatMessagesKey(sid)).toBe(messagesKey(sid));
    }
  });
});
