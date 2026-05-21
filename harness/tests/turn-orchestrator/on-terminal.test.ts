import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  __pendingForTest as pending,
  clearTerminalWaiter,
  handleTerminalStateWrite,
  installTerminalWaiter,
  isTerminalStateWrite,
} from '../../src/turn-orchestrator/on-terminal.js';

afterEach(() => {
  pending.clear();
});

describe('isTerminalStateWrite condition', () => {
  it('matches state:updated on turn_state with state === "stopped"', () => {
    expect(
      isTerminalStateWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'tearing_down' },
        new_value: { state: 'stopped' },
        message_type: 'state',
      }),
    ).toBe(true);
  });

  it('matches state:created with state === "stopped" (replay edge case)', () => {
    expect(
      isTerminalStateWrite({
        event_type: 'state:created',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: { state: 'stopped' },
        message_type: 'state',
      }),
    ).toBe(true);
  });

  it('skips non-terminal turn_state writes', () => {
    expect(
      isTerminalStateWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'awaiting_assistant' },
        new_value: { state: 'function_execute' },
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('skips state:deleted', () => {
    expect(
      isTerminalStateWrite({
        event_type: 'state:deleted',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'stopped' },
        new_value: null,
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('skips writes to keys other than turn_state', () => {
    expect(
      isTerminalStateWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: false,
        new_value: true,
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('skips writes where new_value lacks a state field', () => {
    expect(
      isTerminalStateWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'tearing_down' },
        new_value: { turn_count: 3 },
        message_type: 'state',
      }),
    ).toBe(false);
  });
});

describe('installTerminalWaiter + handleTerminalStateWrite', () => {
  it('resolves the waiter when a terminal write fires for that session', async () => {
    const waiter = installTerminalWaiter('sess-abc');

    handleTerminalStateWrite({
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-abc/turn_state',
      old_value: { state: 'tearing_down' },
      new_value: { state: 'stopped' },
      message_type: 'state',
    });

    await expect(waiter).resolves.toBeUndefined();
  });

  it('ignores writes for unrelated sessions', async () => {
    const waiter = installTerminalWaiter('sess-abc');
    let resolved = false;
    waiter.then(() => {
      resolved = true;
    });

    handleTerminalStateWrite({
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-xyz/turn_state',
      old_value: { state: 'tearing_down' },
      new_value: { state: 'stopped' },
      message_type: 'state',
    });

    await Promise.resolve();
    expect(resolved).toBe(false);

    clearTerminalWaiter('sess-abc');
  });

  it('clearTerminalWaiter removes the waiter without resolving it', async () => {
    const waiter = installTerminalWaiter('sess-abc');
    let settled = false;
    waiter.then(() => {
      settled = true;
    });

    clearTerminalWaiter('sess-abc');

    handleTerminalStateWrite({
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-abc/turn_state',
      old_value: { state: 'tearing_down' },
      new_value: { state: 'stopped' },
      message_type: 'state',
    });

    await Promise.resolve();
    expect(settled).toBe(false);
  });

  it('handleTerminalStateWrite is a no-op for malformed events', () => {
    expect(() => handleTerminalStateWrite(null)).not.toThrow();
    expect(() =>
      handleTerminalStateWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'not/a/match',
        new_value: { state: 'stopped' },
        message_type: 'state',
      }),
    ).not.toThrow();
  });

  it('multiple terminal writes for the same session resolve the waiter exactly once', async () => {
    const waiter = installTerminalWaiter('sess-abc');
    const resolver = vi.fn();
    waiter.then(resolver);

    handleTerminalStateWrite({
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-abc/turn_state',
      old_value: { state: 'tearing_down' },
      new_value: { state: 'stopped' },
      message_type: 'state',
    });
    handleTerminalStateWrite({
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-abc/turn_state',
      old_value: { state: 'stopped' },
      new_value: { state: 'stopped' },
      message_type: 'state',
    });

    await Promise.resolve();
    expect(resolver).toHaveBeenCalledTimes(1);
  });
});
