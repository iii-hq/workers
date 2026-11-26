/**
 * `claude::terminal::describe`: what the console page passes to
 * `shell::pty::open`. It prepares the session it describes, in the folder the
 * page asks for, every time it is called.
 */

import type { IIIClient } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import type { ResolveOpen } from '../../src/agents.js';
import { TerminalConfigSchema } from '../../src/config.js';
import { registerTerminal } from '../../src/terminal/terminal.js';
import type { HostPrepared } from '../../src/terminal/workspace.js';
import { CLAUDE, fakeClaude } from '../_helpers/fake-claude.js';

type Handler = (payload: unknown) => Promise<Record<string, unknown>>;

function setup(hostOver: Partial<HostPrepared> = {}) {
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
          return { stdout: '', stderr: '', exit_code: 1 };
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
  registerTerminal(
    iii,
    () => host,
    () => TerminalConfigSchema.parse({}),
    resolveOpen,
  );
  const describeFn = handlers.get('claude::terminal::describe');
  if (!describeFn) throw new Error('claude::terminal::describe was not registered');
  return { claude, describeFn, resolveOpen };
}

describe('claude::terminal::describe', () => {
  it('takes the folder per session, in its request schema', () => {
    const { describeFn } = setup();
    const schema = describeFn.options.request_format as { properties: Record<string, unknown> };
    expect(Object.keys(schema.properties)).toEqual(
      expect.arrayContaining(['cwd', 'session', 'mode', 'claude_session_id']),
    );
    const response = describeFn.options.response_format as { properties: Record<string, unknown> };
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
  const original = { session: id, mode: 'original', claude_session_id: id };
  it.each([
    { follow: true as const, claude_session_id: id },
    { pty_session_id: 'fixture-original' },
  ])('does not prepare a terminal for %j', async (plan) => {
    const { describeFn, resolveOpen, claude } = setup();
    resolveOpen.mockResolvedValueOnce(plan);
    expect(await describeFn.handler(original)).toEqual(plan);
    expect(resolveOpen).toHaveBeenCalledWith(
      id,
      { mode: 'original', claude_session_id: id },
      false,
    );
    expect(claude.installs).toEqual([]);
  });

  it('keeps repeat explicit original requests on native attach, even after describing a fork', async () => {
    const { describeFn, resolveOpen } = setup();
    const args = ['attach', 'abcd1234'];
    const cwd = '/fixture';
    resolveOpen.mockImplementation(async (_id, request) =>
      request?.mode === 'fork'
        ? { args: ['--resume', id, '--fork-session'], cwd, copy: true }
        : { args, cwd },
    );

    const first = await describeFn.handler(original);
    expect(first).toMatchObject({ args, cwd });
    expect(first).not.toHaveProperty('copy');
    expect(await describeFn.handler({ ...original, mode: 'fork' })).toMatchObject({ copy: true });
    const reopened = await describeFn.handler(original);
    expect(reopened).toMatchObject({ args, cwd });
    expect(reopened).not.toHaveProperty('copy');
  });

  it('will not launch a stale attach after preparation', async () => {
    const { describeFn, resolveOpen } = setup();
    resolveOpen
      .mockResolvedValueOnce({ args: ['attach', 'abcd1234'], cwd: '/fixture' })
      .mockResolvedValueOnce({ follow: true, claude_session_id: id });
    await expect(describeFn.handler(original)).rejects.toThrow(/changed/);
    expect(resolveOpen).toHaveBeenCalledTimes(2);
  });

  it('describes an explicit copy only for the verified target', async () => {
    const { describeFn, resolveOpen } = setup();
    const args = ['--resume', id, '--fork-session'];
    resolveOpen.mockResolvedValue({ args, cwd: '/fixture', copy: true });
    const spec = await describeFn.handler({ ...original, mode: 'fork' });
    expect(resolveOpen).toHaveBeenLastCalledWith(id, { mode: 'fork', claude_session_id: id });
    expect(spec.copy).toBe(true);
    expect((spec.args as string[]).slice(-3)).toEqual(args);
  });

  it.each([
    { mode: 'original' },
    { mode: 'fork', session: id },
    { ...original, claude_session_id: 'bad' },
    { ...original, mode: 'resume' },
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
