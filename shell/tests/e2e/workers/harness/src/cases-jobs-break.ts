import { expect, type TestCase } from './cases.ts';

export const JOB_BREAK_CASES: TestCase[] = [
  {
    name: 'job_status_unknown_id_rejects',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::status', { job_id: 'job-does-not-exist' }),
        'no such job',
      );
    },
  },
  {
    name: 'job_status_empty_id_rejects',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::status', { job_id: '' }),
        'no such job',
      );
    },
  },
  {
    name: 'job_status_id_wrong_type_rejects',
    async run({ call }) {
      let threw = false;
      try {
        await call('shell::status', { job_id: 42 });
      } catch {
        threw = true;
      }
      expect(threw, 'numeric job_id must be rejected');
    },
  },
  {
    name: 'job_kill_twice_second_is_idempotent_no_op',
    async run({ call, sleep }) {
      const spawned = await call('shell::exec_bg', { command: 'sleep', args: ['5'] });
      const jobId = spawned.job_id;
      const first = await call('shell::kill', { job_id: jobId });
      expect(first.killed === true, `first kill returns killed:true (got ${JSON.stringify(first.killed)})`);
      await sleep(300);
      const second = await call('shell::kill', { job_id: jobId });
      expect(
        second.killed === false,
        `second kill returns killed:false (got ${JSON.stringify(second.killed)})`,
      );
      expect(
        typeof second.reason === 'string',
        `second kill includes a reason field (got ${JSON.stringify(second.reason)})`,
      );
    },
  },
  {
    name: 'job_kill_already_finished_is_no_op',
    async run({ call, sleep }) {
      const spawned = await call('shell::exec_bg', {
        command: 'sh',
        args: ['-c', 'echo bye'],
      });
      const jobId = spawned.job_id;
      await sleep(500);
      const r = await call('shell::kill', { job_id: jobId });
      expect(
        r.killed === false,
        `kill on finished job returns killed:false (got ${JSON.stringify(r.killed)})`,
      );
      expect(
        r.status === 'finished' || r.status === 'killed',
        `kill on finished job reports terminal status (got ${JSON.stringify(r.status)})`,
      );
    },
  },
  {
    name: 'job_exec_bg_concurrent_flood_respects_cap',
    async run({ call, sleep }) {
      const promises = Array.from({ length: 5 }, () =>
        call('shell::exec_bg', { command: 'sleep', args: ['5'] }),
      );
      const settled = await Promise.allSettled(promises);
      const succeeded = settled.filter((s) => s.status === 'fulfilled');
      const rejected = settled.filter((s) => s.status === 'rejected');
      expect(
        succeeded.length <= 2,
        `at most 2 concurrent spawns should succeed (got ${succeeded.length})`,
      );
      expect(
        rejected.length >= 3,
        `at least 3 must reject under cap (got ${rejected.length})`,
      );
      const sawCapMessage = rejected.some((r) => {
        const msg = (r as PromiseRejectedResult).reason?.message ?? String((r as PromiseRejectedResult).reason);
        return msg.includes('max concurrent jobs');
      });
      expect(sawCapMessage, 'at least one rejection must mention the cap');
      for (const s of succeeded) {
        const jobId = (s as PromiseFulfilledResult<{ job_id: string }>).value.job_id;
        await call('shell::kill', { job_id: jobId }).catch(() => {});
      }
      await sleep(300);
    },
  },
  {
    name: 'job_list_after_retention_window_is_empty',
    async run({ call, sleep }) {
      const spawned = await call('shell::exec_bg', {
        command: 'sh',
        args: ['-c', 'echo done'],
      });
      await sleep(500);
      await call('shell::list', {});
      await sleep(2500);
      const after = await call('shell::list', {});
      const ids = (after.jobs ?? after.records ?? []).map((j: { id: string }) => j.id);
      expect(
        !ids.includes(spawned.job_id),
        `${spawned.job_id} must be pruned after retention window (got ${JSON.stringify(ids)})`,
      );
    },
  },
];
