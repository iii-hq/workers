import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fsWriteStream } from './cases-fs-host.ts';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

function newWorkdir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `iii-shell-fs-err-${prefix}-`));
}

export const FS_ERROR_CASES: TestCase[] = [
  {
    name: 'fs_rm_nonexistent_returns_s211',
    async run({ call, expectError }) {
      const root = newWorkdir('rm-404');
      await expectError(
        () => call('shell::fs::rm', {
          path: join(root, 'never-existed'),
          recursive: false,
        }),
        'S211',
      );
    },
  },
  {
    name: 'fs_rm_non_empty_dir_without_recursive_returns_s214',
    async run(ctx: CaseContext) {
      const root = newWorkdir('rm-full');
      const dir = join(root, 'has-files');
      await ctx.call('shell::fs::mkdir', { path: dir, mode: '0755' });
      await fsWriteStream(ctx, { path: join(dir, 'kid.txt'), bytes: 'hi' });
      await ctx.expectError(
        () => ctx.call('shell::fs::rm', { path: dir, recursive: false }),
        'S214',
      );
      await ctx.call('shell::fs::rm', { path: dir, recursive: true });
    },
  },
  {
    name: 'fs_mkdir_existing_without_parents_returns_s213',
    async run({ call, expectError }) {
      const root = newWorkdir('mkdir-dup');
      const dir = join(root, 'd');
      await call('shell::fs::mkdir', { path: dir, mode: '0755' });
      await expectError(
        () => call('shell::fs::mkdir', { path: dir, mode: '0755', parents: false }),
        'S213',
      );
    },
  },
  {
    name: 'fs_mkdir_existing_with_parents_is_idempotent',
    async run({ call }) {
      const root = newWorkdir('mkdir-idem');
      const dir = join(root, 'd');
      const first = await call('shell::fs::mkdir', { path: dir, mode: '0755', parents: true });
      expectEqual(first.created, true, 'first mkdir creates the dir');
      const second = await call('shell::fs::mkdir', { path: dir, mode: '0755', parents: true });
      expect(
        second.created === false || second.created === true,
        `second mkdir.created is a bool (got ${JSON.stringify(second.created)})`,
      );
    },
  },
  {
    name: 'fs_mv_overwrite_false_collision_returns_s213',
    async run(ctx: CaseContext) {
      const root = newWorkdir('mv-coll');
      const src = join(root, 'src.txt');
      const dst = join(root, 'dst.txt');
      await fsWriteStream(ctx, { path: src, bytes: 'src-content' });
      await fsWriteStream(ctx, { path: dst, bytes: 'dst-content' });
      await ctx.expectError(
        () => ctx.call('shell::fs::mv', { src, dst, overwrite: false }),
        'S213',
      );
    },
  },
  {
    name: 'fs_chmod_nonexistent_returns_s211',
    async run({ call, expectError }) {
      const root = newWorkdir('chmod-404');
      await expectError(
        () => call('shell::fs::chmod', {
          path: join(root, 'missing'),
          mode: '0644',
          recursive: false,
        }),
        'S211',
      );
    },
  },
  {
    name: 'fs_stat_on_directory_reports_is_dir_true',
    async run({ call }) {
      const root = newWorkdir('stat-dir');
      const dir = join(root, 'subdir');
      await call('shell::fs::mkdir', { path: dir, mode: '0755' });
      const s = await call('shell::fs::stat', { path: dir });
      expectEqual(s.is_dir, true, 'is_dir true for a directory');
      expectEqual(s.is_symlink, false, 'is_symlink false for a real dir');
    },
  },
];
