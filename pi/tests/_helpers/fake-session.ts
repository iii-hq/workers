/** Scripted Pi AgentSession replacement: stores the subscribe listener, fires
 *  a fixed event list when prompt() is called, and records the build options,
 *  prompt, steering/follow-up, abort, and dispose calls it sees. */

import type { AgentSession } from '@earendil-works/pi-coding-agent';
import type { BuildSessionOptions } from '../../src/session.js';

export type PiEvent = { type: string; [k: string]: unknown };

export type SessionCapture = {
  buildOpts?: BuildSessionOptions;
  promptText?: string;
  steered: string[];
  followUps: string[];
  aborted: boolean;
  disposed: boolean;
};

export function newCapture(): SessionCapture {
  return { steered: [], followUps: [], aborted: false, disposed: false };
}

export type ScriptOptions = {
  events?: PiEvent[];
  stats?: unknown;
  sessionId?: string;
  sessionFile?: string;
  throwOnPrompt?: Error;
  gate?: Promise<void>;
};

export function scriptedSession(opts: ScriptOptions, capture: SessionCapture): AgentSession {
  let listener: ((e: PiEvent) => void) | undefined;
  const stats = opts.stats ?? {
    sessionFile: opts.sessionFile ?? '/sessions/cs-1.jsonl',
    sessionId: opts.sessionId ?? 'cs-1',
    tokens: { input: 5, output: 2, cacheRead: 0, cacheWrite: 0, total: 7 },
    cost: 0.01,
  };
  const fake = {
    subscribe(l: (e: PiEvent) => void) {
      listener = l;
      return () => {
        listener = undefined;
      };
    },
    async prompt(text: string) {
      capture.promptText = text;
      for (const e of opts.events ?? []) listener?.(e);
      if (opts.gate) await opts.gate;
      if (opts.throwOnPrompt) throw opts.throwOnPrompt;
    },
    getSessionStats() {
      return stats;
    },
    get sessionId() {
      return opts.sessionId ?? 'cs-1';
    },
    get sessionFile() {
      return opts.sessionFile ?? '/sessions/cs-1.jsonl';
    },
    async steer(t: string) {
      capture.steered.push(t);
    },
    async followUp(t: string) {
      capture.followUps.push(t);
    },
    async abort() {
      capture.aborted = true;
    },
    dispose() {
      capture.disposed = true;
    },
  };
  return fake as unknown as AgentSession;
}

/** One turn: a single bash tool call, then a final assistant message. */
export const fullTurnEvents: PiEvent[] = [
  { type: 'agent_start' },
  { type: 'turn_start' },
  {
    type: 'tool_execution_start',
    toolCallId: 'call_1',
    toolName: 'bash',
    args: { command: 'ls' },
  },
  {
    type: 'tool_execution_end',
    toolCallId: 'call_1',
    toolName: 'bash',
    result: 'files',
    isError: false,
  },
  {
    type: 'message_end',
    message: { role: 'assistant', content: [{ type: 'text', text: 'done' }] },
  },
  { type: 'turn_end', message: { role: 'assistant' }, toolResults: [] },
  { type: 'agent_end', messages: [] },
];
