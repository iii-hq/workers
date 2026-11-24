/**
 * `claude::terminal::describe`: what the console page passes to
 * `shell::pty::open`. It prepares the session it describes, in the folder the
 * page asks for, every time it is called.
 */

import type { IIIClient } from 'iii-sdk';
import { describe, expect, it } from 'vitest';
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
  registerTerminal(
    iii,
    () => host,
    () => TerminalConfigSchema.parse({}),
  );
  const describeFn = handlers.get('claude::terminal::describe');
  if (!describeFn) throw new Error('claude::terminal::describe was not registered');
  return { claude, describeFn };
}

describe('claude::terminal::describe', () => {
  it('takes the folder per session, in its request schema', () => {
    const { describeFn } = setup();
    const schema = describeFn.options.request_format as { properties: Record<string, unknown> };
    expect(Object.keys(schema.properties)).toContain('cwd');
    const response = describeFn.options.response_format as { properties: Record<string, unknown> };
    expect(Object.keys(response.properties)).toEqual(
      expect.arrayContaining(['program', 'args', 'cwd', 'env', 'plugin', 'project_dir']),
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
});
