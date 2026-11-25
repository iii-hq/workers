import {
  appendFile,
  link,
  mkdir,
  rename,
  symlink,
  truncate,
  unlink,
  writeFile,
} from 'node:fs/promises';
import { join } from 'node:path';
import { context, trace } from '@opentelemetry/api';
import { registerWorker } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import {
  deferred,
  id,
  record,
  recordId,
  type SyntheticHost,
  withSyntheticHost,
} from './_helpers/external-replies-host.js';

const GET = 'claude::agents::get';
const ON = 'claude::agents::on-reply';
const lookup = (s: SyntheticHost, session = id) =>
  s.client.call({ function_id: GET, payload: { claude_session_id: session } });
const observe = (s: SyntheticHost, key: string, session = id) => s.client.observe(ON, key, session);
const lines = (...rows: unknown[]) => rows.map((r) => `${JSON.stringify(r)}\n`).join('');
const answer = (n: number, parent: number | null, text: string) =>
  record(n, parent, 'assistant', text);
const toolResultTranscript = (fields: { content?: unknown; tool_use_id?: unknown }) =>
  lines(
    record(1, null, 'user', 'synthetic prompt'),
    record(2, 1, 'assistant', [{ type: 'tool_use', id: 'toolu_1', name: 'Read', input: {} }]),
    record(3, 2, 'user', [{ type: 'tool_result', tool_use_id: 'toolu_1', ...fields }]),
    answer(4, 3, 'synthetic answer'),
  );
const malformedToolResults = [
  { name: 'numeric content', fields: { content: 42 } },
  { name: 'numeric block', fields: { content: [42] } },
  { name: 'unknown block type', fields: { content: [{ type: 'tool_use' }] } },
  { name: 'malformed text block', fields: { content: [{ type: 'text', text: 42 }] } },
  { name: 'malformed image block', fields: { content: [{ type: 'image', source: 42 }] } },
  {
    name: 'malformed document source',
    fields: { content: [{ type: 'document', source: { type: 'content', content: [42] } }] },
  },
  {
    name: 'malformed search result',
    fields: {
      content: [{ type: 'search_result', source: 'synthetic', title: 'synthetic', content: [42] }],
    },
  },
  {
    name: 'malformed tool reference',
    fields: { content: [{ type: 'tool_reference', tool_name: 42 }] },
  },
  { name: 'numeric tool use ID', fields: { tool_use_id: 42, content: 'synthetic result' } },
  { name: 'missing tool use ID', fields: { tool_use_id: undefined } },
];
const reads = (s: SyntheticHost) => s.host.calls.filter((c) => c.payload.command === 'python3');
const seen = (s: SyntheticHost, observer: string) =>
  s.host.deliveries.filter((d) => d.observer === observer).map((d) => d.event.reply);

describe('lookup', () => {
  it('uses the current CLI row, not an alias, short id, unknown kind or headless-only row', async () => {
    await withSyntheticHost(async (s) => {
      expect(await lookup(s)).toEqual({
        claude_session_id: id,
        cwd: '/fixture',
        name: 'fixture',
        status: 'idle',
        last_reply: { id: recordId(1), text: 'fixture answer' },
      });
      s.host.cli[0] = { kind: 'background', state: 'blocked', id: 'deadbeef', sessionId: id };
      expect(await lookup(s)).toMatchObject({ cwd: null, name: null, status: 'waiting' });
      await expect(lookup(s, 'deadbeef')).rejects.toThrow('invalid Claude session id');
      s.host.cli[0].kind = 'unsupported';
      await expect(lookup(s)).rejects.toThrow('session not listed');
      s.host.cli.length = 0;
      s.host.records.push({ session_id: recordId(9), claude_session_id: id, status: 'done' });
      await expect(lookup(s, recordId(9))).rejects.toThrow('session not listed');
      await expect(lookup(s)).rejects.toThrow('session not listed');
      s.host.inventoryOK = false;
      await expect(lookup(s)).rejects.toThrow(/^inventory unavailable$/);
      expect(reads(s)).toHaveLength(2);
    });
  });

  it('uses an idle CLI row even when an old headless record shares its UUID', async () => {
    await withSyntheticHost(async (s) => {
      s.host.records.push({ session_id: recordId(9), claude_session_id: id, status: 'done' });
      expect(await lookup(s)).toEqual({
        claude_session_id: id,
        cwd: '/fixture',
        name: 'fixture',
        status: 'idle',
        last_reply: { id: recordId(1), text: 'fixture answer' },
      });
    });
  });

  it.each([
    'NaN',
    'Infinity',
    '-Infinity',
  ])('rejects the nonstandard JSON token %s in a complete active record', async (token) => {
    await withSyntheticHost(async (s) => {
      const row = JSON.stringify(record(1, null, 'user', ['INVALID_NUMBER']));
      await writeFile(s.file, `${row.replace('"INVALID_NUMBER"', token)}\n`);
      await expect(lookup(s)).rejects.toThrow(/^transcript unavailable$/);
    });
  });

  it('rejects malformed user message blocks on the active chain', async () => {
    await withSyntheticHost(async (s) => {
      await writeFile(s.file, lines(record(1, null, 'user', [42])));
      await expect(lookup(s)).rejects.toThrow(/^transcript unavailable$/);

      for (const block of [{ type: 42 }, { type: 'text' }, { type: 'text', text: 7 }]) {
        await writeFile(s.file, lines(record(1, null, 'user', [block]), answer(2, 1, 'later')));
        await expect(lookup(s)).rejects.toThrow(/^transcript unavailable$/);
      }
    });
  });

  it.each(malformedToolResults)('rejects an active tool result with $name', async ({ fields }) => {
    await withSyntheticHost(async (s) => {
      await writeFile(s.file, toolResultTranscript(fields));
      await expect(lookup(s)).rejects.toThrow(/^transcript unavailable$/);
      expect(s.host.snapshots).toEqual([]);
    });
  });

  it('accepts string, block array and omitted tool result content', async () => {
    await withSyntheticHost(async (s) => {
      for (const content of [
        'synthetic result',
        [],
        [
          { type: 'text', text: 'synthetic result' },
          { type: 'image', source: { type: 'url', url: 'https://example.invalid/image.png' } },
          {
            type: 'document',
            source: { type: 'content', content: [{ type: 'text', text: 'synthetic document' }] },
          },
          {
            type: 'search_result',
            title: 'synthetic result',
            source: 'synthetic',
            content: [{ type: 'text', text: 'synthetic result' }],
          },
          { type: 'tool_reference', tool_name: 'Read' },
        ],
        undefined,
      ]) {
        await writeFile(s.file, toolResultTranscript({ content }));
        expect((await lookup(s)).last_reply).toEqual({ id: recordId(4), text: 'synthetic answer' });
      }
    });
  });

  it('selects complete visible blocks on the active branch and never exports raw records', async () => {
    await withSyntheticHost(async (s) => {
      await writeFile(
        s.file,
        lines(
          record(1, null, 'user', 'secret prompt'),
          answer(2, 1, 'secret abandoned'),
          record(3, 1, 'assistant', [
            { type: 'thinking', thinking: 'secret reasoning' },
            { type: 'text', text: 'first' },
            { type: 'tool_use', input: 'secret argument' },
            { type: 'text', text: 'second' },
          ]),
          record(4, 3, 'assistant', '  '),
          { ...answer(5, 4, 'secret meta'), isMeta: true },
          { ...answer(6, 5, 'secret sidechain'), isSidechain: true },
          { type: 'summary', summary: 'secret summary', leafUuid: recordId(2) },
        ) + JSON.stringify(answer(7, 5, 'not yet')),
      );
      expect((await lookup(s)).last_reply).toEqual({ id: recordId(3), text: 'first\nsecond' });
      expect(JSON.parse(s.host.snapshots[0])).toEqual({
        cursor: expect.any(String),
        replies: [{ id: recordId(3), text: 'first\nsecond' }],
      });
      await appendFile(s.file, '\n');
      expect((await lookup(s)).last_reply).toEqual({ id: recordId(7), text: 'not yet' });
    });
  });

  it('does not fall back to an abandoned assistant sibling when the active branch has no text', async () => {
    await withSyntheticHost(async (s) => {
      await writeFile(
        s.file,
        lines(
          record(1, null, 'user', 'hello'),
          answer(2, 1, 'abandoned'),
          record(3, 1, 'assistant', [{ type: 'tool_use', id: 'toolu_1', input: 'private' }]),
          record(4, 3, 'user', [
            { type: 'tool_result', tool_use_id: 'toolu_1', content: 'private' },
          ]),
        ),
      );
      expect((await lookup(s)).last_reply).toBeNull();
      await appendFile(
        s.file,
        lines(
          { ...answer(5, 2, 'annotation'), isMeta: true },
          { ...answer(6, 2, 'teammate'), teamName: 'other' },
        ),
      );
      expect((await lookup(s)).last_reply).toBeNull();
      expect(s.host.snapshots.join('')).not.toMatch(/abandoned|private|hello|teammate/);
    });
  });

  it('distinguishes broken chains and corrupt complete records from a successful empty read', async () => {
    await withSyntheticHost(async (s) => {
      for (const data of [
        lines(answer(1, 3, 'orphan')),
        lines(answer(1, 1, 'cycle')),
        lines(answer(1, null, 'a'), answer(1, null, 'duplicate')),
        '{broken\n',
        lines({ ...answer(1, null, 'wrong session'), sessionId: recordId(99) }),
        lines({ ...answer(1, null, 'invalid id'), uuid: 'other' }),
        lines({ ...answer(1, null, 'no parent field'), parentUuid: undefined }),
        lines({ type: 'user', uuid: recordId(1), parentUuid: null }),
        lines({ type: 'unsupported-format', message: { content: 'private' } }),
      ]) {
        await writeFile(s.file, data);
        await expect(lookup(s)).rejects.toThrow(/^transcript unavailable$/);
      }
      await writeFile(s.file, '');
      expect((await lookup(s)).last_reply).toBeNull();
    });
  });
});

describe('host reader', () => {
  it('keeps reply text out of both shell exec payloads and rejects modified output', async () => {
    await withSyntheticHost(async (s) => {
      const text = 'fixture answer';
      expect((await lookup(s)).last_reply).toEqual({ id: recordId(1), text });
      expect((await lookup(s)).last_reply?.text).toBe(text);
      const [first, , second] = s.host.outputs;
      expect(first).toMatch(/^[A-Za-z0-9+/]+={0,2}$/);
      expect(second).not.toBe(first);
      expect(s.host.outputs.join('')).not.toContain(text);
      for (const { payload } of reads(s)) {
        // The Rust SDK's payload redactor masks values under keys containing
        // "secret", but stdout and argv are recorded verbatim.
        expect(payload.env).toHaveProperty('CLAUDE_REPLY_SECRET_PAD');
        expect(JSON.stringify(payload)).not.toContain(text);
      }
      s.host.hostOutput = (stdout) => `${stdout[0] === 'A' ? 'B' : 'A'}${stdout.slice(1)}`;
      await expect(lookup(s)).rejects.toThrow(/^transcript unavailable$/);
    });
  });

  it('keeps lookup and poll text out of recording Node SDK spans', async () => {
    type Handler = (value: unknown) => unknown;
    type WrappedHandler = (value: unknown) => Promise<unknown>;
    let wrap!: (fn: string, handler: Handler) => WrappedHandler;
    vi.stubEnv('III_DISABLE_TRACE_PAYLOADS', undefined);
    try {
      await withSyntheticHost(
        async (s) => {
          vi.stubEnv('HOME', s.root);
          const sdk = registerWorker('ws://127.0.0.1:0', {
            enableMetricsReporting: false,
            reconnectionConfig: { maxRetries: 0 },
            otel: {
              enabled: true,
              fetchInstrumentationEnabled: false,
              metricsEnabled: false,
              reconnectionConfig: { maxRetries: 0 },
            },
          });
          const { wrapHandler } = sdk as unknown as {
            wrapHandler: (fn: string, handler: Handler) => WrappedHandler;
          };
          wrap = wrapHandler.bind(sdk);
          const probe = trace.getTracer('reply-privacy-test').startSpan('recorder probe');
          const addEvent = vi.spyOn(Object.getPrototypeOf(probe), 'addEvent');
          const policy = process.env.III_DISABLE_TRACE_PAYLOADS;
          try {
            expect(probe.isRecording()).toBe(true);
            // Positive control: the pinned SDK records plaintext if enabled.
            delete process.env.III_DISABLE_TRACE_PAYLOADS;
            await context.with(trace.setSpan(context.active(), probe), () =>
              wrap('test::recorder-control', () => ({ text: 'recorder control' }))({}),
            );
            expect(JSON.stringify(addEvent.mock.calls)).toContain('recorder control');
            if (policy !== undefined) process.env.III_DISABLE_TRACE_PAYLOADS = policy;
            addEvent.mockClear();

            expect((await lookup(s)).last_reply?.text).toBe('fixture answer');
            await observe(s, 'A');
            await appendFile(s.file, lines(answer(2, 1, 'synthetic later answer')));
            await s.poll();
            expect(seen(s, 'A')).toEqual([{ id: recordId(2), text: 'synthetic later answer' }]);
            const spanEvents = JSON.stringify(addEvent.mock.calls);
            expect(spanEvents).not.toMatch(/fixture answer|synthetic later answer/);
          } finally {
            probe.end();
            addEvent.mockRestore();
            await sdk.shutdown();
            trace.disable();
            context.disable();
          }
        },
        true,
        (fn, handler) => (value) => wrap(fn, handler)(value),
      );
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it('pins the configured shared projects store, using the inventory environment, not cwd', async () => {
    await withSyntheticHost(async (s) => {
      const shared = join(s.root, 'shared store');
      await rename(s.projects, shared);
      await symlink(shared, s.projects);
      s.host.cli[0].cwd = '/not/the/transcript/path';
      expect((await lookup(s)).last_reply?.text).toBe('fixture answer');
      expect(reads(s)[0].payload).toMatchObject({
        command: 'python3',
        env: { CLAUDE_CONFIG_DIR: s.root },
        timeout_ms: 5000,
      });
      expect((reads(s)[0].payload.args as string[]).slice(0, 4)).toEqual(['-I', '-S', '-B', '-c']);
      expect(reads(s)[0].payload).not.toHaveProperty('cwd');
    });
  });

  it('rejects traversal, ambiguous matches, descendant links and nonregular files', async () => {
    await withSyntheticHost(async (s) => {
      await expect(lookup(s, `../${id}`)).rejects.toThrow('invalid Claude session id');
      expect(reads(s)).toHaveLength(0);
      const other = join(s.projects, 'other');
      await mkdir(other);
      const duplicate = join(other, `${id}.jsonl`);
      await writeFile(duplicate, lines(answer(1, null, 'other answer')));
      await expect(lookup(s)).rejects.toThrow('transcript unavailable');
      await unlink(duplicate);
      const target = join(s.root, 'outside');
      await rename(s.file, target);
      await symlink(target, s.file);
      await expect(lookup(s)).rejects.toThrow('transcript unavailable');
      await unlink(s.file);
      await link(target, s.file);
      await expect(lookup(s)).rejects.toThrow('transcript unavailable');
      await unlink(s.file);
      await mkdir(s.file);
      await expect(lookup(s)).rejects.toThrow('transcript unavailable');
      await rename(s.project, join(s.root, 'elsewhere'));
      await symlink(join(s.root, 'elsewhere'), s.project);
      await expect(lookup(s)).rejects.toThrow('transcript unavailable');
      expect(s.host.outputs.join('')).toBe('');
    });
  });

  it('fails quietly for a missing file, over-limit record, result or file, or host failure', async () => {
    await withSyntheticHost(async (s) => {
      await unlink(s.file);
      await expect(lookup(s)).rejects.toThrow('transcript unavailable');
      for (const bytes of [49 * 1024, 1024 * 1024]) {
        await writeFile(s.file, lines(answer(1, null, 'x'.repeat(bytes))));
        await expect(lookup(s)).rejects.toThrow('transcript unavailable');
      }
      await truncate(s.file, 8 * 1024 * 1024 + 1);
      await expect(lookup(s)).rejects.toThrow('transcript unavailable');
      s.host.beforeRead = async () => {
        throw new Error('private diagnostic');
      };
      await expect(lookup(s)).rejects.toThrow(/^transcript unavailable$/);
      expect(s.host.outputs.join('')).toBe('');
    });
  });
});

function barrier() {
  const entered = deferred<void>();
  const release = deferred<void>();
  return {
    entered: entered.promise,
    release: () => release.resolve(),
    wait: async () => {
      entered.resolve();
      await release.promise;
    },
  };
}

describe('observation', () => {
  it('keeps a physical boundary per accepted observer and targets only its new active replies', async () => {
    await withSyntheticHost(async (s) => {
      await appendFile(s.file, lines(answer(2, 1, 'old sibling')));
      await observe(s, 'A');
      await appendFile(s.file, lines(answer(3, 1, 'new sibling')));
      await observe(s, 'B');
      const other = recordId(99);
      s.host.cli.push({ kind: 'background', sessionId: other });
      await writeFile(join(s.project, `${other}.jsonl`), '');
      await observe(s, 'other', other);
      expect(s.host.snapshots.join('')).not.toMatch(/sibling|fixture answer/);
      await s.poll();
      expect(seen(s, 'A')).toEqual([{ id: recordId(3), text: 'new sibling' }]);
      expect(seen(s, 'B')).toEqual([]);
      await appendFile(
        s.file,
        lines(answer(4, 3, 'not active'), answer(5, 3, 'equal'), answer(6, 5, 'equal')),
      );
      await s.poll();
      await s.poll();
      const both = [
        { id: recordId(5), text: 'equal' },
        { id: recordId(6), text: 'equal' },
      ];
      expect(seen(s, 'A')).toEqual([{ id: recordId(3), text: 'new sibling' }, ...both]);
      expect(seen(s, 'B')).toEqual(both);
      expect(seen(s, 'other')).toEqual([]);
      expect(s.host.snapshots.join('')).not.toContain('not active');
      expect(
        s.host.calls.filter((c) => c.function_id === ON).map((c) => c.metadata?.for_observer),
      ).toEqual(['A', 'A', 'A', 'B', 'B']);
    });
  });

  it('defers partial records, retries failed reads, and rebaselines on replacement, rewrite and truncation', async () => {
    await withSyntheticHost(async (s) => {
      await observe(s, 'A');
      await appendFile(s.file, JSON.stringify(answer(2, 1, 'complete later')));
      await s.poll();
      await appendFile(s.file, '\n');
      s.host.beforeRead = async () => {
        throw new Error('transient');
      };
      await s.poll();
      expect(seen(s, 'A')).toEqual([]);
      s.host.beforeRead = async () => {};
      await s.poll();
      const first = [{ id: recordId(2), text: 'complete later' }];
      expect(seen(s, 'A')).toEqual(first);
      await rename(s.file, join(s.root, 'previous'));
      await writeFile(s.file, lines(answer(10, null, 'replacement')));
      await s.poll();
      // Same inode, different complete prefix; unlike a truncation the file grows.
      await writeFile(s.file, lines(answer(11, null, 'replacement'), answer(12, 11, 'rewritten')));
      await s.poll();
      await writeFile(s.file, lines(answer(13, null, 'smaller')));
      await s.poll();
      expect(seen(s, 'A')).toEqual(first);
      await appendFile(s.file, lines(answer(14, 13, 'after reset')));
      await s.poll();
      expect(seen(s, 'A')).toEqual([...first, { id: recordId(14), text: 'after reset' }]);
      await s.restart();
      await observe(s, 'restarted');
      await s.poll();
      expect(seen(s, 'restarted')).toEqual([]);
      await appendFile(s.file, lines(answer(15, 14, 'after restart')));
      await s.poll();
      expect(seen(s, 'restarted')).toEqual([{ id: recordId(15), text: 'after restart' }]);
    });
  });

  it('counts pending admissions toward the observer cap', async () => {
    await withSyntheticHost(async (s) => {
      const blocked = barrier();
      s.host.beforeRead = blocked.wait;
      const admissions = Promise.allSettled(
        Array.from({ length: 256 }, (_, i) => observe(s, String(i))),
      );
      await blocked.entered;
      await expect(observe(s, 'excess')).rejects.toMatchObject({ code: 'OBSERVER_LIMIT' });
      await setPermission(s, false);
      blocked.release();
      expect((await admissions).filter((a) => a.status === 'rejected')).toHaveLength(256);
      expect(reads(s)).toHaveLength(1);
      expect(s.timers.size).toBe(0);
    });
  });

  it.each(malformedToolResults)('rejects a baseline tool result with $name', async ({ fields }) => {
    await withSyntheticHost(async (s) => {
      await writeFile(s.file, toolResultTranscript(fields));
      await expect(observe(s, 'malformed')).rejects.toThrow(/^transcript unavailable$/);
      expect(s.timers.size).toBe(0);
    });
  });

  it('serializes admission with polling, rejects unreadable baselines and stops on last removal', async () => {
    await withSyntheticHost(async (s) => {
      await unlink(s.file);
      await expect(observe(s, 'failed')).rejects.toThrow('transcript unavailable');
      expect(s.timers.size).toBe(0);
      await writeFile(s.file, '');
      const stopA = await observe(s, 'A');
      const block = barrier();
      s.host.beforeRead = block.wait;
      const polling = s.poll();
      await block.entered;
      const before = reads(s).length;
      const admission = observe(s, 'B');
      await stopA();
      expect(reads(s)).toHaveLength(before);
      await appendFile(s.file, lines(answer(1, null, 'during read')));
      block.release();
      await polling;
      const stopB = await admission;
      await s.poll();
      expect(s.host.deliveries).toEqual([]);
      await stopB();
      expect(s.timers.size).toBe(0);
      const stopped = reads(s).length;
      await s.poll();
      expect(reads(s)).toHaveLength(stopped);
    });
  });
});

async function setPermission(s: SyntheticHost, enabled: boolean) {
  s.host.config = async () => ({ external_sessions: enabled });
  await s.refresh();
}

describe('authorization', () => {
  it.each([
    'configuration:deleted',
    'configuration:registered',
  ])('does not let %s discard an in-flight settings update', async (event) => {
    await withSyntheticHost(async (s) => {
      expect((await s.list()).agents).toHaveLength(1);
      const response = deferred<unknown>();
      const captured = deferred<void>();
      s.host.config = () => {
        captured.resolve();
        return response.promise;
      };
      const update = s.configEvent('configuration:updated');
      await captured.promise;
      s.host.config = async () => null;
      await s.configEvent(event);
      expect((await s.list()).agents).toHaveLength(1);
      response.resolve({ external_sessions: false });
      await update;
      expect((await s.list()).agents).toEqual([]);
      await expect(lookup(s)).rejects.toThrow('reply access disabled');
    });
  });
  it('closes on deletion during an older refresh and rebaselines after registration', async () => {
    await withSyntheticHost(async (s) => {
      await observe(s, 'A');
      const slow = deferred<unknown>();
      s.host.config = () => slow.promise;
      const oldRefresh = s.configEvent('configuration:updated');
      s.host.config = async () => null;
      await s.configEvent('configuration:deleted');
      slow.resolve({ external_sessions: true });
      await oldRefresh;
      const before = reads(s).length;
      await expect(lookup(s)).rejects.toThrow('reply access disabled');
      await expect(observe(s, 'B')).rejects.toThrow('reply access disabled');
      await appendFile(s.file, lines(answer(2, 1, 'while absent')));
      await s.poll();
      expect(reads(s)).toHaveLength(before);
      s.host.config = async () => ({ external_sessions: true });
      await s.configEvent('configuration:registered');
      await s.poll();
      expect(seen(s, 'A')).toEqual([]);
      await appendFile(s.file, lines(answer(3, 2, 'after registration')));
      await s.poll();
      expect(seen(s, 'A')).toEqual([{ id: recordId(3), text: 'after registration' }]);
    });
  });

  it('does not send a previously matched reply after removal or into an offline queue', async () => {
    await withSyntheticHost(async (s) => {
      for (const stage of ['remove', 'disconnect']) {
        const remove = await observe(s, stage);
        const block = barrier();
        s.host.beforeDelivery = block.wait;
        const n = stage === 'remove' ? 2 : 3;
        await appendFile(s.file, lines(answer(n, n - 1, 'matched')));
        const pending = s.poll();
        await block.entered;
        if (stage === 'remove') await remove();
        else s.host.socketOpen = false;
        block.release();
        await pending;
        expect(seen(s, stage)).toEqual([]);
      }
    });
  });

  it.each([
    'pending',
    'acknowledged',
  ])('revokes permission across %s active-to-active configuration observer replay', async (replay) => {
    await withSyntheticHost(async (s) => {
      await observe(s, 'A');
      const stalled = barrier();
      s.host.beforeRead = stalled.wait;
      const oldLookup = expect(lookup(s)).rejects.toThrow('reply access disabled');
      await stalled.entered;

      // No lease is checked while offline, and the observer state stays active.
      s.host.socketOpen = false;
      s.host.config = async () => ({ external_sessions: false });
      s.host.connectionEpoch++;
      s.host.configWatchAnswered = false;
      s.host.socketOpen = true;
      if (replay === 'acknowledged') s.host.configWatchAnswered = true;
      const beforeReplay = s.host.calls.length;
      stalled.release();
      s.host.beforeRead = async () => {};
      await oldLookup;
      if (replay === 'pending') {
        await expect(lookup(s)).rejects.toThrow('reply access disabled');
        await expect(observe(s, 'before-ack')).rejects.toThrow('reply access disabled');
        await s.poll();
        expect(s.host.calls.slice(beforeReplay).map((c) => c.function_id)).toEqual([GET]);
        s.host.configWatchAnswered = true;
      }

      await s.poll();
      expect(
        s.host.calls
          .slice(beforeReplay)
          .map((c) => c.function_id)
          .filter((fn) => fn !== GET),
      ).toEqual(['configuration::get']);
      await expect(lookup(s)).rejects.toThrow('reply access disabled');
      await expect(observe(s, 'denied')).rejects.toThrow('reply access disabled');

      await appendFile(s.file, lines(answer(2, 1, 'during the gap')));
      s.host.config = async () => ({ external_sessions: true });
      await s.configEvent('configuration:updated');
      await s.poll();
      expect(seen(s, 'A')).toEqual([]);
      await appendFile(s.file, lines(answer(3, 2, 'after new confirmation')));
      await s.poll();
      expect(seen(s, 'A')).toEqual([{ id: recordId(3), text: 'after new confirmation' }]);
    });
  });

  it('requires a parsed current live true, never the enabled seed, timeout or an older refresh', async () => {
    await withSyntheticHost(async (s) => {
      async function closed() {
        await expect(lookup(s)).rejects.toThrow('reply access disabled');
        await expect(lookup(s, 'invalid')).rejects.toThrow('reply access disabled');
        await expect(observe(s, 'denied')).rejects.toThrow('reply access disabled');
        expect(
          s.host.calls.filter(
            (c) => c.function_id.startsWith('shell::') || c.function_id.startsWith('state::'),
          ),
        ).toEqual([]);
      }
      await closed();
      await setPermission(s, false);
      await closed();
      s.host.config = async () => {
        throw new Error('timeout');
      };
      await s.refresh();
      await closed();
      for (const value of [
        null,
        {},
        { external_sessions: 'true' },
        { external_sessions: true, terminal: 1 },
      ]) {
        s.host.config = async () => value;
        await s.refresh();
        await closed();
      }
      const slow = deferred<unknown>();
      s.host.config = () => slow.promise;
      const pending = s.refresh();
      await setPermission(s, false);
      slow.resolve({ external_sessions: true });
      await pending;
      await closed();
      await setPermission(s, true);
      expect((await lookup(s)).last_reply?.text).toBe('fixture answer');
    }, false);
  });

  it('revokes lookups and admissions spanning off/on, including a stale negative inventory result', async () => {
    await withSyntheticHost(async (s) => {
      for (const phase of ['inventory', 'lookup', 'admission']) {
        const block = barrier();
        if (phase === 'inventory') s.host.beforeInventory = block.wait;
        else s.host.beforeRead = block.wait;
        const result = phase === 'admission' ? observe(s, 'stale') : lookup(s);
        await block.entered;
        await setPermission(s, false);
        await setPermission(s, true);
        s.host.inventoryOK = phase !== 'inventory';
        block.release();
        await expect(result).rejects.toThrow('reply access disabled');
        if (phase === 'inventory') expect(reads(s)).toHaveLength(0);
        s.host.inventoryOK = true;
      }
      expect(s.timers.size).toBe(0);
    });
  });

  it('pauses polling without host calls and discards work spanning disable or uncertain refresh', async () => {
    await withSyntheticHost(async (s) => {
      await observe(s, 'A');
      for (const stage of ['read', 'delivery']) {
        const block = barrier();
        if (stage === 'read') s.host.beforeRead = block.wait;
        else s.host.beforeDelivery = block.wait;
        const n = stage === 'read' ? 2 : 3;
        await appendFile(s.file, lines(answer(n, n - 1, 'not authorized for delivery')));
        const polling = s.poll();
        await block.entered;
        await setPermission(s, false);
        await setPermission(s, true);
        block.release();
        await polling;
        await s.poll(); // fresh baseline, never backfill the interrupted poll
        expect(seen(s, 'A')).toEqual([]);
      }
      s.host.config = async () => null;
      await s.refresh();
      const count = s.host.calls.length;
      await appendFile(s.file, lines(answer(4, 3, 'while disabled')));
      await s.poll();
      expect(s.host.calls).toHaveLength(count);
      await setPermission(s, true);
      await s.poll();
      expect(seen(s, 'A')).toEqual([]);
      await appendFile(s.file, lines(answer(5, 4, 'authorized')));
      await s.poll();
      expect(seen(s, 'A')).toEqual([{ id: recordId(5), text: 'authorized' }]);
      expect(new Set(s.host.calls.map((c) => c.function_id))).toEqual(
        new Set(['configuration::get', 'state::list', 'shell::exec', ON]),
      );
      expect(
        new Set(
          s.host.calls.filter((c) => c.function_id === 'shell::exec').map((c) => c.payload.command),
        ),
      ).toEqual(new Set(["'fixture-cli' agents --json --all", 'python3']));
    });
  });
});
