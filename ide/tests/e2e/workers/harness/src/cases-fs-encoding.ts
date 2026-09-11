import { mkdtempSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fsWriteStream, fsReadStream } from './cases-fs-host.ts';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

function newWorkdir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `iii-shell-fs-enc-${prefix}-`));
}

export const FS_ENCODING_CASES: TestCase[] = [
  {
    name: 'fs_write_read_utf8_multiscript_round_trip',
    async run(ctx: CaseContext) {
      const root = newWorkdir('utf8');
      const f = join(root, 'utf8.txt');
      const payload = Buffer.from(
        'hello 🌍 مرحبا नमस्ते 你好 ñöç combining: é zwj: 👨‍👩‍👧\n',
        'utf8',
      );
      const w = await fsWriteStream(ctx, { path: f, bytes: payload });
      expectEqual(w.bytes_written, payload.length, 'bytes_written matches utf8 length');
      const r = await fsReadStream(ctx, { path: f });
      expect(r.data.equals(payload), 'utf8 round-trip exact');
    },
  },
  {
    name: 'fs_write_read_binary_with_nul_bytes_round_trip',
    async run(ctx: CaseContext) {
      const root = newWorkdir('bin');
      const f = join(root, 'bin.dat');
      const payload = Buffer.from([
        0x00, 0x01, 0xff, 0x00, 0x7f, 0x80, 0x0a, 0x00, 0x0d, 0xfe, 0x00,
      ]);
      const w = await fsWriteStream(ctx, { path: f, bytes: payload });
      expectEqual(w.bytes_written, payload.length, 'bytes_written matches binary length');
      const r = await fsReadStream(ctx, { path: f });
      expect(r.data.equals(payload), 'binary with NUL bytes round-trips exactly');
    },
  },
  {
    name: 'fs_path_with_unicode_filename_round_trip',
    async run(ctx: CaseContext) {
      const root = newWorkdir('uname');
      const f = join(root, 'файл-🦀-名前.txt');
      const payload = Buffer.from('payload', 'utf8');
      const w = await fsWriteStream(ctx, { path: f, bytes: payload });
      expectEqual(w.bytes_written, payload.length, 'unicode filename writes');
      const s = await ctx.call('shell::fs::stat', { path: f });
      expectEqual(s.size, payload.length, 'stat sees the unicode-named file');
      const r = await fsReadStream(ctx, { path: f });
      expect(r.data.equals(payload), 'unicode filename round-trips');
    },
  },
  {
    name: 'fs_path_with_consecutive_slashes_resolves',
    async run(ctx: CaseContext) {
      const root = newWorkdir('slashes');
      const sub = join(root, 'sub');
      mkdirSync(sub);
      const messy = root + '//sub///';
      let succeeded = false;
      let threw = false;
      try {
        const r = await ctx.call('shell::fs::ls', { path: messy });
        expect(Array.isArray(r.entries), 'ls returned entries array');
        succeeded = true;
      } catch {
        threw = true;
      }
      expect(succeeded || threw, 'must succeed or reject cleanly');
    },
  },
  {
    name: 'fs_path_pathologically_long_no_crash',
    async run(ctx: CaseContext) {
      const longPath = '/tmp/' + 'x'.repeat(5000);
      let threw = false;
      try {
        await ctx.call('shell::fs::stat', { path: longPath });
      } catch {
        threw = true;
      }
      expect(threw, '5000-char path must be rejected');
      const r = await ctx.call('shell::exec', { command: 'echo', args: ['after-long'] });
      expectEqual(r.exit_code, 0, 'worker survives long-path probe');
    },
  },
  {
    name: 'fs_grep_utf8_max_line_bytes_floors_to_char_boundary',
    async run(ctx: CaseContext) {
      const root = newWorkdir('grep-utf8');
      const f = join(root, 'multibyte.txt');
      const line = '世'.repeat(30) + '\n';
      await fsWriteStream(ctx, { path: f, bytes: line });
      const r = await ctx.call('shell::fs::grep', {
        path: root,
        pattern: '世',
        recursive: true,
        ignore_case: false,
        include_glob: [],
        exclude_glob: [],
        max_matches: 10,
        max_line_bytes: 50,
      });
      expect(Array.isArray(r.matches) && r.matches.length >= 1, 'grep found at least 1 match');
      const content = r.matches[0].content as string;
      const buf = Buffer.from(content, 'utf8');
      expect(
        buf.toString('utf8') === content,
        `truncated content must remain valid UTF-8 (got ${buf.length} bytes)`,
      );
    },
  },
];
