import type { ISdk } from 'iii-sdk';

export interface CaseContext {
  call: (functionId: string, payload: unknown) => Promise<any>;
  sleep: (ms: number) => Promise<void>;
  expectError: (fn: () => Promise<unknown>, pattern: string | RegExp) => Promise<void>;
  iii: ISdk;
}

export interface TestCase {
  name: string;
  run(ctx: CaseContext): Promise<void>;
}

export function expect(cond: boolean, msg: string): asserts cond {
  if (!cond) throw new Error(msg);
}

export function expectEqual(actual: unknown, expected: unknown, msg: string): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${msg}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

export const FUNCTION_CASES: TestCase[] = [
  {
    name: 'exec echo hi',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'echo', args: ['hi'] });
      expectEqual(r.exit_code, 0, 'exit_code');
      expectEqual(r.stdout, 'hi\n', 'stdout');
      expectEqual(r.stderr, '', 'stderr');
      expectEqual(r.timed_out, false, 'timed_out');
      expectEqual(r.stdout_truncated, false, 'stdout_truncated');
      expectEqual(r.stderr_truncated, false, 'stderr_truncated');
    },
  },
  {
    name: 'exec parses shell-words when args omitted',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'echo "a b"' });
      expectEqual(r.exit_code, 0, 'exit_code');
      expectEqual(r.stdout, 'a b\n', 'stdout');
    },
  },
  {
    name: 'exec uses args field verbatim',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'echo', args: ['a', 'b'] });
      expectEqual(r.exit_code, 0, 'exit_code');
      expectEqual(r.stdout, 'a b\n', 'stdout');
    },
  },
  {
    name: 'exec propagates non-zero exit code',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'false' });
      expectEqual(r.exit_code, 1, 'exit_code from `false`');
      expectEqual(r.timed_out, false, 'timed_out');
    },
  },
  {
    name: 'exec captures stderr separately from stdout',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'sh',
        args: ['-c', 'echo err 1>&2'],
      });
      expectEqual(r.exit_code, 0, 'exit_code');
      expectEqual(r.stdout, '', 'stdout empty');
      expectEqual(r.stderr, 'err\n', 'stderr captured');
    },
  },
  {
    name: 'exec returns finite duration_ms',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'echo', args: ['x'] });
      expect(typeof r.duration_ms === 'number', 'duration_ms is number');
      expect(Number.isFinite(r.duration_ms) && r.duration_ms >= 0, 'duration_ms >= 0');
    },
  },
];
