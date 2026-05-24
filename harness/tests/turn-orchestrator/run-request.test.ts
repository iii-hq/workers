import { describe, expect, it } from 'vitest';
import { parseRunRequest } from '../../src/turn-orchestrator/run-request.js';

describe('parseRunRequest', () => {
  it('maps persisted run::start fields with defaults for missing keys', () => {
    expect(parseRunRequest({})).toEqual({
      provider: '',
      model: '',
      mode: null,
      system_prompt: '',
      function_schemas: [],
    });
  });

  it('passes through provided string fields', () => {
    expect(parseRunRequest({ provider: 'openai', model: 'gpt-4', system_prompt: 'hi' })).toEqual({
      provider: 'openai',
      model: 'gpt-4',
      mode: null,
      system_prompt: 'hi',
      function_schemas: [],
    });
  });

  it('rejects invalid mode values and accepts valid ones', () => {
    expect(parseRunRequest({ mode: 'invalid' }).mode).toBeNull();
    expect(parseRunRequest({ mode: 'plan' }).mode).toBe('plan');
    expect(parseRunRequest({ mode: 'ask' }).mode).toBe('ask');
    expect(parseRunRequest({ mode: 'agent' }).mode).toBe('agent');
  });

  it('coerces non-string fields to defaults', () => {
    expect(parseRunRequest({ provider: 123, model: null, system_prompt: {} })).toEqual({
      provider: '',
      model: '',
      mode: null,
      system_prompt: '',
      function_schemas: [],
    });
  });
});

describe('parseRunRequest function_schemas', () => {
  it('defaults to [] and carries an array', () => {
    expect(parseRunRequest({}).function_schemas).toEqual([]);
    expect(parseRunRequest({ function_schemas: [{ name: 'x' }] }).function_schemas).toHaveLength(1);
  });
});
