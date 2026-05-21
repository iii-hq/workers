import type { AgentFunction } from '../types/function.js';

export function functionsToOpenai(functions: AgentFunction[]): unknown[] {
  return functions.map((t) => ({
    type: 'function',
    function: {
      name: t.name,
      description: t.description,
      parameters: t.parameters,
    },
  }));
}
