import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

// Poll shell::status until the job leaves `running`, or the deadline passes
// (in which case the still-running job is returned so the assertion can fail
// loudly — e.g. if max_bg_timeout_ms never took effect).
async function awaitTerminal(ctx: CaseContext, jobId: string, deadlineMs: number): Promise<any> {
  const start = Date.now();
  for (;;) {
    const s = await ctx.call('shell::status', { job_id: jobId });
    if (s.job.status !== 'running') return s.job;
    if (Date.now() - start >= deadlineMs) return s.job;
    await ctx.sleep(500);
  }
}

// Covers the host bg-job hard cap added in v0.4.0: `max_bg_timeout_ms` bounds
// background jobs SEPARATELY from the foreground `max_timeout_ms`. The e2e
// config sets max_bg_timeout_ms: 6000 > max_timeout_ms: 5000, so a single
// cap test also proves the regression fix (a bg job is no longer killed at the
// 5s foreground cap — it survives to its own 6s bg cap).
export const JOBS_BG_TIMEOUT_CASES: TestCase[] = [
  {
    name: 'bg_job_killed_at_max_bg_timeout_ms',
    async run(ctx: CaseContext) {
      const spawned = await ctx.call('shell::exec_bg', { command: 'sleep', args: ['20'] });
      const job = await awaitTerminal(ctx, spawned.job_id, 12_000);
      expectEqual(job.status, 'killed', 'bg job killed at its hard cap (and survived past the 5s foreground cap)');
      expect(
        typeof job.stderr === 'string' && job.stderr.includes('hard cap'),
        `stderr names the hard cap: ${JSON.stringify(job.stderr)}`,
      );
    },
  },
  {
    name: 'bg_job_under_bg_cap_completes',
    async run(ctx: CaseContext) {
      const spawned = await ctx.call('shell::exec_bg', {
        command: 'sh',
        args: ['-c', 'sleep 0.3; echo done'],
      });
      const job = await awaitTerminal(ctx, spawned.job_id, 4_000);
      expectEqual(job.status, 'finished', 'under-cap bg job finishes');
      expectEqual(job.stdout, 'done\n', 'stdout captured');
    },
  },
];
