import { describe, expect, it } from 'vitest';
import { route } from '../../src/turn-orchestrator/states/steering.js';

describe('steering route()', () => {
  it('abort wins over everything', () => {
    expect(route(true, true, true, true)).toBe('abort');
  });

  it('steering takes precedence over followup and function results', () => {
    expect(route(false, true, true, true)).toBe('steering');
  });

  it('followup takes precedence over function results', () => {
    expect(route(false, false, true, true)).toBe('followup');
  });

  it('function results route to continue_after_function', () => {
    expect(route(false, false, false, true)).toBe('continue_after_function');
  });

  it('nothing pending -> end_turn', () => {
    expect(route(false, false, false, false)).toBe('end_turn');
  });
});
