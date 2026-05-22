import { describe, expect, it } from 'vitest';
import { functionsToOpenai } from '../../src/provider-llamacpp/wire-tools.js';

describe('functionsToOpenai (llamacpp)', () => {
  it('maps each AgentFunction into the OpenAI tool envelope', () => {
    const out = functionsToOpenai([
      {
        name: 'sandbox::ls',
        description: 'list dir',
        parameters: { type: 'object', properties: {} },
      },
      {
        name: 'fs::read',
        description: 'read file',
        parameters: { type: 'object', properties: { path: { type: 'string' } } },
      },
    ]) as Array<{ type: string; function: { name: string; description: string } }>;
    expect(out).toHaveLength(2);
    expect(out[0]?.type).toBe('function');
    expect(out[0]?.function.name).toBe('sandbox::ls');
    expect(out[1]?.function.name).toBe('fs::read');
  });

  it('preserves an empty input as an empty array', () => {
    expect(functionsToOpenai([])).toEqual([]);
  });
});
