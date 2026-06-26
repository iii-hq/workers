/** Scripted `opencode run --format json` subprocess. Emits the given JSON event
 *  objects (one per stdout line) then closes with `code`, recording the spawn
 *  bin/args and any kill(). Shapes mirror a live `opencode run --format json`. */

import { EventEmitter } from 'node:events';
import { Readable } from 'node:stream';

export type SpawnCapture = { bin?: string; args?: string[]; killed: boolean };

export function newSpawnCapture(): SpawnCapture {
  return { killed: false };
}

export function scriptedSpawn(
  events: Array<object | string>,
  capture: SpawnCapture,
  opts: { code?: number; stderr?: string; hang?: boolean } = {},
) {
  const { code = 0, stderr = '', hang = false } = opts;
  return (bin: string, args: string[]) => {
    capture.bin = bin;
    capture.args = args;
    // A string event is emitted verbatim (e.g. a malformed line the parser must
    // drop); an object is JSON-serialized like a real OpenCode event.
    const lines = events.map((e) => `${typeof e === 'string' ? e : JSON.stringify(e)}\n`);
    const stdout = hang ? new Readable({ read() {} }) : Readable.from(lines);
    const stderrStream = Readable.from(stderr ? [stderr] : []);
    const child = new EventEmitter() as EventEmitter & {
      stdout: Readable;
      stderr: Readable;
      kill: (sig?: string) => void;
    };
    child.stdout = stdout;
    child.stderr = stderrStream;
    child.kill = () => {
      capture.killed = true;
      setImmediate(() => child.emit('close', 143));
    };
    if (!hang) {
      stdout.on('end', () => setImmediate(() => child.emit('close', code)));
    }
    return child;
  };
}

export const ev = {
  step_start: (sid = 'ses_1') => ({
    type: 'step_start',
    sessionID: sid,
    part: { type: 'step-start' },
  }),
  text: (text: string, sid = 'ses_1') => ({
    type: 'text',
    sessionID: sid,
    part: { type: 'text', text },
  }),
  tool: (tool: string, input: unknown, output: string, sid = 'ses_1', exit = 0) => ({
    type: 'tool_use',
    sessionID: sid,
    part: {
      type: 'tool',
      tool,
      callID: 'toolu_1',
      state: { status: exit === 0 ? 'completed' : 'error', input, output, metadata: { exit } },
    },
  }),
  step_finish: (sid = 'ses_1') => ({
    type: 'step_finish',
    sessionID: sid,
    part: {
      type: 'step-finish',
      reason: 'stop',
      tokens: { input: 3, output: 5, reasoning: 0, cache: { write: 100, read: 0 } },
      cost: 0.01,
    },
  }),
};

export const fullTurn = [
  ev.step_start(),
  ev.text('pong'),
  ev.tool('bash', { command: 'ls' }, 'files\n'),
  ev.step_finish(),
];
