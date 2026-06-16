import { describe, expect, it } from 'vitest';
import { defaultRunRequest } from '../../src/turn-orchestrator/run-request.js';

describe('defaultRunRequest', () => {
  it('is the empty run request used when no record exists yet', () => {
    expect(defaultRunRequest()).toEqual({
      provider: '',
      model: '',
      mode: null,
      system_prompt: '',
      function_schemas: [],
    });
  });
});
