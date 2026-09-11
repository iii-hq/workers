import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fsWriteStream, fsReadStream } from './cases-fs-host.ts';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

function newWorkdir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `iii-shell-conc-${prefix}-`));
}

export const CONCURRENCY_CASES: TestCase[] = [
  {
    name: 'concurrency_two_writes_same_path_one_wins',
    async run(ctx: CaseContext) {
      const root = newWorkdir('two-write');
      const f = join(root, 'race.bin');
      const a = Buffer.from('A'.repeat(2048));
      const b = Buffer.from('B'.repeat(2048));
      const [ra, rb] = await Promise.allSettled([
        fsWriteStream(ctx, { path: f, bytes: a }),
        fsWriteStream(ctx, { path: f, bytes: b }),
      ]);
      expect(
        ra.status === 'fulfilled' && rb.status === 'fulfilled',
        `both writes must complete (a=${ra.status}, b=${rb.status})`,
      );
      const final = await fsReadStream(ctx, { path: f });
      expect(
        final.data.equals(a) || final.data.equals(b),
        'final content must equal exactly one of the two payloads',
      );
    },
  },
  {
    name: 'concurrency_list_during_job_churn',
    async run({ call, sleep }) {
      const a = await call('shell::exec_bg', { command: 'sleep', args: ['1'] });
      const b = await call('shell::exec_bg', { command: 'sleep', args: ['1'] });
      const lists = await Promise.allSettled(
        Array.from({ length: 10 }, () => call('shell::list', {})),
      );
      const fulfilled = lists.filter((s) => s.status === 'fulfilled') as PromiseFulfilledResult<{
        jobs?: Array<{ id: string }>;
        records?: Array<{ id: string }>;
      }>[];
      expectEqual(fulfilled.length, 10, 'all 10 list calls succeed');
      for (const f of fulfilled) {
        const jobs = f.value.jobs ?? f.value.records ?? [];
        expect(
          Array.isArray(jobs) && jobs.length <= 2,
          `each list returns 0..2 jobs (got ${jobs.length})`,
        );
      }
      await call('shell::kill', { job_id: a.job_id }).catch(() => {});
      await call('shell::kill', { job_id: b.job_id }).catch(() => {});
      await sleep(300);
    },
  },
  {
    name: 'concurrency_concurrent_reads_same_file',
    async run(ctx: CaseContext) {
      const root = newWorkdir('many-read');
      const f = join(root, 'shared.bin');
      const payload = Buffer.alloc(64 * 1024);
      for (let i = 0; i < payload.length; i++) payload[i] = (i * 7) & 0xff;
      await fsWriteStream(ctx, { path: f, bytes: payload });
      const reads = await Promise.all(
        Array.from({ length: 5 }, () => fsReadStream(ctx, { path: f })),
      );
      for (const r of reads) {
        expect(r.data.equals(payload), 'every concurrent read returns identical bytes');
      }
    },
  },
];
