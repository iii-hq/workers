/**
 * Parser for the flat `session/<sid>/messages` agent-scope array.
 */

import { z } from 'zod';
import type { AgentMessage } from '../types/agent-message.js';

const FlatMessagesSchema = z
  .array(z.custom<AgentMessage>((v) => v != null && typeof v === 'object'))
  .catch([]);

export function parseFlatMessages(raw: unknown): AgentMessage[] {
  return FlatMessagesSchema.parse(raw ?? []);
}
