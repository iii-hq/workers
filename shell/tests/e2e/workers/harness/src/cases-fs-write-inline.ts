import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';
import { fsReadStream } from './cases-fs-host.ts';

function newWorkdir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `iii-shell-fs-${prefix}-`));
}

// Any valid uuid works: the inline-content-on-sandbox rejection happens in the
// worker BEFORE the sandbox is ever dispatched, so no real VM (or mock) is hit.
const FAKE_SANDBOX_ID = '00000000-0000-4000-8000-000000000000';

// Exercises the inline-string and multi-file forms of shell::fs::write added in
// v0.4.0 — the path agents actually use (no streaming channel to construct),
// which the rest of the fs suite only covers via fsWriteStream/ContentRef.
export const FS_WRITE_INLINE_CASES: TestCase[] = [
  {
    name: 'fs_write_inline_string_creates_file',
    async run(ctx: CaseContext) {
      const root = newWorkdir('inline');
      const f = join(root, 'hello.txt');
      const r = await ctx.call('shell::fs::write', {
        path: f,
        content: 'hello inline\n',
        mode: '0600',
      });
      expectEqual(r.bytes_written, 13, 'bytes_written = inline byte length');
      expectEqual(r.path, f, 'path echoed');
      expect(
        Array.isArray(r.files) && r.files.length === 0,
        `single write leaves files empty: ${JSON.stringify(r.files)}`,
      );
      const back = await fsReadStream(ctx, { path: f });
      expectEqual(back.data.toString('utf8'), 'hello inline\n', 'inline content round-trips on disk');
    },
  },
  {
    name: 'fs_write_inline_default_mode_no_channel_needed',
    async run(ctx: CaseContext) {
      const root = newWorkdir('inline-default');
      const f = join(root, 'd.txt');
      const r = await ctx.call('shell::fs::write', { path: f, content: 'x' });
      expectEqual(r.bytes_written, 1, 'one byte written with default mode');
      const back = await fsReadStream(ctx, { path: f });
      expectEqual(back.data.toString('utf8'), 'x', 'content round-trips');
    },
  },
  {
    name: 'fs_write_batch_writes_all_files',
    async run(ctx: CaseContext) {
      const root = newWorkdir('batch');
      const a = join(root, 'a.txt');
      const b = join(root, 'b.txt');
      const r = await ctx.call('shell::fs::write', {
        files: [
          { path: a, content: 'A' },
          { path: b, content: 'B', mode: '0600' },
        ],
      });
      expectEqual(r.bytes_written, 2, 'batch bytes_written sums the files');
      expectEqual(r.path, '', 'batch leaves single-file path empty');
      expect(Array.isArray(r.files) && r.files.length === 2, `two file results: ${JSON.stringify(r.files)}`);
      expectEqual(r.files[0].path, a, 'first result path');
      expectEqual(r.files[0].bytes_written, 1, 'first result bytes');
      expectEqual(r.files[1].path, b, 'second result path');
      const ra = await fsReadStream(ctx, { path: a });
      const rb = await fsReadStream(ctx, { path: b });
      expectEqual(ra.data.toString('utf8'), 'A', 'file a content');
      expectEqual(rb.data.toString('utf8'), 'B', 'file b content');
    },
  },
  {
    name: 'fs_write_batch_single_entry_shape',
    async run(ctx: CaseContext) {
      const root = newWorkdir('batch1');
      const f = join(root, 'only.txt');
      const r = await ctx.call('shell::fs::write', {
        files: [{ path: f, content: 'solo' }],
      });
      expect(Array.isArray(r.files) && r.files.length === 1, `one file result: ${JSON.stringify(r.files)}`);
      expectEqual(r.files[0].bytes_written, 4, 'bytes for the single batch entry');
    },
  },
  {
    name: 'fs_write_rejects_both_single_and_files_s210',
    async run(ctx: CaseContext) {
      const root = newWorkdir('ambig');
      await ctx.expectError(
        () =>
          ctx.call('shell::fs::write', {
            path: join(root, 'x.txt'),
            content: 'x',
            files: [{ path: join(root, 'y.txt'), content: 'y' }],
          }),
        'S210',
      );
    },
  },
  {
    name: 'fs_write_rejects_empty_files_s210',
    async run(ctx: CaseContext) {
      await ctx.expectError(() => ctx.call('shell::fs::write', { files: [] }), 'S210');
    },
  },
  {
    name: 'fs_write_rejects_missing_content_s210',
    async run(ctx: CaseContext) {
      const root = newWorkdir('nocontent');
      await ctx.expectError(
        () => ctx.call('shell::fs::write', { path: join(root, 'z.txt') }),
        'S210',
      );
    },
  },
  {
    name: 'fs_write_inline_on_sandbox_rejects_s210',
    async run(ctx: CaseContext) {
      // Inline string content is host-only; on a sandbox target the worker
      // rejects with S210 before forwarding (no sandbox mock involved).
      await ctx.expectError(
        () =>
          ctx.call('shell::fs::write', {
            target: { kind: 'sandbox', sandbox_id: FAKE_SANDBOX_ID },
            path: '/sb/x',
            content: 'inline not allowed on sandbox',
          }),
        'S210',
      );
    },
  },
];
