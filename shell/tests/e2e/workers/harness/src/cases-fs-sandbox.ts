import type { ISdk } from 'iii-sdk';
import { ChannelReader } from 'iii-sdk';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

const URL = process.env.III_URL ?? 'ws://127.0.0.1:49134';

interface CapturedCall {
  function_id: string;
  payload: any;
}

const captured: CapturedCall[] = [];
let nextResponse: any = null;
let nextErrorMessage: string | null = null;
let sandboxWriteDrainedBytes: Buffer | null = null;
let sandboxReadBytesToServe: Buffer | null = null;

export function resetMocks(): void {
  captured.length = 0;
  nextResponse = null;
  nextErrorMessage = null;
  sandboxWriteDrainedBytes = null;
  sandboxReadBytesToServe = null;
}

export function setSandboxReadBytes(bytes: Buffer): void {
  sandboxReadBytesToServe = bytes;
}

export function getSandboxWriteDrainedBytes(): Buffer | null {
  return sandboxWriteDrainedBytes;
}

export function scriptResponse(value: any): void {
  nextResponse = value;
}

export function scriptError(s_code: string, detail: string): void {
  nextErrorMessage = `${s_code}: ${detail}`;
}

export function getCapturedCalls(): readonly CapturedCall[] {
  return captured;
}

export const SANDBOX_ID = '11111111-2222-3333-4444-555555555555';

export function setupSandboxMocks(iii: ISdk): void {
  const simpleOps = ['ls', 'stat', 'mkdir', 'rm', 'chmod', 'mv', 'grep', 'sed'];
  for (const op of simpleOps) {
    iii.registerFunction(`sandbox::fs::${op}`, async (payload: any) => {
      captured.push({ function_id: `sandbox::fs::${op}`, payload });
      if (nextErrorMessage !== null) {
        const m = nextErrorMessage;
        nextErrorMessage = null;
        throw new Error(m);
      }
      const r = nextResponse;
      nextResponse = null;
      return r ?? {};
    });
  }
  iii.registerFunction('sandbox::fs::write', async (payload: any) => {
    captured.push({ function_id: 'sandbox::fs::write', payload });
    sandboxWriteDrainedBytes = null;
    if (nextErrorMessage !== null) {
      const m = nextErrorMessage;
      nextErrorMessage = null;
      throw new Error(m);
    }
    const reader: ChannelReader = payload.content;
    const drained = await reader.readAll();
    sandboxWriteDrainedBytes = drained;
    const r = nextResponse;
    nextResponse = null;
    return r ?? { bytes_written: drained.length, path: payload.path };
  });
  iii.registerFunction('sandbox::fs::read', async (payload: any) => {
    captured.push({ function_id: 'sandbox::fs::read', payload });
    if (nextErrorMessage !== null) {
      const m = nextErrorMessage;
      nextErrorMessage = null;
      throw new Error(m);
    }
    const bytes = sandboxReadBytesToServe ?? Buffer.from('');
    const channel = await iii.createChannel(64);
    (async () => {
      channel.writer.stream.write(bytes, (err: Error | null | undefined) => {
        if (err) {
          channel.writer.close();
          return;
        }
        channel.writer.stream.end(() => channel.writer.close());
      });
    })();
    const r = nextResponse;
    nextResponse = null;
    return (
      r ?? {
        content: channel.readerRef,
        size: bytes.length,
        mode: '0644',
        mtime: 1700000000,
      }
    );
  });
  // NOTE: we deliberately do NOT mock `sandbox::exec` here. The
  // exec-side e2e cases (cases-exec-sandbox.ts) drive the *real*
  // iii-sandbox worker so the test exercises the full
  // shell -> sandbox::exec -> microVM -> response path, not just
  // shell's wire shape. The fs::* mocks above remain because the
  // fs-sandbox cases test wire-level forwarding shape only and would
  // otherwise need a real sandbox per case (slow cold-boot churn).
}

export const FS_SANDBOX_CASES: TestCase[] = [
  {
    name: 'fs_sandbox_ls_forwards_payload_with_sandbox_id',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({
        entries: [
          { name: 'a.txt', is_dir: false, size: 1, mode: '0644', mtime: 0, is_symlink: false },
        ],
      });
      const r = await ctx.call('shell::fs::ls', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        path: '/x',
      });
      expectEqual(r.entries.length, 1, 'response.entries len');
      expectEqual(r.entries[0].name, 'a.txt', 'forwarded entry name');
      expectEqual(captured.length, 1, 'mock invocation count');
      expectEqual(captured[0].function_id, 'sandbox::fs::ls', 'forwarded function id');
      expectEqual(captured[0].payload.sandbox_id, SANDBOX_ID, 'sandbox_id injected');
      expectEqual(captured[0].payload.path, '/x', 'path forwarded');
    },
  },
  {
    name: 'fs_sandbox_stat_unwraps_transparent_response',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({
        name: 'x.txt',
        is_dir: false,
        size: 7,
        mode: '0640',
        mtime: 1700000000,
        is_symlink: false,
      });
      const r = await ctx.call('shell::fs::stat', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        path: '/sb/x.txt',
      });
      expectEqual(r.size, 7, 'transparent stat.size');
      expectEqual(r.mode, '0640', 'transparent stat.mode');
    },
  },
  {
    name: 'fs_sandbox_mkdir_forwards_mode_and_parents',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({ created: true });
      const r = await ctx.call('shell::fs::mkdir', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        path: '/sb/new',
        mode: '0700',
        parents: true,
      });
      expectEqual(r.created, true, 'created');
      expectEqual(captured[0].payload.mode, '0700', 'mode forwarded');
      expectEqual(captured[0].payload.parents, true, 'parents forwarded');
    },
  },
  {
    name: 'fs_sandbox_rm_recursive_propagates_response',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({ removed: true });
      const r = await ctx.call('shell::fs::rm', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        path: '/sb/dir',
        recursive: true,
      });
      expectEqual(r.removed, true, 'removed');
      expectEqual(captured[0].payload.recursive, true, 'recursive forwarded');
    },
  },
  {
    name: 'fs_sandbox_chmod_forwards_uid_gid',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({ updated: 3 });
      const r = await ctx.call('shell::fs::chmod', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        path: '/sb/tree',
        mode: '0700',
        uid: 1000,
        gid: 1000,
        recursive: true,
      });
      expectEqual(r.updated, 3, 'updated count');
      expectEqual(captured[0].payload.uid, 1000, 'uid forwarded');
      expectEqual(captured[0].payload.gid, 1000, 'gid forwarded');
    },
  },
  {
    name: 'fs_sandbox_mv_forwards_overwrite',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({ moved: true });
      const r = await ctx.call('shell::fs::mv', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        src: '/sb/a',
        dst: '/sb/b',
        overwrite: true,
      });
      expectEqual(r.moved, true, 'moved');
      expectEqual(captured[0].payload.overwrite, true, 'overwrite forwarded');
      expectEqual(captured[0].payload.src, '/sb/a', 'src forwarded');
      expectEqual(captured[0].payload.dst, '/sb/b', 'dst forwarded');
    },
  },
  {
    name: 'fs_sandbox_grep_forwards_full_payload',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({
        matches: [{ path: '/sb/a.rs', line: 7, content: 'foo' }],
        truncated: false,
      });
      const r = await ctx.call('shell::fs::grep', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        path: '/sb',
        pattern: 'foo',
        recursive: true,
        ignore_case: true,
        include_glob: ['*.rs'],
        exclude_glob: ['target/*'],
        max_matches: 50,
        max_line_bytes: 4096,
      });
      expectEqual(r.matches.length, 1, 'matches len');
      expectEqual(r.matches[0].line, 7, 'match.line');
      const p = captured[0].payload;
      expectEqual(p.ignore_case, true, 'ignore_case forwarded');
      expectEqual(p.max_matches, 50, 'max_matches forwarded');
      expectEqual(p.include_glob[0], '*.rs', 'include_glob forwarded');
    },
  },
  {
    name: 'fs_sandbox_sed_forwards_full_payload',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptResponse({
        results: [{ path: '/sb/a.rs', replacements: 2, success: true }],
        total_replacements: 2,
      });
      const r = await ctx.call('shell::fs::sed', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        files: ['/sb/a.rs'],
        recursive: false,
        include_glob: [],
        exclude_glob: [],
        pattern: 'foo',
        replacement: 'bar',
        regex: false,
        first_only: false,
        ignore_case: false,
      });
      expectEqual(r.total_replacements, 2, 'total_replacements');
      expect(Array.isArray(r.results) && r.results[0].success === true, 'success flag forwarded');
    },
  },
  {
    name: 'fs_sandbox_engine_s211_propagates_to_caller',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptError('S211', 'path not found: /sb/missing');
      await ctx.expectError(
        () =>
          ctx.call('shell::fs::ls', {
            target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
            path: '/sb/missing',
          }),
        'S211',
      );
    },
  },
  {
    name: 'fs_sandbox_engine_s002_sandbox_not_found_propagates',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptError('S002', 'sandbox not found');
      await ctx.expectError(
        () =>
          ctx.call('shell::fs::stat', {
            target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
            path: '/anything',
          }),
        'S002',
      );
    },
  },
  {
    name: 'fs_sandbox_engine_s217_bad_regex_propagates',
    async run(ctx: CaseContext) {
      resetMocks();
      scriptError('S217', 'bad regex: unclosed character class');
      await ctx.expectError(
        () =>
          ctx.call('shell::fs::grep', {
            target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
            path: '/sb',
            pattern: '[unclosed',
            recursive: true,
            ignore_case: false,
            include_glob: [],
            exclude_glob: [],
            max_matches: 10,
            max_line_bytes: 4096,
          }),
        'S217',
      );
    },
  },
  {
    name: 'fs_sandbox_write_forwards_content_ref_and_passes_bytes_through',
    async run(ctx: CaseContext) {
      resetMocks();
      const channel = await ctx.iii.createChannel(64);
      const writePromise = new Promise<void>((resolve, reject) => {
        channel.writer.stream.once('error', reject);
        channel.writer.stream.write(Buffer.from('hello sandbox'), (err) => {
          if (err) { reject(err); return; }
          channel.writer.stream.end(() => resolve());
        });
      });
      const triggerPromise = ctx.call('shell::fs::write', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        path: '/sb/x',
        mode: '0644',
        parents: false,
        content: channel.readerRef,
      });
      await writePromise;
      channel.writer.close();
      const r = await triggerPromise;
      expectEqual(r.bytes_written, 13, 'bytes_written round-trips');
      expectEqual(r.path, '/sb/x', 'path forwarded');
      expectEqual(captured.length, 1, 'mock invoked exactly once');
      const drained = getSandboxWriteDrainedBytes();
      expect(drained !== null && drained.equals(Buffer.from('hello sandbox')),
        'engine drained the caller\'s bytes verbatim',
      );
      const fwdedPayload = captured[0].payload;
      expectEqual(fwdedPayload.path, '/sb/x', 'forwarded payload.path');
      expectEqual(fwdedPayload.mode, '0644', 'forwarded payload.mode');
      expect(fwdedPayload.content instanceof ChannelReader,
        'forwarded payload.content resolved to ChannelReader by SDK',
      );
    },
  },
  {
    name: 'fs_sandbox_read_returns_engine_channel_ref_and_streams_bytes',
    async run(ctx: CaseContext) {
      resetMocks();
      setSandboxReadBytes(Buffer.from('sandbox payload'));
      const r = await ctx.call('shell::fs::read', {
        target: { kind: 'sandbox', sandbox_id: SANDBOX_ID },
        path: '/sb/y',
      });
      expectEqual(r.size, 15, 'size from engine');
      expectEqual(r.mode, '0644', 'mode from engine');
      expect(r.content && typeof r.content.channel_id === 'string',
        `engine response.content shape: ${JSON.stringify(r.content)}`,
      );
      const reader = new ChannelReader(URL, r.content);
      const data = await reader.readAll();
      expectEqual(data.toString('utf8'), 'sandbox payload', 'streamed bytes');
      const fwdedPayload = captured[0].payload;
      expectEqual(fwdedPayload.path, '/sb/y', 'forwarded payload.path');
      expect(fwdedPayload.content === undefined, 'read request must NOT carry content');
    },
  },
];
