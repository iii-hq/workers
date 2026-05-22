import { describe, expect, it } from 'vitest';
import { functionsToOpenai } from '../../src/provider-lmstudio/wire-tools.js';
import type { AgentFunction } from '../../src/types/function.js';

describe('functionsToOpenai (lmstudio)', () => {
  // wire-tools.ts is intentionally separate from the OpenAI/Kimi copies so
  // LM Studio-specific tool-format extensions can land without coupling
  // providers. Pinning the exact output shape catches accidental drift
  // (e.g. adding a `strict` field, renaming `function` → `tool`, etc.).
  it('maps each AgentFunction to OpenAI tool shape with name, description, parameters', () => {
    const fns: AgentFunction[] = [
      {
        name: 'shell::exec',
        description: 'Run a shell command',
        parameters: { type: 'object', properties: { cmd: { type: 'string' } } },
      },
      {
        name: 'fs::read',
        description: 'Read a file',
        parameters: { type: 'object', properties: { path: { type: 'string' } } },
      },
    ];
    const out = functionsToOpenai(fns) as Array<Record<string, unknown>>;
    expect(out).toHaveLength(2);
    expect(out[0]).toEqual({
      type: 'function',
      function: {
        name: 'shell::exec',
        description: 'Run a shell command',
        parameters: { type: 'object', properties: { cmd: { type: 'string' } } },
      },
    });
    expect(out[1]?.type).toBe('function');
    expect((out[1] as { function: { name: string } }).function.name).toBe('fs::read');
  });

  it('returns an empty array for empty input (no leftover wrappers)', () => {
    expect(functionsToOpenai([])).toEqual([]);
  });
});
