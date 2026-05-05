import type { TestCase } from './cases.ts';
import { expectEqual } from './cases.ts';

export const EDGE_CASES: TestCase[] = [
  {
    name: 'missing command field rejects',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { args: ['hi'] }),
        "missing 'command'",
      );
    },
  },
  {
    name: 'non-string args entry rejects',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 'echo', args: ['--n', 5] as unknown as string[] }),
        'must be a string',
      );
    },
  },
  {
    name: 'bad shell-words quoting rejects',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 'echo "unterminated' }),
        'parse command',
      );
    },
  },
  {
    name: 'nonexistent unlisted command is rejected by allowlist',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: '/_no_such_bin_at_all_' }),
        'not in allowlist',
      );
    },
  },
  {
    name: 'empty args array runs the bare program',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'echo', args: [] });
      expectEqual(r.exit_code, 0, 'exit_code');
      expectEqual(r.stdout, '\n', 'bare echo prints newline');
    },
  },
];
