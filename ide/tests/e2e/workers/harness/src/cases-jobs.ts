import type { TestCase } from './cases.ts';
import { expect, expectEqual } from './cases.ts';

export const JOB_CASES: TestCase[] = [
  {
    name: 'exec_bg returns job_id and argv',
    async run({ call, sleep }) {
      const r = await call('shell::exec_bg', {
        command: 'sh',
        args: ['-c', 'sleep 0.1; echo done'],
      });
      expect(typeof r.job_id === 'string' && r.job_id.length > 0, 'job_id present');
      expectEqual(r.argv, ['sh', '-c', 'sleep 0.1; echo done'], 'argv echoed');
      await sleep(500);
    },
  },
  {
    name: 'status reports running then finished with stdout',
    async run({ call, sleep }) {
      const spawned = await call('shell::exec_bg', {
        command: 'sh',
        args: ['-c', 'sleep 0.3; echo done'],
      });
      const jobId = spawned.job_id;

      const running = await call('shell::status', { job_id: jobId });
      expectEqual(running.job.status, 'running', 'status running');

      await sleep(800);

      const finished = await call('shell::status', { job_id: jobId });
      expectEqual(finished.job.status, 'finished', 'status finished');
      expectEqual(finished.job.exit_code, 0, 'exit_code 0');
      expectEqual(finished.job.stdout, 'done\n', 'stdout captured');
    },
  },
  {
    name: 'kill marks running job as killed',
    async run({ call, sleep }) {
      const spawned = await call('shell::exec_bg', { command: 'sleep', args: ['5'] });
      const jobId = spawned.job_id;
      await call('shell::kill', { job_id: jobId });
      await sleep(300);
      const after = await call('shell::status', { job_id: jobId });
      expectEqual(after.job.status, 'killed', 'status killed');
    },
  },
  {
    name: 'kill on unknown job_id rejects',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::kill', { job_id: 'job-does-not-exist' }),
        'no such job',
      );
    },
  },
  {
    name: 'list includes spawned job',
    async run({ call, sleep }) {
      const spawned = await call('shell::exec_bg', {
        command: 'sh',
        args: ['-c', 'sleep 0.1'],
      });
      const list = await call('shell::list', {});
      const ids = (list.jobs ?? list.records ?? []).map((j: any) => j.id);
      expect(
        Array.isArray(ids) && ids.includes(spawned.job_id),
        `list missing job_id ${spawned.job_id}; got ${JSON.stringify(ids)}`,
      );
      await sleep(500);
    },
  },
  {
    name: 'max_concurrent_jobs cap rejects third spawn',
    async run({ call, sleep, expectError }) {
      const a = await call('shell::exec_bg', { command: 'sleep', args: ['5'] });
      const b = await call('shell::exec_bg', { command: 'sleep', args: ['5'] });
      try {
        await expectError(
          () => call('shell::exec_bg', { command: 'sleep', args: ['5'] }),
          'max concurrent jobs',
        );
      } finally {
        await call('shell::kill', { job_id: a.job_id }).catch(() => {});
        await call('shell::kill', { job_id: b.job_id }).catch(() => {});
        await sleep(300);
      }
    },
  },
  {
    name: 'finished jobs prune after job_retention_secs',
    async run({ call, sleep }) {
      const spawned = await call('shell::exec_bg', {
        command: 'sh',
        args: ['-c', 'echo bye'],
      });
      const jobId = spawned.job_id;
      await sleep(500);
      const before = await call('shell::list', {});
      const idsBefore = (before.jobs ?? before.records ?? []).map((j: any) => j.id);
      expect(idsBefore.includes(jobId), 'job present before retention window');

      await sleep(2500);
      const after = await call('shell::list', {});
      const idsAfter = (after.jobs ?? after.records ?? []).map((j: any) => j.id);
      expect(!idsAfter.includes(jobId), `job ${jobId} not pruned: got ${JSON.stringify(idsAfter)}`);
    },
  },
];
