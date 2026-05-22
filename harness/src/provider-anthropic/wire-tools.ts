import type { AgentFunction } from '../types/function.js';
import { encodeToolName } from './wire-messages.js';

export function functionsToWire(tools: AgentFunction[]): unknown[] {
  return tools.map((t) => ({
    name: encodeToolName(t.name),
    description: t.description,
    input_schema: t.parameters,
  }));
}
