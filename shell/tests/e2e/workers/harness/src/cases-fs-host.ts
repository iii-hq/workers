import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ChannelReader } from 'iii-sdk';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

const URL = process.env.III_URL ?? 'ws://127.0.0.1:49134';

function newWorkdir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `iii-shell-fs-${prefix}-`));
}

export async function fsWriteStream(
  ctx: CaseContext,
  args: {
    path: string;
    mode?: string;
    parents?: boolean;
    bytes: Buffer | string;
    target?: object;
  },
): Promise<{ bytes_written: number; path: string }> {
  const bytes =
    typeof args.bytes === 'string' ? Buffer.from(args.bytes, 'utf8') : args.bytes;
  const channel = await ctx.iii.createChannel(64);
  if (bytes.length === 0) {
    // SDK quirk: write(empty) doesn't trigger WS connect, making close()
    // a no-op. sendMessage('') forces the socket open.
    channel.writer.sendMessage('');
    channel.writer.close();
    return (await ctx.call('shell::fs::write', {
      target: args.target ?? { kind: 'host' },
      path: args.path,
      mode: args.mode ?? '0644',
      parents: args.parents ?? false,
      content: channel.readerRef,
    })) as { bytes_written: number; path: string };
  }
  const writePromise = new Promise<void>((resolve, reject) => {
    channel.writer.stream.once('error', reject);
    channel.writer.stream.write(bytes, (err: Error | null | undefined) => {
      if (err) {
        reject(err);
        return;
      }
      channel.writer.stream.end(() => resolve());
    });
  });
  const triggerPromise = ctx.call('shell::fs::write', {
    target: args.target ?? { kind: 'host' },
    path: args.path,
    mode: args.mode ?? '0644',
    parents: args.parents ?? false,
    content: channel.readerRef,
  });
  await writePromise;
  channel.writer.close();
  return (await triggerPromise) as { bytes_written: number; path: string };
}

export async function fsReadStream(
  ctx: CaseContext,
  args: { path: string; target?: object },
): Promise<{ size: number; mode: string; mtime: number; data: Buffer }> {
  const resp = await ctx.call('shell::fs::read', {
    target: args.target ?? { kind: 'host' },
    path: args.path,
  });
  const reader = new ChannelReader(URL, resp.content);
  const data = await reader.readAll();
  return { size: resp.size, mode: resp.mode, mtime: resp.mtime, data };
}

export const FS_HOST_CASES: TestCase[] = [
  {
    name: 'fs_host_mkdir_then_ls_finds_dir',
    async run(ctx: CaseContext) {
      const root = newWorkdir('mkdir');
      const child = join(root, 'sub');
      const m = await ctx.call('shell::fs::mkdir', { path: child, mode: '0755' });
      expect(m.created === true, `mkdir.created=${m.created}`);
      const ls = await ctx.call('shell::fs::ls', { path: root });
      expect(
        Array.isArray(ls.entries) && ls.entries.some((e: any) => e.name === 'sub' && e.is_dir),
        `ls did not show sub: ${JSON.stringify(ls)}`,
      );
    },
  },
  {
    name: 'fs_host_stat_reports_size',
    async run(ctx: CaseContext) {
      const root = newWorkdir('stat');
      const f = join(root, 'a.txt');
      await fsWriteStream(ctx, { path: f, bytes: 'hello' });
      const s = await ctx.call('shell::fs::stat', { path: f });
      expectEqual(s.size, 5, 'stat.size');
      expect(s.is_dir === false, `is_dir=${s.is_dir}`);
    },
  },
  {
    name: 'fs_host_write_then_read_round_trips',
    async run(ctx: CaseContext) {
      const root = newWorkdir('rw');
      const f = join(root, 'r.txt');
      const payload = 'round trip 123';
      await fsWriteStream(ctx, { path: f, mode: '0600', bytes: payload });
      const r = await fsReadStream(ctx, { path: f });
      expectEqual(r.data.toString('utf8'), payload, 'streamed read content');
    },
  },
  {
    name: 'fs_host_rm_file',
    async run(ctx: CaseContext) {
      const root = newWorkdir('rm');
      const f = join(root, 'doomed.txt');
      await fsWriteStream(ctx, { path: f, bytes: 'bye' });
      const r = await ctx.call('shell::fs::rm', { path: f, recursive: false });
      expect(r.removed === true, `rm.removed=${r.removed}`);
      await ctx.expectError(() => ctx.call('shell::fs::stat', { path: f }), 'S211');
    },
  },
  {
    name: 'fs_host_chmod_sets_mode',
    async run(ctx: CaseContext) {
      const root = newWorkdir('chmod');
      const f = join(root, 'p.txt');
      await fsWriteStream(ctx, { path: f, bytes: 'x' });
      const c = await ctx.call('shell::fs::chmod', {
        path: f,
        mode: '0600',
        recursive: false,
      });
      expectEqual(c.entries_changed, 1, 'chmod.entries_changed');
      expectEqual(c.path, f, 'chmod.path');
      const s = await ctx.call('shell::fs::stat', { path: f });
      expectEqual(s.mode, '0600', 'stat.mode after chmod');
    },
  },
  {
    name: 'fs_host_mv_renames',
    async run(ctx: CaseContext) {
      const root = newWorkdir('mv');
      const a = join(root, 'a');
      const b = join(root, 'b');
      await fsWriteStream(ctx, { path: a, bytes: 'm' });
      const m = await ctx.call('shell::fs::mv', { src: a, dst: b, overwrite: false });
      expect(m.moved === true, `mv.moved=${m.moved}`);
      await ctx.expectError(() => ctx.call('shell::fs::stat', { path: a }), 'S211');
      const s = await ctx.call('shell::fs::stat', { path: b });
      expectEqual(s.size, 1, 'after mv stat.size');
    },
  },
  {
    name: 'fs_host_grep_finds_lines',
    async run(ctx: CaseContext) {
      const root = newWorkdir('grep');
      const f = join(root, 'g.txt');
      await fsWriteStream(ctx, { path: f, bytes: 'alpha\nbeta\nalpha-beta\n' });
      const g = await ctx.call('shell::fs::grep', {
        path: root,
        pattern: 'alpha',
        recursive: true,
        ignore_case: false,
        include_glob: [],
        exclude_glob: [],
        max_matches: 10,
        max_line_bytes: 8192,
      });
      expectEqual(g.matches.length, 2, 'grep.matches.length');
      expect(!g.truncated, 'grep.truncated should be false');
    },
  },
  {
    name: 'fs_host_sed_replaces',
    async run(ctx: CaseContext) {
      const root = newWorkdir('sed');
      const f = join(root, 's.txt');
      await fsWriteStream(ctx, { path: f, bytes: 'hello hello' });
      const s = await ctx.call('shell::fs::sed', {
        files: [f],
        recursive: false,
        include_glob: [],
        exclude_glob: [],
        pattern: 'hello',
        replacement: 'HI',
        regex: false,
        first_only: false,
        ignore_case: false,
      });
      expectEqual(s.total_replacements, 2, 'sed.total_replacements');
      const r = await fsReadStream(ctx, { path: f });
      expectEqual(r.data.toString('utf8'), 'HI HI', 'sed final content');
    },
  },
  {
    name: 'fs_host_target_host_is_default',
    async run(ctx: CaseContext) {
      const root = newWorkdir('default');
      await ctx.call('shell::fs::mkdir', { path: join(root, 'd'), mode: '0755' });
      const ls = await ctx.call('shell::fs::ls', { path: root });
      expect(
        ls.entries.some((e: any) => e.name === 'd'),
        'default target=host should have created/listed dir',
      );
    },
  },
  {
    name: 'fs_host_explicit_target_host',
    async run(ctx: CaseContext) {
      const root = newWorkdir('explicit');
      await ctx.call('shell::fs::mkdir', {
        target: { kind: 'host' },
        path: join(root, 'e'),
        mode: '0755',
      });
      const ls = await ctx.call('shell::fs::ls', {
        target: { kind: 'host' },
        path: root,
      });
      expect(ls.entries.some((e: any) => e.name === 'e'), 'explicit host target works');
    },
  },
  {
    name: 'fs_host_write_streams_512kib_without_buffering',
    async run(ctx: CaseContext) {
      const root = newWorkdir('big');
      const f = join(root, 'big.bin');
      const big = Buffer.alloc(512 * 1024);
      for (let i = 0; i < big.length; i++) big[i] = i & 0xff;
      const w = await fsWriteStream(ctx, { path: f, bytes: big });
      expectEqual(w.bytes_written, big.length, 'bytes_written matches 512 KiB');
      const r = await fsReadStream(ctx, { path: f });
      expectEqual(r.size, big.length, 'read.size matches');
      expect(r.data.equals(big), 'streamed bytes round-trip exactly');
    },
  },
  {
    name: 'fs_host_read_succeeds_with_cap_unset',
    async run(ctx: CaseContext) {
      const root = newWorkdir('cap-off');
      const f = join(root, 'medium.bin');
      const buf = Buffer.alloc(2048);
      await fsWriteStream(ctx, { path: f, bytes: buf });
      const r = await fsReadStream(ctx, { path: f });
      expectEqual(r.size, 2048, 'read works with cap unset');
    },
  },
  {
    name: 'fs_host_write_empty_stream_creates_zero_byte_file',
    async run(ctx: CaseContext) {
      const root = newWorkdir('empty');
      const f = join(root, 'empty.bin');
      const w = await fsWriteStream(ctx, { path: f, bytes: Buffer.alloc(0) });
      expectEqual(w.bytes_written, 0, 'bytes_written = 0');
      const r = await fsReadStream(ctx, { path: f });
      expectEqual(r.size, 0, 'file exists with size 0');
      expectEqual(r.data.length, 0, 'streamed body is empty');
    },
  },
];
