import { describe, expect, it } from 'vitest';
import type { PreparedEntry } from '../../src/turn-orchestrator/persistence.js';

describe('PreparedEntry with pre_approved', () => {
  it('accepts a pre_approved: true entry', () => {
    const entry: PreparedEntry = {
      function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
      blocked: null,
      pre_approved: true,
    };
    expect(entry.pre_approved).toBe(true);
  });

  it('defaults pre_approved to undefined', () => {
    const entry: PreparedEntry = {
      function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
      blocked: null,
    };
    expect(entry.pre_approved).toBeUndefined();
  });
});
