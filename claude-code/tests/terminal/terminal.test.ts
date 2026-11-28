/**
 * `claude::terminal::describe`: what the console page passes to
 * `shell::pty::open`. It prepares the session it describes, in the folder the
 * page asks for, every time it is called.
 */

import type { IIIClient } from 'iii-sdk';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { type CliRow, registerAgents, type ResolveOpen } from '../../src/agents.js';
import { TerminalConfigSchema } from '../../src/config.js';
import { registerTerminal } from '../../src/terminal/terminal.js';
import type { HostPrepared } from '../../src/terminal/workspace.js';
import type { SessionRecord } from '../../src/types.js';
import { CLAUDE, fakeClaude } from '../_helpers/fake-claude.js';

// The worker's own runs in flight, by iii session id.
const live = new Set<string>();
vi.mock('../../src/run.js', async (original) => ({
  ...(await original<typeof import('../../src/run.js')>()),
  isLive: (id: string) => live.has(id),
}));
// What `tmux list-panes -a` and `ps` print on the terminal host; null: neither exists.
let hostTools: null | { panes: () => string; ps: string } = null;
beforeEach(() => {
  live.clear();
  hostTools = null;
});

type Handler = (payload: unknown) => Promise<Record<string, unknown>>;

function setup(
  hostOver: Partial<HostPrepared> = {},
  inventory?: () => CliRow[],
  visibilityEpoch: () => number | null = () => 1,
  records: SessionRecord[] = [],
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
          if (hostTools && (p.command === 'tmux' || p.command === 'ps')) {
            const stdout = p.command === 'tmux' ? hostTools.panes() : hostTools.ps;
            return { stdout, stderr: '', exit_code: 0 };
          }
          if (Array.isArray(p.args)) return claude.run(p.command, p.args, p.cwd);
          if (inventory && String(p.command).endsWith(' agents --json --all')) {
            return { stdout: JSON.stringify(inventory()), stderr: '', exit_code: 0 };
          }
          return { stdout: '', stderr: '', exit_code: 1 };
        case 'state::list':
          return p.scope === 'claude_sessions' ? records : [];
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
  // The terminal host can be replaced (a new binary or workspace) while a describe runs.
  let current = host;
  const swapHost = () => {
    current = { ...host };
  };
  registerTerminal(
    iii,
    () => current,
    () => TerminalConfigSchema.parse({}),
    resolveOpen,
    visibilityEpoch,
  );
  const describeFn = handlers.get('claude::terminal::describe');
  if (!describeFn) throw new Error('claude::terminal::describe was not registered');
  return { claude, describeFn, resolveOpen, swapHost };
}

describe('claude::terminal::describe', () => {
  it('takes the folder per session, in its request schema', () => {
    const { describeFn } = setup();
    const schema = describeFn.options.request_format as { properties: Record<string, unknown> };
    expect(schema.properties.mode).toMatchObject({ enum: ['fork', 'resume'] });
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
      /invalid option/i,
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
    { ...fork, mode: 'original' },
    { mode: 'resume', session: id },
  ])('rejects incomplete or unrecognized mode requests: %j', async (payload) => {
    const { describeFn, resolveOpen, claude } = setup();
    await expect(describeFn.handler(payload)).rejects.toThrow();
    expect(resolveOpen).not.toHaveBeenCalled();
    expect(claude.installs).toEqual([]);
  });

  describe('resume mode', () => {
    const resume = { ...legacy, mode: 'resume', claude_session_id: id };
    const elsewhere = /^This session is open in another terminal\. Close it there to resume it\.$/;
    const background: CliRow = {
      id: 'abcd1234',
      sessionId: id,
      kind: 'background',
      cwd: '/fixture',
      pid: 41,
      state: 'working',
    };

    it('resume mode returns --resume for a session no process runs', async () => {
      const row: CliRow = { sessionId: id, kind: 'interactive', cwd: '/fixture', status: 'idle' };
      const { describeFn } = setup({}, () => [row]);
      const spec = await describeFn.handler(resume);
      expect(spec).toMatchObject({ cwd: '/fixture', claude_session_id: id });
      expect((spec.args as string[]).slice(-2)).toEqual(['--resume', id]);
      expect(spec.args).not.toContain('--fork-session');
      expect(spec).not.toHaveProperty('copy');
    });

    it('resume mode returns attach for a running background session', async () => {
      const { describeFn } = setup({}, () => [background]);
      const spec = await describeFn.handler(resume);
      expect(spec).toMatchObject({ args: ['attach', 'abcd1234'], claude_session_id: id });
      expect(spec).not.toHaveProperty('copy');
    });

    it('resume mode returns --resume for a finished background session', async () => {
      const finished: CliRow = { ...background, pid: undefined, state: 'done' };
      const { describeFn } = setup({}, () => [finished]);
      const spec = await describeFn.handler(resume);
      expect((spec.args as string[]).slice(-2)).toEqual(['--resume', id]);
      expect(spec.args).not.toContain('--fork-session');
      expect(spec.args).not.toContain('attach');
      expect(spec).not.toHaveProperty('copy');
    });

    it('resume mode refuses a session running in another terminal', async () => {
      const row: CliRow = { sessionId: id, kind: 'interactive', cwd: '/fixture', pid: 7 };
      const { claude, describeFn } = setup({}, () => [row]);
      await expect(describeFn.handler(resume)).rejects.toThrow(elsewhere);
      expect(claude.installs).toEqual([]);
    });

    it('resume mode refuses a conversation running under another row', async () => {
      const run = (session_id: string): SessionRecord => ({
        session_id,
        claude_session_id: id,
        cwd: '/fixture',
        model: 'm',
        status: 'done',
        turns: 1,
        total_cost_usd: 0,
        usage: null,
        updated_at_ms: 1,
      });
      live.add('running-run');
      const { claude, describeFn } = setup({}, () => [], undefined, [
        run('running-run'),
        run('stopped-run'),
      ]);
      const request = { session: 'stopped-run', mode: 'resume', claude_session_id: id };
      await expect(describeFn.handler(request)).rejects.toThrow(elsewhere);
      expect(claude.installs).toEqual([]);
    });

    it('resume mode refuses a second resume while the first is opening', async () => {
      const row: CliRow = { sessionId: id, kind: 'interactive', cwd: '/fixture', status: 'idle' };
      const { claude, describeFn } = setup({}, () => [row]);
      const run = claude.run;
      let release = () => {};
      const paused = new Promise<void>((resolve) => {
        release = resolve;
      });
      let entered = () => {};
      const preparing = new Promise<void>((resolve) => {
        entered = resolve;
      });
      vi.spyOn(claude, 'run').mockImplementation(async (program, args, folder) => {
        entered();
        await paused;
        return run(program, args, folder);
      });
      const first = describeFn.handler(resume);
      await preparing;
      await expect(describeFn.handler(resume)).rejects.toThrow(elsewhere);
      release();
      // The guard the first call set does not refuse its own revalidation.
      const spec = await first;
      expect((spec.args as string[]).slice(-2)).toEqual(['--resume', id]);
      expect(spec).not.toHaveProperty('copy');
    });

    it('resume mode refuses a running background session without a short id', async () => {
      const { describeFn } = setup({}, () => [{ ...background, id: undefined }]);
      await expect(describeFn.handler(resume)).rejects.toThrow(
        /^the background session has no short id to open it by$/,
      );
    });

    it('resume mode refuses a mismatched conversation', async () => {
      const row: CliRow = { sessionId: id, kind: 'interactive', cwd: '/fixture', status: 'idle' };
      const { claude, describeFn } = setup({}, () => [row]);
      const other = '12345678-1111-4000-8000-000000000002';
      await expect(describeFn.handler({ ...resume, claude_session_id: other })).rejects.toThrow(
        /changed or is unavailable/,
      );
      expect(claude.installs).toEqual([]);
    });

    describe('a live session in a tmux pane', () => {
      // claude runs as pid 42 under the shell (pid 40) of pane %25, window @4, session $0.
      const tmuxRow: CliRow = { sessionId: id, kind: 'interactive', cwd: '/fixture', pid: 42 };
      const ATTACH = [
        'attach-session',
        '-t',
        '$0',
        ';',
        'select-window',
        '-t',
        '@4',
        ';',
        'select-pane',
        '-t',
        '%25',
      ];
      const inPane = (panes: () => string = () => '40 $0 @4 %25\n') => {
        hostTools = { panes, ps: '1 0\n40 1\n42 40\n' };
      };

      it('resume mode returns the tmux attach of its pane, unprefixed', async () => {
        inPane();
        const { describeFn } = setup({}, () => [tmuxRow]);
        const spec = await describeFn.handler(resume);
        expect(spec).toMatchObject({ program: 'tmux', args: ATTACH, claude_session_id: id });
        expect(spec).not.toHaveProperty('copy');
      });

      it('the tmux attach runs with TMUX set to the empty string', async () => {
        inPane();
        const { describeFn } = setup({}, () => [tmuxRow]);
        const spec = await describeFn.handler(resume);
        expect((spec.env as Record<string, string>).TMUX).toBe('');
        expect(spec.env).toMatchObject({ III_URL: 'ws://127.0.0.1:49134' });
      });

      it('refuses when the tmux pane changed between the two resolves', async () => {
        let reads = 0;
        inPane(() => (++reads === 1 ? '40 $0 @4 %25\n' : '40 $0 @4 %26\n'));
        const { describeFn } = setup({}, () => [tmuxRow]);
        await expect(describeFn.handler(resume)).rejects.toThrow(
          /^the session changed while opening; refresh the list$/,
        );
        expect(reads).toBe(2);
      });

      it('resume mode refuses a tmux plan whose ids do not have tmux shapes', async () => {
        const { describeFn, resolveOpen } = setup();
        const bad = [...ATTACH];
        bad[10] = '%1;kill';
        resolveOpen.mockResolvedValue({
          program: 'tmux',
          args: bad,
          cwd: '/fixture',
          env: { TMUX: '' },
        });
        await expect(describeFn.handler(resume)).rejects.toThrow(/^no resume plan/);
      });

      it('legacy open of a tmux row still returns a --fork-session copy', async () => {
        inPane();
        const { describeFn } = setup({}, () => [tmuxRow]);
        const spec = await describeFn.handler(legacy);
        expect(spec).toMatchObject({ copy: true, program: CLAUDE });
        expect((spec.args as string[]).slice(-3)).toEqual(['--resume', id, '--fork-session']);
        expect(spec.env).not.toHaveProperty('TMUX');
      });
    });
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

  describe('host change while opening', () => {
    const hostChanged = /^the terminal host changed while opening; refresh the list$/;
    const resume = { args: ['--resume', 'x'], cwd: '/repo' };
    const forkPlan = () => ({
      args: ['--resume', id, '--fork-session'],
      cwd: '/repo',
      copy: true as const,
    });

    // Runs `during` the first time preparation touches the host.
    function duringPreparation(claude: ReturnType<typeof setup>['claude'], during: () => void) {
      const run = claude.run;
      let changed = false;
      vi.spyOn(claude, 'run').mockImplementation(async (program, args, folder) => {
        if (!changed) {
          changed = true;
          during();
        }
        return run(program, args, folder);
      });
    }

    it('refuses a spec when the host changes during preparation', async () => {
      const { claude, describeFn, resolveOpen, swapHost } = setup();
      resolveOpen.mockResolvedValueOnce(resume);
      duringPreparation(claude, swapHost);
      await expect(describeFn.handler({ session: 's' })).rejects.toThrow(hostChanged);
    });

    it('refuses a fork when the host changes during the fork re-resolve', async () => {
      const { describeFn, resolveOpen, swapHost } = setup();
      resolveOpen.mockResolvedValueOnce(forkPlan()).mockImplementationOnce(async () => {
        swapHost();
        return forkPlan();
      });
      await expect(describeFn.handler(fork)).rejects.toThrow(hostChanged);
      expect(resolveOpen).toHaveBeenCalledTimes(2);
    });

    it('reports a host change instead of the preparation error', async () => {
      const { claude, describeFn, resolveOpen, swapHost } = setup();
      resolveOpen.mockResolvedValueOnce(resume);
      claude.missing.add('/repo');
      duringPreparation(claude, swapHost);
      await expect(describeFn.handler({ session: 's' })).rejects.toThrow(hostChanged);
    });

    it('a visibility change wins over a host change when preparation fails', async () => {
      let epoch = 1;
      const { claude, describeFn, resolveOpen, swapHost } = setup({}, undefined, () => epoch);
      resolveOpen.mockResolvedValueOnce(resume);
      claude.missing.add('/repo');
      duringPreparation(claude, () => {
        swapHost();
        epoch = 2;
      });
      await expect(describeFn.handler({ session: 's' })).rejects.toThrow(
        /^session visibility changed; refresh the list$/,
      );
    });

    it('unchanged host still returns the spec', async () => {
      const { describeFn, resolveOpen } = setup();
      resolveOpen.mockResolvedValueOnce(forkPlan()).mockResolvedValueOnce(forkPlan());
      await expect(describeFn.handler(fork)).resolves.toMatchObject({
        cwd: '/repo',
        copy: true,
        claude_session_id: id,
      });
    });

    it('describe without a session ignores a host change', async () => {
      const { claude, describeFn, swapHost } = setup();
      duringPreparation(claude, swapHost);
      await expect(describeFn.handler({ cwd: '/work/app' })).resolves.toMatchObject({
        program: CLAUDE,
        cwd: '/work/app',
      });
    });
  });
});
