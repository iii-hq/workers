import { expectEqual, type TestCase } from './cases.ts';

// Any valid uuid works: stdin-on-sandbox is rejected in the worker before the
// sandbox backend touches a VM (same is_empty() gate as cwd/env).
const FAKE_SANDBOX_ID = '00000000-0000-4000-8000-000000000000';

// Covers the per-call `stdin` field added to shell::exec / shell::exec_bg in
// v0.4.0 — the agent-friendly alternative to a shell heredoc. `cat` is in the
// e2e allowlist and echoes stdin to stdout, so it's the natural probe.
export const EXEC_STDIN_CASES: TestCase[] = [
  {
    name: 'exec_stdin_piped_to_cat',
    async run({ call }) {
      const r = await call('shell::exec', { command: 'cat', stdin: 'hello stdin' });
      expectEqual(r.exit_code, 0, 'cat exits 0');
      expectEqual(r.stdout, 'hello stdin', 'cat echoes stdin to stdout');
      expectEqual(r.stderr, '', 'no stderr');
    },
  },
  {
    name: 'exec_stdin_omitted_is_closed',
    async run({ call }) {
      // No stdin -> /dev/null: cat sees immediate EOF, exits 0 with no output
      // (and does NOT hang waiting for input).
      const r = await call('shell::exec', { command: 'cat' });
      expectEqual(r.exit_code, 0, 'cat with closed stdin exits 0');
      expectEqual(r.stdout, '', 'no stdin -> empty stdout');
    },
  },
  {
    name: 'exec_bg_stdin_piped',
    async run({ call, sleep }) {
      const spawned = await call('shell::exec_bg', { command: 'cat', stdin: 'bg stdin' });
      const jobId = spawned.job_id;
      await sleep(500);
      const done = await call('shell::status', { job_id: jobId });
      expectEqual(done.job.status, 'finished', 'bg cat finished');
      expectEqual(done.job.stdout, 'bg stdin', 'bg cat echoed its stdin');
    },
  },
  {
    name: 'exec_stdin_on_sandbox_rejects_s210',
    async run({ call, expectError }) {
      await expectError(
        () =>
          call('shell::exec', {
            command: 'cat',
            stdin: 'x',
            target: { kind: 'sandbox', sandbox_id: FAKE_SANDBOX_ID },
          }),
        'S210',
      );
    },
  },
];
