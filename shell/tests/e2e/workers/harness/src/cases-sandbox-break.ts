import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { StreamChannelRef } from 'iii-sdk';
import { ChannelReader } from 'iii-sdk';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';
import {
  resetMocks,
  scriptResponse,
  scriptError,
  getCapturedCalls,
  SANDBOX_ID,
} from './cases-fs-sandbox.ts';

const URL = process.env.III_URL ?? 'ws://127.0.0.1:49134';

function newWorkdir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `iii-shell-sb-break-${prefix}-`));
}

const SB_TARGET = { kind: 'sandbox' as const, sandbox_id: SANDBOX_ID };

export const SANDBOX_BREAK_CASES: TestCase[] = [
  {
    name: 'sandbox_mock_returns_empty_object_for_ls_yields_s216',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({});
      await ctx.expectError(
        () => ctx.call('shell::fs::ls', { target: SB_TARGET, path: '/x' }),
        'S216',
      );
    },
  },
  {
    name: 'sandbox_mock_returns_wrong_type_for_entries_yields_s216',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({ entries: 'this should be an array' });
      await ctx.expectError(
        () => ctx.call('shell::fs::ls', { target: SB_TARGET, path: '/x' }),
        'S216',
      );
    },
  },
  {
    name: 'sandbox_mock_returns_null_yields_s216',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse(null);
      await ctx.expectError(
        () => ctx.call('shell::fs::ls', { target: SB_TARGET, path: '/x' }),
        'S216',
      );
    },
  },
  {
    name: 'sandbox_mock_extra_unknown_fields_tolerated',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({
        entries: [
          { name: 'ok.txt', is_dir: false, size: 4, mode: '0644', mtime: 0, is_symlink: false },
        ],
        future_field_added_in_v2: 'should be ignored',
        another_extra: 42,
        nested_extra: { also: 'ignored' },
      });
      const r = await ctx.call('shell::fs::ls', { target: SB_TARGET, path: '/x' });
      expectEqual(r.entries.length, 1, 'entries forwarded despite extras');
      expectEqual(r.entries[0].name, 'ok.txt', 'entry payload intact');
    },
  },
  {
    name: 'sandbox_unknown_s_code_collapses_to_s216',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptError('S299', 'fictional future error code');
      await ctx.expectError(
        () => ctx.call('shell::fs::ls', { target: SB_TARGET, path: '/x' }),
        'S216',
      );
    },
  },
  {
    name: 'sandbox_engine_s213_propagates_to_caller',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptError('S213', 'destination already exists: /sb/dst');
      await ctx.expectError(
        () => ctx.call('shell::fs::mv', {
          target: SB_TARGET,
          src: '/sb/a',
          dst: '/sb/dst',
          overwrite: false,
        }),
        'S213',
      );
    },
  },
  {
    name: 'sandbox_engine_error_without_s_code_falls_back_to_s216',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptError('NOTACODE', 'just a plain old error string with no code');
      await ctx.expectError(
        () => ctx.call('shell::fs::ls', { target: SB_TARGET, path: '/x' }),
        'S216',
      );
    },
  },
  {
    // Sandbox writes are NOT subject to host caps — the sandbox is
    // expected to enforce its own limits.
    name: 'sandbox_write_path_bypasses_host_cap',
    async run(ctx: CaseContext) {
      resetMocks();
      const root = newWorkdir('cap-bypass');
      const channel = await ctx.iii.createChannel(64);
      const oversize = Buffer.alloc(2 * 1024 * 1024); // 2 MiB > 1 MiB host cap

      const writePromise = new Promise<void>((resolve, reject) => {
        channel.writer.stream.once('error', reject);
        channel.writer.stream.write(oversize, (err) => {
          if (err) return reject(err);
          channel.writer.stream.end(() => resolve());
        });
      });
      const trigger = ctx.call('shell::fs::write', {
        target: SB_TARGET,
        path: join(root, 'big.bin'),
        mode: '0644',
        parents: false,
        content: channel.readerRef,
      });
      await writePromise;
      channel.writer.close();
      const r = await trigger;
      expectEqual(
        r.bytes_written,
        oversize.length,
        'sandbox write succeeds at 2 MiB despite 1 MiB host cap',
      );
      const calls = getCapturedCalls();
      const writeCall = calls.find((c) => c.function_id === 'sandbox::fs::write');
      expect(writeCall !== undefined, 'mock received the sandbox write call');
    },
  },
  {
    name: 'sandbox_read_with_bogus_engine_channel_ref_passes_through',
    async run(ctx: CaseContext) {
      resetMocks();
      const bogusRef: StreamChannelRef = {
        channel_id: '99999999-9999-9999-9999-999999999999',
        access_key: 'no-such-key',
        direction: 'read',
      };
      scriptResponse({
        content: bogusRef,
        size: 42,
        mode: '0644',
        mtime: 1700000000,
      });
      const resp = await ctx.call('shell::fs::read', {
        target: SB_TARGET,
        path: '/sb/whatever',
      });
      expectEqual(resp.size, 42, 'metadata forwarded verbatim');
      expectEqual(resp.content.channel_id, bogusRef.channel_id, 'bogus channel_id forwarded as-is');
      const reader = new ChannelReader(URL, resp.content);
      let threwOrEmpty = false;
      try {
        const data = await reader.readAll();
        threwOrEmpty = data.length === 0;
      } catch {
        threwOrEmpty = true;
      }
      expect(threwOrEmpty, 'bogus ref must fail to drain or be empty');
      const r = await ctx.call('shell::exec', { command: 'echo', args: ['ok'] });
      expectEqual(r.exit_code, 0, 'worker survives bogus engine ref');
    },
  },
  {
    name: 'sandbox_mock_null_in_nested_required_field_yields_s216',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({
        entries: [
          {
            name: null, // non-optional in FsEntry
            is_dir: false,
            size: 1,
            mode: '0644',
            mtime: 0,
            is_symlink: false,
          },
        ],
      });
      await ctx.expectError(
        () => ctx.call('shell::fs::ls', { target: SB_TARGET, path: '/x' }),
        'S216',
      );
    },
  },
];
