import type { TestCase } from './cases.ts';
import { expectEqual } from './cases.ts';

export const EDGE_CASES: TestCase[] = [
  {
    name: 'missing command field rejects',
    async run({ call, expectError }) {
      // serde's missing-field wording uses backticks (`missing field
      // `command``) on the typed schema path; older deployments produced
      // `"missing 'command'"`. Match either by checking "missing" + "command"
      // separately so the test pins behavior, not exact phrasing.
      await expectError(
        () => call('shell::exec', { args: ['hi'] }),
        /missing[\s\S]*command|command[\s\S]*missing/,
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
    // Policy no longer screens argv[0]; a nonexistent program surfaces the
    // OS spawn failure instead of a policy rejection.
    name: 'nonexistent command fails at spawn, not policy',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: '/_no_such_bin_at_all_' }),
        'os error 2', // ADJUST after first run: pin a stable substring of the actual spawn error
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
