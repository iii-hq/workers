/**
 * `claude::terminal::describe`: what the console page passes to
 * `shell::pty::open`. It prepares the session it describes, in the folder the
 * page asks for, every time it is called.
 */

import type { IIIClient } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import { type CliRow, registerAgents, type ResolveOpen } from '../../src/agents.js';
import { TerminalConfigSchema } from '../../src/config.js';
import { registerTerminal } from '../../src/terminal/terminal.js';
import type { HostPrepared } from '../../src/terminal/workspace.js';
import { CLAUDE, fakeClaude } from '../_helpers/fake-claude.js';

type Handler = (payload: unknown) => Promise<Record<string, unknown>>;

function setup(
  hostOver: Partial<HostPrepared> = {},
  inventory?: () => CliRow[],
  visibilityEpoch: () => number | null = () => 1,
) {
  const claude = fakeClaude();
  const handlers = new Map<string, { handler: Handler; options: Record<string, unknown> }>();
  const iii = {
    registerFunction: (id: string, handler: Handler, options: Record<string, unknown>) => {
      handlers.set(id, { handler, options });
    },
    // biome-ignore lint/suspicious/noExplicitAny: minimal stand-in for the SDK client
    call: async (request: any): Promise<any> => {
      const p = request.payload ?? {};
      switch (request.function_id) {
        case 'shell::exec':
          if (Array.isArray(p.args)) return claude.run(p.command, p.args, p.cwd);
          if (inventory && String(p.command).endsWith(' agents --json --all')) {
            return { stdout: JSON.stringify(inventory()), stderr: '', exit_code: 0 };
          }
          return { stdout: '', stderr: '', exit_code: 1 };
        case 'state::list':
          return [];
        case 'shell::pty::sessions':
          return { sessions: [] };
        case 'http::status':
          return { url: 'http://127.0.0.1:3111' };
        case 'engine::functions::list':
          return { functions: [] };
        case 'compose::list':
          return {
            projects: [
              {
                file: '/home/u/proj/worker-compose.yaml',
                containers: [{ container: 'claude-code' }],
              },
            ],
          };
        default:
          throw new Error(`function_not_found: ${request.function_id}`);
      }
    },
  } as unknown as IIIClient;
  const host: HostPrepared = {
    home: '/ide-root/claude-code',
    executable: CLAUDE,
    env: { III_URL: 'ws://127.0.0.1:49134' },
    bridge: '/usr/local/bin/iii',
    detail: '',
    ...hostOver,
  };
  const resolveOpen = vi.fn<ResolveOpen>();
  if (inventory) {
    const agents = registerAgents(
      iii,
      () => ({ ...host, args: [], workspace: '' }),
      { has: () => false },
      {
        get epoch() {
          return visibilityEpoch();
        },
        confirm: async () => visibilityEpoch(),
      },
    );
    resolveOpen.mockImplementation(agents.resolveOpen);
  }
  registerTerminal(
    iii,
    () => host,
    () => TerminalConfigSchema.parse({}),
    resolveOpen,
    visibilityEpoch,
  );
  const describeFn = handlers.get('claude::terminal::describe');
  if (!describeFn) throw new Error('claude::terminal::describe was not registered');
  return { claude, describeFn, resolveOpen };
}

describe('claude::terminal::describe', () => {
  it('takes the folder per session, in its request schema', () => {
    const { describeFn } = setup();
    const schema = describeFn.options.request_format as { properties: Record<string, unknown> };
    expect(schema.properties.mode).toMatchObject({ const: 'fork' });
    expect(Object.keys(schema.properties)).toEqual(
      expect.arrayContaining(['cwd', 'session', 'mode', 'claude_session_id']),
    );
    const response = describeFn.options.response_format as { properties: Record<string, unknown> };
    expect(response.properties).not.toHaveProperty('follow');
    expect(Object.keys(response.properties)).toEqual(
      expect.arrayContaining(['program', 'args', 'cwd', 'env', 'plugin', 'project_dir', 'copy']),
    );
  });

  it('describes a session in the folder asked for, with the plugin checked there', async () => {
    const { claude, describeFn } = setup();
    const spec = await describeFn.handler({ cwd: '/work/app' });
    expect(spec).toMatchObject({
      program: CLAUDE,
      cwd: '/work/app',
      project_dir: '/home/u/proj',
      plugin: { state: 'installed', folder: '/work/app' },
      activity_bridge: '/usr/local/bin/iii',
    });
    expect(claude.installs.map((i) => i.projectPath)).toEqual(['/work/app']);
  });

  it('describes a session in the compose project folder when none is asked for', async () => {
    const { describeFn } = setup();
    expect((await describeFn.handler({})).cwd).toBe('/home/u/proj');
    expect((await describeFn.handler(undefined)).cwd).toBe('/home/u/proj');
    expect((await describeFn.handler({ cwd: '   ' })).cwd).toBe('/home/u/proj');
  });

  it('fails with the reason when the host has no claude', async () => {
    const { describeFn } = setup({ executable: '', detail: 'claude could not be installed' });
    await expect(describeFn.handler({ cwd: '/w' })).rejects.toThrow(/could not be installed/);
  });

  it('fails without a claude before looking the asked session up', async () => {
    const { describeFn, resolveOpen } = setup({ executable: '', detail: 'no claude' });
    await expect(describeFn.handler({ session: 's' })).rejects.toThrow('no claude');
    expect(resolveOpen).not.toHaveBeenCalled();
  });

  it('does not look a session up when none is asked for', async () => {
    const { describeFn, resolveOpen } = setup();
    const spec = await describeFn.handler({ cwd: '/work/app' });
    expect(spec).not.toHaveProperty('copy');
    expect(resolveOpen).not.toHaveBeenCalled();
  });

  it("resumes a listed session in its own folder, prepared there, after the session's own argv", async () => {
    const { claude, describeFn, resolveOpen } = setup();
    const plain = await describeFn.handler({ cwd: '/repo' });
    resolveOpen.mockResolvedValueOnce({ args: ['--resume', 'x'], cwd: '/repo' });
    const spec = await describeFn.handler({ session: 's', cwd: '/elsewhere' });
    expect(resolveOpen).toHaveBeenCalledWith('s');
    expect(spec).toMatchObject({ cwd: '/repo', plugin: { folder: '/repo' } });
    expect(spec.args).toEqual([...(plain.args as string[]), '--resume', 'x']);
    expect(spec).not.toHaveProperty('copy');
    expect(claude.installs.map((i) => i.projectPath)).toEqual(['/repo']);
  });

  it('opens a copy of a session running elsewhere, and says so', async () => {
    const { describeFn, resolveOpen } = setup();
    const copy = { args: ['--resume', 'x', '--fork-session'], cwd: '/repo', copy: true as const };
    resolveOpen.mockResolvedValueOnce(copy);
    const spec = await describeFn.handler({ session: 's' });
    expect(spec).toMatchObject({ cwd: '/repo', copy: true });
    expect((spec.args as string[]).slice(-3)).toEqual(['--resume', 'x', '--fork-session']);
  });

  const id = '12345678-1111-4000-8000-000000000001';
  const legacy = { session: id };
  const fork = { ...legacy, mode: 'fork', claude_session_id: id };

  it('requires live permission for a previously known external id even without a mode', async () => {
    const cwd = '/fixture/private-legacy-directory';
    let epoch: number | null = null;
    const row: CliRow = { sessionId: id, kind: 'interactive', cwd, status: 'idle' };
    const { claude, describeFn } = setup(
      {},
      () => [row],
      () => epoch,
    );
    const run = vi.spyOn(claude, 'run');
    const hidden = /^no session with that id is listed$/;

    await expect(describeFn.handler(legacy)).rejects.toThrow(hidden);
    expect(run).not.toHaveBeenCalled();

    epoch = 1;
    await expect(describeFn.handler(legacy)).resolves.toMatchObject({
      cwd,
      args: expect.arrayContaining(['--resume', id]),
    });

    epoch = null;
    run.mockClear();
    await expect(describeFn.handler(legacy)).rejects.toThrow(hidden);
    expect(run).not.toHaveBeenCalled();
  });
  it('rejects original mode without resolving or preparing a terminal', async () => {
    const { describeFn, resolveOpen, claude } = setup();
    resolveOpen.mockResolvedValueOnce({ args: ['--resume', id], cwd: '/fixture' });
    await expect(describeFn.handler({ ...fork, mode: 'original' })).rejects.toThrow(
      /invalid input/i,
    );
    expect(resolveOpen).not.toHaveBeenCalled();
    expect(claude.installs).toEqual([]);
  });

  it('does not advertise a lease-only reopen option', () => {
    const { describeFn } = setup();
    expect(describeFn.options).not.toHaveProperty('request_format.properties.reopen_held_resume');
  });

  it('describes separate forks of working and finished background conversations', async () => {
    const live: CliRow = {
      id: 'abcd1234',
      sessionId: id,
      kind: 'background',
      cwd: '/fixture',
      pid: 41,
      state: 'working',
    };
    let row = live;
    const { describeFn } = setup({}, () => [row]);
    for (const snapshot of [live, { ...live, pid: undefined, state: 'done' }]) {
      row = snapshot;
      expect(await describeFn.handler(fork)).toMatchObject({
        copy: true,
        claude_session_id: id,
        args: expect.arrayContaining(['--resume', id, '--fork-session']),
      });
    }
  });

  it.each([
    { payload: fork, next: null, missing: true },
    { payload: fork, next: 2, missing: true },
    { payload: legacy, next: null, missing: true },
    { payload: legacy, next: 2, missing: false },
  ])('redacts terminal preparation after permission changes: %j', async ({
    payload,
    next,
    missing,
  }) => {
    const cwd = '/fixture/private-fork-directory';
    let epoch: number | null = 1;
    const row: CliRow = { sessionId: id, kind: 'interactive', cwd, state: 'idle' };
    const { claude, describeFn } = setup(
      {},
      () => [row],
      () => epoch,
    );
    if (missing) {
      claude.missing.add(cwd);
      // A caller who still has access receives the actionable preparation error.
      await expect(describeFn.handler(payload)).rejects.toThrow(
        `the folder ${cwd} is not reachable`,
      );
    }

    const run = claude.run;
    let release = () => {};
    const paused = new Promise<void>((resolve) => {
      release = resolve;
    });
    let entered = () => {};
    const checkingFolder = new Promise<void>((resolve) => {
      entered = resolve;
    });
    vi.spyOn(claude, 'run').mockImplementation(async (program, args, folder) => {
      if (program === 'pwd' && folder === cwd) {
        entered();
        await paused;
      }
      return run(program, args, folder);
    });
    const opening = describeFn.handler(payload);
    await checkingFolder;
    epoch = next;
    release();
    await expect(opening).rejects.toThrow(/^session visibility changed; refresh the list$/);
    if (missing) expect(claude.installs).toEqual([]);
  });

  it('refuses a fork if fresh inventory changes after preparation', async () => {
    const { describeFn, resolveOpen } = setup();
    const plan = { args: ['--resume', id, '--fork-session'], cwd: '/fixture', copy: true } as const;
    resolveOpen
      .mockResolvedValueOnce({ ...plan, args: [...plan.args] })
      .mockResolvedValueOnce({ ...plan, args: [...plan.args], cwd: '/different' });
    await expect(describeFn.handler(fork)).rejects.toThrow(/changed/);
  });

  it('describes an explicit copy only for the verified target', async () => {
    const { describeFn, resolveOpen } = setup();
    const args = ['--resume', id, '--fork-session'];
    resolveOpen.mockResolvedValue({ args, cwd: '/fixture', copy: true });
    const spec = await describeFn.handler(fork);
    expect(resolveOpen).toHaveBeenLastCalledWith(id, { mode: 'fork', claude_session_id: id });
    expect(spec).toMatchObject({ copy: true, claude_session_id: id });
    expect((spec.args as string[]).slice(-3)).toEqual(args);
  });

  it.each([
    { mode: 'fork', session: id },
    { ...fork, claude_session_id: 'bad' },
    { ...fork, mode: 'resume' },
  ])('rejects incomplete or unrecognized mode requests: %j', async (payload) => {
    const { describeFn, resolveOpen, claude } = setup();
    await expect(describeFn.handler(payload)).rejects.toThrow();
    expect(resolveOpen).not.toHaveBeenCalled();
    expect(claude.installs).toEqual([]);
  });

  it('attaches to a background session with nothing but attach', async () => {
    const { describeFn, resolveOpen } = setup();
    resolveOpen.mockResolvedValueOnce({ args: ['attach', '7e64247f'], cwd: '' });
    const spec = await describeFn.handler({ session: 's' });
    expect(spec).toMatchObject({ args: ['attach', '7e64247f'], cwd: '/home/u/proj' });
  });

  it('refuses with the reason the session cannot be opened', async () => {
    const { describeFn, resolveOpen } = setup();
    resolveOpen.mockRejectedValueOnce(new Error('no session with that id is listed'));
    await expect(describeFn.handler({ session: 's' })).rejects.toThrow(
      'no session with that id is listed',
    );
  });

  it.each(['', 'x'.repeat(201), 7])('rejects the session %j', async (session) => {
    const { describeFn, resolveOpen } = setup();
    await expect(describeFn.handler({ session })).rejects.toThrow();
    expect(resolveOpen).not.toHaveBeenCalled();
  });

  it('keeps the function out of the catalog and traces', () => {
    const { describeFn } = setup();
    expect(describeFn.options.metadata).toEqual({ internal: true, trace_hidden: true });
  });
});
