// Token-aware tail selection. Pure logic, no I/O.

import type { AgentMessage } from '../types/agent-message.js';

export type Turn = {
  start: number;
  end: number; // exclusive
  index: number; // index of the user message
};

export type Tail = {
  start: number;
  index: number;
};

export type CompletedCompaction = {
  /** Index of the user message that produced this compaction (or -1 if unknown). */
  userIndex: number;
  /** Index of the assistant summary message (or -1 if unknown). */
  assistantIndex: number;
  /** The summary text. */
  summary: string;
};

export type SelectInput = {
  messages: AgentMessage[];
  budget: number;
  tailTurns: number;
  estimate: (messages: AgentMessage[]) => number;
};

export type SelectResult = {
  head: AgentMessage[];
  tail_start_index: number | undefined;
};

export type CompactionEntryLike = {
  id: string;
  summary: string;
  tokens_before: number;
  timestamp: number;
};

export type MessageWithEntryId = {
  entry_id: string;
  message: AgentMessage;
};

export type SelectWithEntryIdsInput = {
  entries: MessageWithEntryId[];
  budget: number;
  tailTurns: number;
  estimate: (messages: AgentMessage[]) => number;
};

export type SelectWithEntryIdsResult = {
  head: MessageWithEntryId[];
  tail_start_id: string | undefined;
};

export function turns(messages: AgentMessage[]): Turn[] {
  const result: Turn[] = [];
  for (let i = 0; i < messages.length; i++) {
    if (messages[i]?.role !== 'user') continue;
    result.push({ index: i, start: i, end: messages.length });
  }
  for (let i = 0; i < result.length - 1; i++) {
    const current = result[i];
    const nextStart = result[i + 1]?.start;
    if (current && nextStart !== undefined) current.end = nextStart;
  }
  return result;
}

export function splitTurn(input: {
  messages: AgentMessage[];
  turn: Turn;
  budget: number;
  estimate: (m: AgentMessage[]) => number;
}): Tail | undefined {
  if (input.budget <= 0) return undefined;
  if (input.turn.end - input.turn.start <= 1) return undefined;
  for (let start = input.turn.start + 1; start < input.turn.end; start++) {
    const size = input.estimate(input.messages.slice(start, input.turn.end));
    if (size > input.budget) continue;
    return { start, index: start };
  }
  return undefined;
}

export function select(input: SelectInput): SelectResult {
  if (input.tailTurns <= 0) return { head: input.messages, tail_start_index: undefined };
  const all = turns(input.messages);
  if (all.length === 0) return { head: input.messages, tail_start_index: undefined };
  const recent = all.slice(-input.tailTurns);
  const sizes = recent.map((t) => input.estimate(input.messages.slice(t.start, t.end)));

  let total = 0;
  let keep: Tail | undefined;
  for (let i = recent.length - 1; i >= 0; i--) {
    const turn = recent[i];
    const size = sizes[i];
    if (turn === undefined || size === undefined) continue;
    if (total + size <= input.budget) {
      total += size;
      keep = { start: turn.start, index: turn.index };
      continue;
    }
    const remaining = input.budget - total;
    const split = splitTurn({
      messages: input.messages,
      turn,
      budget: remaining,
      estimate: input.estimate,
    });
    if (split) keep = split;
    break;
  }
  if (!keep || keep.start === 0) {
    return { head: input.messages, tail_start_index: undefined };
  }
  return {
    head: input.messages.slice(0, keep.start),
    tail_start_index: keep.start,
  };
}

export function selectWithEntryIds(input: SelectWithEntryIdsInput): SelectWithEntryIdsResult {
  const messages = input.entries.map((e) => e.message);
  const r = select({
    messages,
    budget: input.budget,
    tailTurns: input.tailTurns,
    estimate: input.estimate,
  });
  const headCount = r.head.length;
  const head = input.entries.slice(0, headCount);
  const tail_start_id =
    r.tail_start_index !== undefined ? input.entries[r.tail_start_index]?.entry_id : undefined;
  return { head, tail_start_id };
}

export function completedCompactions(entries: CompactionEntryLike[]): CompletedCompaction[] {
  return entries
    .filter((e) => e.summary && e.summary.length > 0)
    .slice()
    .sort((a, b) => a.timestamp - b.timestamp)
    .map((e) => ({ summary: e.summary, userIndex: -1, assistantIndex: -1 }));
}
