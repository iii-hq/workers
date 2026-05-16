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
    name: 'nonexistent command surfaces spawn error',
    async run({ call, expectError }) {
      // T13: no shell-side allowlist anymore — the error comes from the
      // OS at spawn time (ENOENT), not from a pre-spawn allowlist check.
      await expectError(
        () => call('shell::exec', { command: '/_no_such_bin_at_all_' }),
        'spawn',
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
