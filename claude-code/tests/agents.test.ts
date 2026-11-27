import { appendFile } from 'node:fs/promises';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type CliRow, mergeInventory, parseAgentsJson, registerAgents } from '../src/agents.js';
import { CONFIG_ID, observeConfigChanges } from '../src/configuration.js';
import type { SessionRecord } from '../src/types.js';
import {
  deferred,
  record as transcriptRecord,
  recordId,
  withSyntheticHost,
} from './_helpers/external-replies-host.js';
import { type FakeIii, fakeIii } from './_helpers/fake-iii.js';

// The runs this worker holds, without driving a real one.
const live = vi.hoisted(() => new Set<string>());
vi.mock('../src/run.js', async (original) => ({
  ...(await original<typeof import('../src/run.js')>()),
  isLive: (id: string) => live.has(id),
}));
beforeEach(() => live.clear());

function record(session_id: string, status: SessionRecord['status'], claude?: string) {
  return {
    session_id,
    claude_session_id: claude ?? null,
    cwd: '/work',
    model: '',
    status,
    turns: 1,
    total_cost_usd: 0,
    usage: null,
    updated_at_ms: 1,
  } satisfies SessionRecord;
}

const interactive = (sessionId: string, status: unknown, extra: CliRow = {}): CliRow => ({
  pid: 42,
  cwd: '/repo',
  kind: 'interactive',
  startedAt: 1000,
  sessionId,
  name: `n-${sessionId}`,
  status,
  ...extra,
});

const background = (sessionId: string, state: unknown, extra: CliRow = {}): CliRow => ({
  id: sessionId.slice(0, 8),
  cwd: '/repo',
  kind: 'background',
  startedAt: 2000,
  sessionId,
  name: `bg-${sessionId}`,
  state,
  ...extra,
});

function merge(cliRows: CliRow[], extra: Partial<Parameters<typeof mergeInventory>[0]> = {}) {
  return mergeInventory({
    records: [],
    isLive: () => false,
    cliRows,
    known: () => true,
    ptyOf: () => undefined,
    external: true,
    ...extra,
  });
}

const byId = (agents: Array<{ id: string }>, id: string) => agents.find((a) => a.id === id);

describe('parseAgentsJson', () => {
  it('reads an array, and nothing else', () => {
    expect(parseAgentsJson('[{"sessionId":"a"},null,3]')).toEqual([{ sessionId: 'a' }]);
    expect(parseAgentsJson('[]')).toEqual([]);
    expect(parseAgentsJson('{"sessionId":"a"}')).toBeNull();
    expect(parseAgentsJson('Error: not logged in')).toBeNull();
    expect(parseAgentsJson('')).toBeNull();
  });
});

describe('mergeInventory', () => {
  it('maps interactive sessions', () => {
    const { agents } = merge([
      interactive('i-busy', 'busy'),
      interactive('i-idle', 'idle'),
      interactive('i-wait', 'waiting', { waitingFor: 'permission' }),
      interactive('i-shell', 'shell'),
    ]);
    expect(byId(agents, 'i-busy')).toEqual({
      id: 'i-busy',
      kind: 'interactive',
      status: 'busy',
      alive: true,
      claude_session_id: 'i-busy',
      name: 'n-i-busy',
      cwd: '/repo',
      started_at_ms: 1000,
    });
    expect(byId(agents, 'i-idle')?.status).toBe('idle');
    expect(byId(agents, 'i-wait')).toMatchObject({ status: 'waiting', waiting_for: 'permission' });
    expect(byId(agents, 'i-shell')?.status).toBe('busy');
  });

  it('maps background sessions, alive only with a pid', () => {
    const { agents } = merge([
      background('b-work', 'working', { pid: 7, status: 'busy' }),
      background('b-block', 'blocked', { pid: 8, waitingFor: 'input' }),
      background('b-done', 'done'),
      background('b-fail', 'failed'),
      background('b-stop', 'stopped'),
      background('b-work-nopid', 'working'),
    ]);
    expect(byId(agents, 'b-work')).toMatchObject({
      kind: 'background',
      status: 'busy',
      alive: true,
      short_id: 'b-work',
    });
    expect(byId(agents, 'b-block')).toMatchObject({ status: 'waiting', waiting_for: 'input' });
    expect(byId(agents, 'b-done')).toMatchObject({ status: 'done', alive: false });
    expect(byId(agents, 'b-fail')?.status).toBe('error');
    expect(byId(agents, 'b-stop')?.status).toBe('stopped');
    expect(byId(agents, 'b-work-nopid')).toMatchObject({ status: 'busy', alive: false });
  });

  it('keeps the live row of a session listed twice', () => {
    const { agents } = merge([background('dup', 'stopped'), interactive('dup', 'idle')]);
    expect(agents).toHaveLength(1);
    expect(agents[0]).toMatchObject({ kind: 'interactive', status: 'idle', alive: true });
  });

  it('skips a row without a sessionId and reads unknown values as unknown', () => {
    const { agents } = merge([
      { kind: 'interactive', pid: 1, status: 'busy' },
      { sessionId: 'odd', kind: 'robot', status: 'dreaming', state: 42, name: 5 },
    ]);
    expect(agents).toEqual([
      { id: 'odd', kind: 'interactive', status: 'unknown', alive: false, claude_session_id: 'odd' },
    ]);
  });

  it('maps headless runs and drops their CLI row', () => {
    const { agents } = merge([interactive('cs-live', 'busy'), interactive('other', 'idle')], {
      records: [
        record('h-live', 'working', 'cs-live'),
        record('h-done', 'done'),
        record('h-err', 'error'),
        record('h-lost', 'working'),
      ],
      isLive: (id) => id === 'h-live',
    });
    expect(byId(agents, 'h-live')).toEqual({
      id: 'h-live',
      kind: 'headless',
      status: 'busy',
      alive: true,
      session_id: 'h-live',
      claude_session_id: 'cs-live',
      cwd: '/work',
      updated_at_ms: 1,
    });
    expect(byId(agents, 'h-done')).toMatchObject({ status: 'done', alive: false });
    expect(byId(agents, 'h-err')?.status).toBe('error');
    expect(byId(agents, 'h-lost')?.status).toBe('unknown');
    expect(byId(agents, 'cs-live')).toBeUndefined();
    expect(byId(agents, 'other')).toBeDefined();
  });

  it.each([
    true,
    false,
  ])('lists a run open in a terminal as its CLI row (external %s)', (external) => {
    const { agents, hidden } = merge([interactive('cs', 'idle'), background('cs', 'done')], {
      records: [record('h', 'done', 'cs'), record('h-same', 'done', 'cs'), record('h2', 'done')],
      known: () => false,
      external,
    });
    expect(agents.map((a) => a.id)).toEqual(['h2', 'cs']);
    expect(byId(agents, 'cs')).toMatchObject({ kind: 'interactive', alive: true });
    expect(hidden).toBe(0);
  });

  it('keeps the headless row while its run is live here, or no process has it', () => {
    const rows = [interactive('cs', 'busy'), background('cs-done', 'done')];
    const { agents } = merge(rows, {
      records: [
        record('h', 'working', 'cs'),
        record('h-same', 'done', 'cs'),
        record('h-done', 'done', 'cs-done'),
      ],
      isLive: (id) => id === 'h',
    });
    expect(agents.map((a) => a.id)).toEqual(['h', 'h-same', 'h-done']);
  });

  it('names the console terminal whose process runs the session', () => {
    const { agents } = merge(
      [
        interactive('mine', 'idle', { pid: 7 }),
        interactive('other', 'idle'),
        background('bg', 'done'),
      ],
      { ptyOf: (pid) => (pid === 7 ? 'pty-7' : undefined) },
    );
    expect(byId(agents, 'mine')).toMatchObject({ pty_session_id: 'pty-7' });
    expect(byId(agents, 'other')).not.toHaveProperty('pty_session_id');
    expect(byId(agents, 'bg')).not.toHaveProperty('pty_session_id');
  });

  it('lists a session of a console terminal, whatever external says', () => {
    const { agents, hidden } = merge(
      [interactive('mine', 'busy', { pid: 7 }), interactive('stranger', 'idle')],
      {
        external: false,
        known: () => false,
        ptyOf: (pid) => (pid === 7 ? 'pty-7' : undefined),
      },
    );
    expect(agents).toEqual([expect.objectContaining({ id: 'mine', pty_session_id: 'pty-7' })]);
    expect(hidden).toBe(1);
  });

  it('reads a run the operator stopped as stopped, and lists newest first', () => {
    const { agents } = merge([], {
      records: [
        { ...record('old', 'done'), updated_at_ms: 1 },
        { ...record('stopped', 'done'), stop_reason: 'interrupted', updated_at_ms: 3 },
        { ...record('new', 'done'), updated_at_ms: 2 },
      ],
    });
    expect(agents.map((a) => a.id)).toEqual(['stopped', 'new', 'old']);
    expect(byId(agents, 'stopped')?.status).toBe('stopped');
  });

  it('gives a short id only where claude::agents::stop would act on it', () => {
    const rows = [background('aaaaaaaa-1', 'working', { pid: 9 })];
    expect(merge(rows).agents[0].short_id).toBe('aaaaaaaa');
    expect(merge(rows, { external: false }).agents[0].short_id).toBeUndefined();
  });

  it('with external off, hides host sessions it does not know and counts them', () => {
    const { agents, hidden } = merge(
      [interactive('seen', 'idle'), interactive('stranger', 'busy'), background('bg', 'done')],
      { external: false, known: (id) => id === 'seen', records: [record('h', 'done')] },
    );
    expect(agents.map((a) => a.id)).toEqual(['h', 'seen']);
    expect(hidden).toBe(2);
  });
});

const prepared = {
  executable: '/opt/claude bin/claude',
  env: { HOME: '/home/u' },
  detail: '',
  workspace: '/ws',
  args: ['--plugin-dir', '/p'],
  bridge: '',
  plugin: '/p',
};

function host(
  exec: (command: string) => unknown,
  options: {
    external?: boolean;
    prepared?: typeof prepared;
    tracked?: string[];
    ptys?: () => unknown;
    visibilityEpoch?: () => number | null;
  } = {},
): FakeIii & ReturnType<typeof registerAgents> {
  const fake = fakeIii();
  const realCall = fake.iii.call.bind(fake.iii);
  (fake.iii as { call: unknown }).call = async (req: {
    function_id: string;
    payload: Record<string, unknown>;
  }) => {
    if (req.function_id === 'shell::pty::sessions') return options.ptys?.() ?? { sessions: [] };
    if (req.function_id !== 'shell::exec') return realCall(req as never);
    fake.calls.push({ function_id: req.function_id, payload: req.payload });
    return exec(String(req.payload.command));
  };
  const tracked = new Set(options.tracked ?? []);
  const agents = registerAgents(
    fake.iii,
    () => options.prepared ?? prepared,
    () => options.external ?? false,
    { has: (id) => tracked.has(id) },
    options.visibilityEpoch ?? (() => (options.external ? 1 : null)),
  );
  return { ...fake, ...agents };
}

const ok = (rows: unknown) => () => ({ stdout: JSON.stringify(rows), stderr: '', exit_code: 0 });

async function list(fake: FakeIii, payload: unknown = {}) {
  return (await fake.registered.get('claude::agents::list')?.(payload)) as {
    agents: Array<{ id: string; pty_session_id?: string }>;
    config_id?: string;
    external_visibility?: string | null;
    detail?: string;
  };
}

describe('claude::agents::list', () => {
  it('only returns the effective configuration entry ID when requested', async () => {
    const fake = host(ok([]), { external: true });
    expect(await list(fake)).not.toHaveProperty('config_id');
    const { config_id } = await list(fake, { include_config_id: true });
    expect(config_id).toBe(process.env.III_CONFIG_NAME?.trim() || 'claude-code');
    await observeConfigChanges(fake.iii, async () => {});
    expect(fake.observed).toContainEqual(
      expect.objectContaining({
        source: expect.objectContaining({
          function_id: 'configuration::on-updated',
          filters: expect.objectContaining({ 'input.id': { $eq: config_id } }),
        }),
      }),
    );
  });

  it('binds opt-in inventory to the current visibility snapshot without inventory reads on probe', async () => {
    let epoch: number | null = null;
    const makeWorker = () =>
      host(ok([interactive('external', 'idle')]), { visibilityEpoch: () => epoch });
    const fake = makeWorker();
    const probe = (worker: FakeIii = fake) =>
      worker.registered.get('claude::agents::visibility')?.({});

    expect(await probe()).toEqual({ external_visibility: null });
    epoch = 1;
    const visible = await probe();
    expect(visible).toEqual({ external_visibility: expect.any(String) });
    expect(await probe(makeWorker())).not.toEqual(visible);
    expect(fake.calls).toEqual([]);
    expect(await list(fake)).not.toHaveProperty('external_visibility');
    const { external_visibility } = await list(fake, { include_config_id: true });
    expect(visible).toEqual({ external_visibility });

    epoch = null;
    expect(await probe()).toEqual({ external_visibility: null });
    expect(await list(fake, { include_config_id: true })).toMatchObject({
      external_visibility: null,
      agents: [],
    });
    epoch = 2;
    expect(await probe()).not.toEqual(visible);
  });

  it('reconfirms a visibility lease against the live setting without listing sessions', async () => {
    let epoch: number | null = 1;
    const fake = host(ok([]), { visibilityEpoch: () => epoch });
    const visibility = fake.registered.get('claude::agents::visibility');
    const probe = () => visibility?.({ confirm_live_config: true });
    const key = `configuration/${CONFIG_ID}`;

    const lease = (await visibility?.({})) as { external_visibility: string };
    expect(lease).toEqual({ external_visibility: expect.any(String) });
    expect(await probe()).toEqual({ external_visibility: null });
    fake.state.set(key, { external_sessions: false });
    expect(await probe()).toEqual({ external_visibility: null });
    fake.state.set(key, { external_sessions: 'true' });
    expect(await probe()).toEqual({ external_visibility: null });
    fake.state.set(key, { external_sessions: true });
    expect(await probe()).toEqual({ ...lease, live_config_confirmed: true });

    const pending = probe();
    epoch = 2;
    expect(await pending).toEqual({ external_visibility: null });
    expect(await probe()).toEqual({
      external_visibility: expect.any(String),
      live_config_confirmed: true,
    });
    epoch = null;
    expect(await probe()).toEqual({ external_visibility: null });
    expect(fake.calls.map((call) => call.function_id)).toEqual(Array(6).fill('configuration::get'));
  });

  it('runs claude agents --json on the terminal host, --all when asked', async () => {
    const fake = host(ok([]), { external: true });
    await list(fake);
    await list(fake, { all: true });
    const execs = fake.calls.filter((c) => c.function_id === 'shell::exec');
    expect(execs.map((c) => c.payload.command)).toEqual([
      "'/opt/claude bin/claude' agents --json",
      "'/opt/claude bin/claude' agents --json --all",
    ]);
    expect(execs[0].payload).toMatchObject({ env: { HOME: '/home/u' }, timeout_ms: 15_000 });
  });

  it('knows a session through the tracker or a hook settle, and says how many it hid', async () => {
    const fake = host(
      ok([
        interactive('tracked', 'idle'),
        interactive('hooked', 'busy'),
        interactive('ran', 'idle'),
        interactive('stranger', 'idle'),
      ]),
      { tracked: ['tracked'] },
    );
    fake.state.set('claude_turns/hooked', { id: 'hooked', source: 'hook', status: 'completed' });
    fake.state.set('claude_turns/ran', { id: 'ran', source: 'run', status: 'completed' });
    fake.state.set('claude_sessions/h', record('h', 'done'));
    const res = await list(fake);
    expect(res.agents.map((a) => a.id)).toEqual(['h', 'tracked', 'hooked']);
    expect(res.detail).toBe(
      '2 sessions on the terminal host are hidden; set external_sessions: true to list them',
    );
  });

  it('keeps external names and directories hidden until live permission is confirmed', async () => {
    let epoch: number | null = null;
    const fake = host(
      ok([
        interactive('known', 'idle'),
        interactive('external', 'idle', { cwd: '/fixture/external-only' }),
      ]),
      { external: true, tracked: ['known'], visibilityEpoch: () => epoch },
    );
    const read = () => list(fake, { include_config_id: true });
    expect((await read()).agents.map((row) => row.id)).toEqual(['known']);
    epoch = 1;
    expect((await read()).agents).toMatchObject([
      { id: 'known' },
      { id: 'external', external_only: true, name: 'n-external', cwd: '/fixture/external-only' },
    ]);
    epoch = null;
    const revoked = await read();
    expect(revoked.agents.map((row) => row.id)).toEqual(['known']);
    expect(JSON.stringify(revoked)).not.toMatch(/n-external|external-only/);
  });

  it('withholds external rows from an inventory read spanning a permission change', async () => {
    let epoch = 1;
    let inventoryStarted!: () => void;
    let releaseInventory!: () => void;
    const started = new Promise<void>((resolve) => {
      inventoryStarted = resolve;
    });
    const release = new Promise<void>((resolve) => {
      releaseInventory = resolve;
    });
    const fake = host(
      async () => {
        inventoryStarted();
        await release;
        return ok([interactive('external', 'idle')])();
      },
      { external: true, visibilityEpoch: () => epoch },
    );
    const pending = list(fake, { include_config_id: true });
    await started;
    epoch++;
    releaseInventory();
    expect(await pending).toMatchObject({ agents: [], external_visibility: null });
  });

  it('reads the console terminal of a session, only from the claude page', async () => {
    const fake = host(
      ok([interactive('a', 'idle', { pid: 1 }), interactive('b', 'idle', { pid: 2 })]),
      {
        external: true,
        ptys: () => ({
          sessions: [
            { session_id: 'pty-a', pid: 1, ui: 'claude', status: 'attached' },
            { session_id: 'pty-b', pid: 2, ui: 'shell', status: 'attached' },
            { session_id: 'pty-c', pid: null, ui: 'claude', status: 'detached' },
          ],
        }),
      },
    );
    const { agents } = await list(fake);
    expect(agents.map((a) => a.pty_session_id)).toEqual(['pty-a', undefined]);
  });

  it('lists without the console terminals when they cannot be read', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const fake = host(ok([interactive('a', 'idle')]), {
      external: true,
      ptys: () => {
        throw new Error('no such function');
      },
    });
    const res = await list(fake);
    expect(res.agents.map((a) => [a.id, a.pty_session_id])).toEqual([['a', undefined]]);
    expect(res.detail).toBeUndefined();
    expect(warn).toHaveBeenCalledOnce();
    warn.mockRestore();
  });

  it.each([
    [
      'no executable',
      () => {
        throw new Error('not called');
      },
      { ...prepared, executable: '', detail: 'claude could not be installed' },
      'claude could not be installed',
    ],
    [
      'a failing exit',
      () => ({ stdout: '', stderr: `  unknown command ${'x'.repeat(300)}`, exit_code: 2 }),
      prepared,
      `claude agents --json failed: exit 2: unknown command ${'x'.repeat(184)}`,
    ],
    [
      'output that is not JSON',
      () => ({ stdout: 'Welcome!', stderr: '', exit_code: 0 }),
      prepared,
      'claude agents --json failed: output is not a JSON array',
    ],
    [
      'an exec that throws',
      () => {
        throw new Error('shell::exec timed out');
      },
      prepared,
      'claude agents --json failed: Error: shell::exec timed out',
    ],
  ])('lists headless runs only, with a detail, on %s', async (_name, exec, current, detail) => {
    const fake = host(exec, { prepared: current, external: true });
    fake.state.set('claude_sessions/h', record('h', 'done'));
    const res = await list(fake);
    expect(res.agents.map((a) => a.id)).toEqual(['h']);
    expect(res.detail).toBe(detail);
  });
});

describe('claude::agents::stop', () => {
  const BG = '7e64247f-5095-4a53-9b54-2d8d61b20356';
  const stop = async (fake: FakeIii, id = BG) =>
    (await fake.registered.get('claude::agents::stop')?.({ id })) as {
      stopped: boolean;
      reason?: string;
    };
  const commands = (fake: FakeIii) =>
    fake.calls.filter((c) => c.function_id === 'shell::exec').map((c) => c.payload.command);

  it('stops a live background session by the short id of a fresh listing', async () => {
    const fake = host(
      (command) =>
        command.endsWith('agents --json')
          ? ok([background(BG, 'working', { pid: 7, status: 'busy' })])()
          : { stdout: '', stderr: '', exit_code: 0 },
      { external: true },
    );
    expect(await stop(fake)).toEqual({ id: BG, stopped: true });
    expect(commands(fake)).toEqual([
      "'/opt/claude bin/claude' agents --json",
      "'/opt/claude bin/claude' stop 7e64247f",
    ]);
  });

  it.each([
    [
      'external_sessions is off',
      [background(BG, 'working', { pid: 7 })],
      false,
      'external_sessions is off',
    ],
    ['an unknown id', [], true, 'no background session with that id'],
    [
      'an interactive session',
      [interactive(BG, 'busy')],
      true,
      'no background session with that id',
    ],
    ['a session with no process', [background(BG, 'stopped')], true, 'not running'],
    [
      'a short id that is not 8 hex',
      [background(BG, 'working', { pid: 7, id: '7e6; rm' })],
      true,
      'no short id',
    ],
  ])('refuses %s without running claude stop', async (_name, rows, external, reason) => {
    const fake = host(ok(rows), { external });
    const res = await stop(fake);
    expect(res.stopped).toBe(false);
    expect(res.reason).toContain(reason);
    expect(commands(fake).some((c) => String(c).includes(' stop '))).toBe(false);
  });

  it('refuses before inventory when live permission is unconfirmed', async () => {
    const fake = host(ok([]), { external: true, visibilityEpoch: () => null });
    expect(await stop(fake)).toEqual({
      id: BG,
      stopped: false,
      reason: 'Terminal host inventory unavailable',
    });
    expect(fake.calls).toEqual([]);
  });

  it.each([
    [null, 'Terminal host inventory unavailable'],
    [2, 'Terminal host inventory unavailable'],
    [1, 'claude agents --json failed: exit 1: inventory unavailable at /fixture/diagnostic'],
  ])('publishes inventory failure with final permission epoch %s', async (finalEpoch, reason) => {
    let epoch: number | null = 1;
    const started = deferred<void>();
    const inventory = deferred<{ stdout: string; stderr: string; exit_code: number }>();
    const fake = host(
      () => {
        started.resolve();
        return inventory.promise;
      },
      { external: true, visibilityEpoch: () => epoch },
    );
    const pending = stop(fake);
    await started.promise;
    epoch = finalEpoch;
    inventory.resolve({
      stdout: '',
      stderr: 'inventory unavailable at /fixture/diagnostic',
      exit_code: 1,
    });
    expect(await pending).toEqual({ id: BG, stopped: false, reason });
    expect(fake.calls).toMatchObject([
      {
        function_id: 'shell::exec',
        payload: { command: "'/opt/claude bin/claude' agents --json" },
      },
    ]);
  });

  it('does not stop a session after permission changes during inventory', async () => {
    let epoch = 1;
    const fake = host(
      () => {
        epoch++;
        return ok([background(BG, 'working', { pid: 7 })])();
      },
      { external: true, visibilityEpoch: () => epoch },
    );
    expect(await stop(fake)).toEqual({
      id: BG,
      stopped: false,
      reason: 'Terminal host inventory unavailable',
    });
    expect(commands(fake)).toEqual(["'/opt/claude bin/claude' agents --json"]);
  });

  it.each([
    'exit failure',
    'exception',
  ])('returns a generic stop failure after permission changes: %s', async (failure) => {
    let epoch: number | null = 1;
    const fake = host(
      (command) => {
        if (command.endsWith('agents --json')) return ok([background(BG, 'working', { pid: 7 })])();
        epoch = null;
        const detail = 'stop unavailable at /fixture/diagnostic';
        if (failure === 'exception') throw new Error(detail);
        return { stdout: '', stderr: detail, exit_code: 1 };
      },
      { external: true, visibilityEpoch: () => epoch },
    );
    expect(await stop(fake)).toEqual({ id: BG, stopped: false, reason: 'claude stop failed' });
  });

  it('never reads a killed claude stop as stopped', async () => {
    const fake = host(
      (command) =>
        command.endsWith('agents --json')
          ? ok([background(BG, 'working', { pid: 7 })])()
          : { stdout: '', stderr: 'shell::exec exceeded its 30000ms timeout', exit_code: null },
      { external: true },
    );
    expect(await stop(fake)).toEqual({
      id: BG,
      stopped: false,
      reason: 'claude stop failed: exit -1: shell::exec exceeded its 30000ms timeout',
    });
  });

  it('reports a failing claude stop', async () => {
    const fake = host(
      (command) =>
        command.endsWith('agents --json')
          ? ok([background(BG, 'working', { pid: 7 })])()
          : { stdout: '', stderr: 'no such session\n', exit_code: 1 },
      { external: true },
    );
    expect(await stop(fake)).toEqual({
      id: BG,
      stopped: false,
      reason: 'claude stop failed: exit 1: no such session',
    });
  });
});

describe('claude::agents::follow-snapshot', () => {
  const function_id = 'claude::agents::follow-snapshot';

  it('reads an ordered burst and excludes a branch abandoned since the first view', async () => {
    await withSyntheticHost(async ({ client, claude_session_id, file }) => {
      const append = (...records: ReturnType<typeof transcriptRecord>[]) =>
        appendFile(file, records.map((entry) => JSON.stringify(entry)).join('\n') + '\n');
      const snapshot = (cursor?: string) =>
        client.call({
          function_id,
          payload: { claude_session_id, ...(cursor ? { cursor } : {}) },
        }) as Promise<{
          cursor: string;
          replies: Array<{ id: string; text: string }>;
          last_reply: { id: string; text: string } | null;
        }>;
      await append(transcriptRecord(2, 1, 'assistant', 'initial latest'));
      const initial = await snapshot();
      expect(initial).toMatchObject({
        replies: [],
        last_reply: { id: recordId(2), text: 'initial latest' },
      });
      const burst = Array.from({ length: 21 }, (_, i) => ({
        id: recordId(i + 3),
        text: 'Reply ' + (i + 1),
      }));
      await append(
        ...burst.map(({ text }, i) => transcriptRecord(i + 3, i + 2, 'assistant', text)),
      );
      const updated = await snapshot(initial.cursor);
      expect(updated.replies).toEqual(burst);
      expect(updated.last_reply).toEqual(burst.at(-1));
      expect((await snapshot(updated.cursor)).replies).toEqual([]);
      await append(transcriptRecord(24, 1, 'user', 'switch branch'));
      expect(await snapshot(initial.cursor)).toMatchObject({
        replies: [],
        last_reply: { id: recordId(1), text: 'fixture answer' },
      });
      await expect(snapshot('bad-' + initial.cursor)).rejects.toThrow('invalid follow cursor');
    });
  });

  it.each([
    'interactive',
    'background',
    'finished',
  ] as const)('follows a synthetic %s conversation without using a terminal', async (kind) => {
    await withSyntheticHost(async ({ host, client, claude_session_id, refresh }) => {
      const backgroundRow = kind !== 'interactive';
      host.cli[0] = {
        ...host.cli[0],
        kind: backgroundRow ? 'background' : 'interactive',
        pid: kind === 'finished' ? undefined : 42,
        ...(backgroundRow ? { state: kind === 'finished' ? 'done' : 'working' } : {}),
      };
      host.calls.length = 0;
      const snapshot = () => client.call({ function_id, payload: { claude_session_id } });
      expect(await snapshot()).toMatchObject({
        claude_session_id,
        last_reply: { id: recordId(1), text: 'fixture answer' },
      });
      expect(host.calls.filter((call) => call.function_id.startsWith('shell::pty::'))).toEqual([]);
      expect(
        host.calls
          .filter((call) => call.function_id === 'shell::exec')
          .map((call) => call.payload.command),
      ).toEqual(["'fixture-cli' agents --json --all", 'python3']);

      host.config = async () => ({ external_sessions: false });
      await refresh();
      await expect(snapshot()).rejects.toThrow('reply access disabled');
    });
  });

  it('does not read transcript data after a lookup loses its live visibility epoch', async () => {
    let epoch: number | null = 1;
    const id = '7e64247f-5095-4a53-9b54-2d8d61b20356';
    const fake = host(
      () => {
        epoch = null;
        return ok([interactive(id, 'idle')])();
      },
      { external: true, visibilityEpoch: () => epoch },
    );
    const snapshot = fake.registered.get(function_id);
    if (!snapshot) throw new Error('follow snapshot was not registered');
    await expect(snapshot({ claude_session_id: id })).rejects.toThrow('reply access disabled');
    expect(
      fake.calls
        .filter((call) => call.function_id === 'shell::exec')
        .map((call) => call.payload.command),
    ).toEqual(["'/opt/claude bin/claude' agents --json --all"]);
  });
});

describe('opening a listed session', () => {
  const ID = '7e64247f-5095-4a53-9b54-2d8d61b20356';
  const RESUME = ['--plugin-dir', '/p', '--resume', ID];
  const open = (
    rows: CliRow[],
    id: string,
    options: Parameters<typeof host>[1] & { records?: SessionRecord[] } = {},
  ) => {
    const fake = host(ok(rows), { external: true, ...options });
    for (const r of options.records ?? []) fake.state.set(`claude_sessions/${r.session_id}`, r);
    return fake.resolveOpen(id);
  };

  const fork = { mode: 'fork' as const, claude_session_id: ID };

  it('resolves an explicit fork separately from the live interactive original', async () => {
    const fake = host(ok([interactive(ID, 'idle')]), { external: true });
    expect(await fake.resolveOpen(ID, fork)).toEqual({
      args: [...RESUME, '--fork-session'],
      cwd: '/repo',
      copy: true,
    });
    expect(await fake.heldElsewhere(ID)).toBe(true);
  });

  it('refuses a fork on hidden, changed, missing or unavailable inventory', async () => {
    await expect(host(ok([interactive(ID, 'busy')])).resolveOpen(ID, fork)).rejects.toThrow(
      'no session with that id is listed',
    );
    const fake = host(ok([]));
    fake.state.set(
      'claude_sessions/h',
      record('h', 'done', '11223344-0000-4000-8000-000000000000'),
    );
    await expect(fake.resolveOpen('h', fork)).rejects.toThrow(/changed/);
    await expect(fake.resolveOpen(ID, fork)).rejects.toThrow('no session with that id is listed');
    await expect(
      host(() => ({ exit_code: 1, stderr: 'failed', stdout: '' })).resolveOpen(ID, fork),
    ).rejects.toThrow('session inventory unavailable');
  });

  it('does not grant an explicit fork across an off/on refresh', async () => {
    let epoch = 1;
    const fake = host(
      () => {
        epoch++;
        return ok([interactive(ID, 'busy')])();
      },
      { external: true, visibilityEpoch: () => epoch },
    );
    await expect(fake.resolveOpen(ID, fork)).rejects.toThrow(/visibility changed/);
  });

  it('resumes a finished worker run in its own directory with external visibility closed', async () => {
    const records = [record('h', 'done', ID)];
    expect(await open([], 'h', { records, external: false })).toEqual({
      args: RESUME,
      cwd: '/work',
    });
  });

  it('opens a copy of a run that is live here', async () => {
    live.add('h');
    const records = [record('h', 'working', ID), record('h-same', 'done', ID)];
    const copy = { args: [...RESUME, '--fork-session'], cwd: '/work', copy: true };
    expect(await open([interactive(ID, 'busy')], 'h', { records })).toEqual(copy);
    // The conversation is what must not get a second writer, whichever row names it.
    expect(await open([interactive(ID, 'busy')], 'h-same', { records })).toEqual(copy);
  });

  it('opens a copy of an interactive session, a run open in a terminal included', async () => {
    const copy = { args: [...RESUME, '--fork-session'], cwd: '/repo', copy: true };
    expect(await open([interactive(ID, 'idle')], ID)).toEqual(copy);
    const records = [record('h', 'done', ID)];
    expect(await open([interactive(ID, 'idle')], ID, { records, external: false })).toEqual(copy);
    // Its headless row is gone, so a stale one opens nothing.
    await expect(open([interactive(ID, 'idle')], 'h', { records })).rejects.toThrow(
      'no session with that id is listed',
    );
  });

  it('resumes an interactive session no process runs', async () => {
    const rows = [interactive(ID, 'idle', { pid: undefined })];
    expect(await open(rows, ID)).toEqual({ args: RESUME, cwd: '/repo' });
  });

  it('attaches to a background session by its short id', async () => {
    expect(await open([background(ID, 'working', { pid: 7 })], ID)).toEqual({
      args: ['attach', '7e64247f'],
      cwd: '/repo',
    });
    const noCwd = [background(ID, 'done', { cwd: undefined })];
    expect(await open(noCwd, ID)).toEqual({ args: ['attach', '7e64247f'], cwd: '/ws' });
  });

  it.each<[string, CliRow[], string, NonNullable<Parameters<typeof open>[2]>, string]>([
    ['an unknown id', [], ID, {}, 'no session with that id is listed'],
    [
      'a hidden session',
      [interactive(ID, 'idle')],
      ID,
      { external: false },
      'no session with that id is listed',
    ],
    [
      'a background session with external_sessions off',
      [background(ID, 'working', { pid: 7 })],
      ID,
      { external: false, tracked: [ID] },
      'external_sessions is off: this worker does not open host sessions',
    ],
    [
      'a background session whose short id is not 8 hex',
      [background(ID, 'working', { pid: 7, id: '--help' })],
      ID,
      {},
      'the background session has no short id to open it by',
    ],
    [
      'a run with no Claude session yet',
      [],
      'h',
      { records: [record('h', 'error')] },
      'this session has no Claude Code conversation to open',
    ],
    [
      'a Claude session id that is not a UUID',
      [interactive('--dangerously-skip-permissions', 'idle')],
      '--dangerously-skip-permissions',
      {},
      'this session has no Claude Code conversation to open',
    ],
    [
      'a session with no directory',
      [],
      'h',
      { records: [{ ...record('h', 'done', ID), cwd: '' }] },
      'this session has no recorded working directory',
    ],
  ])('refuses %s', async (_name, rows, id, options, reason) => {
    await expect(open(rows, id, options)).rejects.toThrow(reason);
  });

  it.each([
    [1, 'claude agents --json failed: exit 2: could not read /fixture/private-directory'],
    [null, 'session inventory unavailable'],
  ])('refuses to open when the listing fails, with diagnostics gated by epoch %s', async (epoch, reason) => {
    const fake = host(
      () => ({ stdout: '', stderr: 'could not read /fixture/private-directory', exit_code: 2 }),
      { external: true, visibilityEpoch: () => epoch },
    );
    fake.state.set('claude_sessions/h', record('h', 'done', ID));
    await expect(fake.resolveOpen('h')).rejects.toThrow(new Error(reason));
  });

  describe('before the CLI lists the process', () => {
    beforeEach(() => void vi.useFakeTimers({ toFake: ['Date'] }));
    afterEach(() => void vi.useRealTimers());

    it('gives one plain resume, then copies, for a while', async () => {
      const fake = host(ok([]));
      fake.state.set('claude_sessions/h', record('h', 'done', ID));
      const plain = { args: RESUME, cwd: '/work' };
      expect(await fake.heldElsewhere(ID)).toBe(false);
      expect(await fake.resolveOpen('h')).toEqual(plain);
      expect(await fake.resolveOpen('h')).toMatchObject({ copy: true });
      expect(await fake.heldElsewhere(ID)).toBe(true);
      vi.advanceTimersByTime(15_000);
      expect(await fake.heldElsewhere(ID)).toBe(false);
      expect(await fake.resolveOpen('h')).toEqual(plain);
    });
  });
});

describe('a conversation open on the terminal host', () => {
  it('is held by a live process, and by nothing else', async () => {
    const rows = [interactive('live', 'idle'), background('done', 'done')];
    const fake = host(ok(rows));
    expect(await fake.heldElsewhere('live')).toBe(true);
    expect(await fake.heldElsewhere('done')).toBe(false);
    expect(await fake.heldElsewhere('other')).toBe(false);
  });

  it('is held by nothing where the host cannot be read', async () => {
    const fake = host(() => ({ stdout: '', stderr: 'boom', exit_code: 2 }));
    expect(await fake.heldElsewhere('live')).toBe(false);
  });
});
