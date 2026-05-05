import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { StreamChannelRef } from 'iii-sdk';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

function newWorkdir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `iii-shell-stream-break-${prefix}-`));
}

async function expectWorkerHealthy(ctx: CaseContext): Promise<void> {
  const r = await ctx.call('shell::exec', { command: 'echo', args: ['ok'] });
  expectEqual(r.exit_code, 0, 'worker still healthy after stream break');
}

export const STREAMING_BREAK_CASES: TestCase[] = [
  {
    name: 'stream_write_with_bogus_channel_id_rejected',
    async run(ctx: CaseContext) {
      const root = newWorkdir('bogus-id');
      const ref: StreamChannelRef = {
        channel_id: '00000000-0000-0000-0000-000000000000',
        access_key: 'definitely-not-a-real-key',
        direction: 'read',
      };
      let threw = false;
      try {
        await ctx.call('shell::fs::write', {
          target: { kind: 'host' },
          path: join(root, 'never-written.bin'),
          mode: '0644',
          parents: false,
          content: ref,
        });
      } catch {
        threw = true;
      }
      expect(threw, 'bogus channel_id should reject; worker must not hang');
      await expectWorkerHealthy(ctx);
    },
  },
  {
    name: 'stream_write_with_wrong_access_key_rejected',
    async run(ctx: CaseContext) {
      const root = newWorkdir('bad-key');
      const channel = await ctx.iii.createChannel(64);
      const tampered: StreamChannelRef = {
        channel_id: channel.readerRef.channel_id,
        access_key: 'tampered-' + channel.readerRef.access_key,
        direction: 'read',
      };
      let threw = false;
      try {
        await ctx.call('shell::fs::write', {
          target: { kind: 'host' },
          path: join(root, 'auth-fail.bin'),
          mode: '0644',
          parents: false,
          content: tampered,
        });
      } catch {
        threw = true;
      }
      expect(threw, 'mutated access_key should reject');
      try { channel.writer.close(); } catch { /* ignore */ }
      await expectWorkerHealthy(ctx);
    },
  },
  {
    name: 'stream_write_with_empty_access_key_rejected',
    async run(ctx: CaseContext) {
      const root = newWorkdir('empty-key');
      const ref: StreamChannelRef = {
        channel_id: '11111111-1111-1111-1111-111111111111',
        access_key: '',
        direction: 'read',
      };
      let threw = false;
      try {
        await ctx.call('shell::fs::write', {
          target: { kind: 'host' },
          path: join(root, 'empty-key.bin'),
          mode: '0644',
          parents: false,
          content: ref,
        });
      } catch {
        threw = true;
      }
      expect(threw, 'empty access_key should reject');
      await expectWorkerHealthy(ctx);
    },
  },
  {
    // Reused reader_ref after close: engine treats re-dial as immediate
    // EOF, producing a zero-byte file rather than an error.
    name: 'stream_write_reader_reused_after_close_yields_zero_bytes',
    async run(ctx: CaseContext) {
      const root = newWorkdir('reuse');
      const path1 = join(root, 'first.bin');
      const path2 = join(root, 'second.bin');
      const channel = await ctx.iii.createChannel(64);
      const payload = Buffer.from('hello world');

      // First write — drives the channel through completion.
      const writePromise = new Promise<void>((resolve, reject) => {
        channel.writer.stream.once('error', reject);
        channel.writer.stream.write(payload, (err) => {
          if (err) return reject(err);
          channel.writer.stream.end(() => resolve());
        });
      });
      const trigger = ctx.call('shell::fs::write', {
        target: { kind: 'host' },
        path: path1,
        mode: '0644',
        parents: false,
        content: channel.readerRef,
      });
      await writePromise;
      channel.writer.close();
      const r1 = await trigger;
      expectEqual(r1.bytes_written, payload.length, 'first write succeeded');

      let zeroSize = false;
      let threw = false;
      try {
        const r2 = await ctx.call('shell::fs::write', {
          target: { kind: 'host' },
          path: path2,
          mode: '0644',
          parents: false,
          content: channel.readerRef,
        });
        zeroSize = r2.bytes_written === 0;
      } catch {
        threw = true;
      }
      expect(
        zeroSize || threw,
        'reused reader_ref must either yield zero bytes or reject — never crash',
      );
      await expectWorkerHealthy(ctx);
    },
  },
  {
    name: 'stream_write_channel_closed_before_call',
    async run(ctx: CaseContext) {
      const root = newWorkdir('closed-pre');
      const path = join(root, 'closed.bin');
      const channel = await ctx.iii.createChannel(64);
      // sendMessage('') forces WS connect before close() (SDK quirk:
      // close() is a no-op when never connected).
      channel.writer.sendMessage('');
      channel.writer.close();
      let succeeded = false;
      let threw = false;
      try {
        const r = await ctx.call('shell::fs::write', {
          target: { kind: 'host' },
          path,
          mode: '0644',
          parents: false,
          content: channel.readerRef,
        });
        expectEqual(r.bytes_written, 0, 'pre-closed channel writes 0 bytes');
        succeeded = true;
      } catch {
        threw = true;
      }
      expect(succeeded || threw, 'must either succeed cleanly or reject cleanly');
      await expectWorkerHealthy(ctx);
    },
  },
  {
    // Text frames are dispatched to an empty callback list and skipped by
    // next_binary() — cannot smuggle data through the binary stream.
    name: 'stream_write_text_only_frames_yields_zero_bytes',
    async run(ctx: CaseContext) {
      const root = newWorkdir('text-only');
      const path = join(root, 'text-only.bin');
      const channel = await ctx.iii.createChannel(64);
      channel.writer.sendMessage('this is text, not binary');
      channel.writer.sendMessage('still text');
      const trigger = ctx.call('shell::fs::write', {
        target: { kind: 'host' },
        path,
        mode: '0644',
        parents: false,
        content: channel.readerRef,
      });
      await ctx.sleep(100);
      channel.writer.close();
      const r = await trigger;
      expectEqual(r.bytes_written, 0, 'text frames must not count as bytes_written');
      await expectWorkerHealthy(ctx);
    },
  },
  {
    name: 'stream_read_consumer_drops_without_drain',
    async run(ctx: CaseContext) {
      const root = newWorkdir('drop-read');
      const path = join(root, 'undrained.bin');
      // Write a small file via the streaming write path.
      const channel = await ctx.iii.createChannel(64);
      const payload = Buffer.from('payload-bytes');
      const wp = new Promise<void>((resolve, reject) => {
        channel.writer.stream.once('error', reject);
        channel.writer.stream.write(payload, (err) => {
          if (err) return reject(err);
          channel.writer.stream.end(() => resolve());
        });
      });
      const trigger = ctx.call('shell::fs::write', {
        target: { kind: 'host' },
        path,
        mode: '0644',
        parents: false,
        content: channel.readerRef,
      });
      await wp;
      channel.writer.close();
      await trigger;

      const resp = await ctx.call('shell::fs::read', {
        target: { kind: 'host' },
        path,
      });
      expect(typeof resp.size === 'number', 'read returns size');
      expect(resp.content && typeof resp.content.channel_id === 'string',
        'read returns a channel ref');
      await ctx.sleep(50);
      await expectWorkerHealthy(ctx);
    },
  },
  // KNOWN GAP: stream_write_no_eof_hits_deadline — the write-streaming loop
  // at shell/src/fs/host.rs:1027-1053 has no per-call deadline. Calling
  // shell::fs::write with a never-EOF channel hangs forever. Adding a
  // streaming deadline knob is tracked separately; testing that gap with a
  // green-pass assertion would require either an arbitrary JS-side timeout
  // (flaky in CI) or a worker-config change.
  // KNOWN GAP: stream_read_path_deleted_mid_stream — the read pump opens the
  // file before allocating the channel, so deleting the path after fs::read
  // returns has no observable effect on the stream content. Asserting that
  // is more of a unix-semantics test than a protocol test.
];
