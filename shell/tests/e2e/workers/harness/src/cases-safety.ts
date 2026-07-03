import type { TestCase } from './cases.ts';
import { expect, expectEqual } from './cases.ts';

export const SAFETY_CASES: TestCase[] = [
  {
    name: 'allowlist rejects unlisted command',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 'nmap', args: ['-v'] }),
        'not in allowlist',
      );
    },
  },
  {
    // This suite runs UNJAILED (config.yaml sets no host_roots). In that mode a
    // command path (anything with '/') is rejected outright: the whole host FS
    // is writable via shell::fs::write, so a path could execute agent-planted
    // bytes and bypass the read-only allowlist. Bare PATH-resolved names work.
    // (In jailed mode an absolute path OUTSIDE the jail roots is still permitted by
    // basename — exercised by the jailed suite / Rust unit tests.)
    name: 'unjailed mode rejects command paths (RCE guard)',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: '/bin/ls', args: ['-d', '.'] }),
        'unjailed',
      );
      // The bare name still resolves via PATH and runs.
      const r = await call('shell::exec', { command: 'ls', args: ['-d', '.'] });
      expectEqual(r.exit_code, 0, 'bare command name still permitted');
    },
  },
  {
    name: 'denylist blocks via regex match on allowlisted command',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 'echo', args: ['harness_denylist_marker'] }),
        'denylist',
      );
    },
  },
  {
    name: 'timeout_ms triggers timed_out=true',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'sleep', args: ['5'], timeout_ms: 200 });
      expectEqual(r.timed_out, true, 'timed_out');
      expectEqual(r.exit_code, null, 'exit_code is null on timeout');
    },
  },
  {
    name: 'max_timeout_ms clamps oversize timeout_ms',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'sleep',
        args: ['0.05'],
        timeout_ms: 999_999,
      });
      expectEqual(r.timed_out, false, 'timed_out');
      expectEqual(r.exit_code, 0, 'exit_code');
      expect(r.duration_ms < 5000, `duration_ms < 5000 (got ${r.duration_ms})`);
    },
  },
  {
    name: 'stdout truncation flags at max_output_bytes',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'sh',
        args: ['-c', "printf 'x%.0s' $(seq 1 8000)"],
      });
      expectEqual(r.exit_code, 0, 'exit_code');
      expectEqual(r.stdout_truncated, true, 'stdout_truncated');
      expectEqual(r.stdout.length, 4096, 'stdout length capped at 4096');
    },
  },
  {
    name: 'stderr truncation flags at max_output_bytes',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'sh',
        args: ['-c', "printf 'y%.0s' $(seq 1 8000) 1>&2"],
      });
      expectEqual(r.exit_code, 0, 'exit_code');
      expectEqual(r.stderr_truncated, true, 'stderr_truncated');
      expectEqual(r.stderr.length, 4096, 'stderr length capped at 4096');
    },
  },
  {
    name: 'env scrubbing drops non-allowlisted vars',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'env' });
      expectEqual(r.exit_code, 0, 'exit_code');
      expect(
        !r.stdout.includes('HARNESS_NOT_ALLOWED'),
        `HARNESS_NOT_ALLOWED leaked through: ${r.stdout}`,
      );
    },
  },
  {
    name: 'env passthrough forwards allowlisted var',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'env' });
      expectEqual(r.exit_code, 0, 'exit_code');
      expect(
        r.stdout.includes('HARNESS_TEST_VAR=harness-allowed-value'),
        `HARNESS_TEST_VAR not forwarded: ${r.stdout}`,
      );
    },
  },
];
