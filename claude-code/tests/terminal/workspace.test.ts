import type { IIIClient } from 'iii-sdk';
import { describe, expect, it } from 'vitest';
import { TerminalConfigSchema } from '../../src/config.js';
import {
  composeProjectDir,
  type HostPrepared,
  MODE_FILE,
  prepareHost,
  prepareSession,
} from '../../src/terminal/workspace.js';
import { CLAUDE, type FakeClaude, fakeClaude } from '../_helpers/fake-claude.js';

type Call = { function_id: string; payload: Record<string, unknown> };

const COMPOSE_FILE = '/home/u/proj/worker-compose.dev.yaml';

/**
 * The engine as the terminal half sees it: the ide worker (`shell::*`) with
 * Claude Code's plugin commands behind `shell::exec`, the http worker, the
 * compose daemon, iii-directory and mcp-server. Everything the worker does
 * goes over the bus, so this is the whole boundary.
 */
function fakeEngine(
  options: {
    claude?: FakeClaude;
    whichClaude?: string;
    noIiiCli?: boolean;
    noMcpServer?: boolean;
    noDirectory?: boolean;
    noCompose?: boolean;
    /** Projects compose::list reports, in order. */
    projects?: { file: string; containers: string[] }[];
    /** Writes that the host accepts but that never land (a full disk, a jail). */
    writesVanish?: boolean;
  } = {},
) {
  const calls: Call[] = [];
  const files: Record<string, string> = {};
  const claude = options.claude ?? fakeClaude();
  const http = { port: 3111 };
  const iii = {
    // biome-ignore lint/suspicious/noExplicitAny: minimal stand-in for the SDK client
    call: async (request: any): Promise<any> => {
      const payload = (request.payload ?? {}) as Record<string, unknown>;
      calls.push({ function_id: request.function_id, payload });
      switch (request.function_id) {
        case 'shell::exec': {
          const command = String(payload.command ?? '');
          if (Array.isArray(payload.args)) {
            return claude.run(command, payload.args as string[], String(payload.cwd ?? ''));
          }
          if (command === 'pwd') return { stdout: '/ide-root\n', stderr: '', exit_code: 0 };
          if (command === 'id -un') return { stdout: 'tony\n', stderr: '', exit_code: 0 };
          if (command === 'command -v claude') {
            const found = options.whichClaude ?? CLAUDE;
            return found
              ? { stdout: `${found}\n`, stderr: '', exit_code: 0 }
              : { stdout: '', stderr: '', exit_code: 1 };
          }
          if (command === 'command -v iii') {
            return options.noIiiCli
              ? { stdout: '', stderr: '', exit_code: 1 }
              : { stdout: '/usr/local/bin/iii\n', stderr: '', exit_code: 0 };
          }
          const test = command.match(/^test -f '([^']*)'$/);
          if (test)
            return { stdout: '', stderr: '', exit_code: files[test[1]] === undefined ? 1 : 0 };
          return { stdout: '', stderr: '', exit_code: 0 };
        }
        case 'shell::fs::mkdir':
          return { created: true };
        case 'shell::fs::write':
          if (!options.writesVanish) files[String(payload.path)] = String(payload.content);
          return { bytes_written: String(payload.content).length };
        case 'http::status':
          return { host: '127.0.0.1', port: http.port, url: `http://127.0.0.1:${http.port}` };
        case 'engine::functions::list':
          return {
            functions: options.noMcpServer ? [] : [{ function_id: 'mcp-server::handler' }],
          };
        case 'directory::agents::get':
          if (options.noDirectory) throw new Error('function_not_found');
          return { id: payload.id, system_prompt: 'The default-mcp prompt.' };
        case 'compose::list':
          if (options.noCompose) throw new Error('function_not_found: compose::list');
          return {
            projects: (
              options.projects ?? [{ file: COMPOSE_FILE, containers: ['ide', 'claude-code'] }]
            ).map((p) => ({
              file: p.file,
              namespace: 'default',
              containers: p.containers.map((container) => ({ container, state: 'ready' })),
            })),
          };
        default:
          throw new Error(`function_not_found: ${request.function_id}`);
      }
    },
  } as unknown as IIIClient;
  return { iii, calls, files, claude, http };
}

const config = (over: Record<string, unknown> = {}) => TerminalConfigSchema.parse(over);

async function host(engine: ReturnType<typeof fakeEngine>, over: Record<string, unknown> = {}) {
  return prepareHost(engine.iii, config(over));
}

const flag = (args: string[], name: string) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};

describe('config', () => {
  it('drops keys an older schema wrote and keeps the defaults', () => {
    const parsed = TerminalConfigSchema.parse({ args: ['--verbose'], removed_key: true });
    expect(parsed.args).toEqual(['--verbose']);
    expect(parsed.setup_workspace).toBe(true);
    expect('removed_key' in parsed).toBe(false);
  });
});

describe('preparing the terminal host (worker start, config change)', () => {
  it('finds the CLI, the iii CLI and the user, and keeps its own folder for its files', async () => {
    const engine = fakeEngine();
    const h = await host(engine);
    expect(h.executable).toBe(CLAUDE);
    expect(h.bridge).toBe('/usr/local/bin/iii');
    expect(h.home).toBe('/ide-root/claude-code');
    expect(h.env).toMatchObject({ USER: 'tony', LOGNAME: 'tony', COLORFGBG: '15;0' });
    expect(h.detail).toBe('');
  });

  it('says when claude is missing and auto_install is off', async () => {
    const engine = fakeEngine({ whichClaude: '' });
    const h = await host(engine, { auto_install: false });
    expect(h.executable).toBe('');
    expect(h.detail).toContain('auto_install is off');
  });

  it('says out loud when the hooks have no iii CLI to reach the bus with', async () => {
    const h = await host(fakeEngine({ noIiiCli: true }));
    expect(h.bridge).toBe('');
    expect(h.detail).toContain('iii` CLI is not on the terminal host');
  });

  it('writes nothing at all: no plugin copy, no hooks', async () => {
    const engine = fakeEngine();
    await host(engine);
    expect(Object.keys(engine.files)).toEqual([]);
  });
});

describe('composeProjectDir', () => {
  it('is the folder holding the compose file of the project that runs this worker', async () => {
    const engine = fakeEngine({
      projects: [
        { file: '/other/worker-compose.yaml', containers: ['state'] },
        { file: COMPOSE_FILE, containers: ['claude-code'] },
      ],
    });
    expect(await composeProjectDir(engine.iii)).toBe('/home/u/proj');
  });

  it('falls back to the only project when none names claude-code', async () => {
    const engine = fakeEngine({
      projects: [{ file: '/solo/worker-compose.yaml', containers: [] }],
    });
    expect(await composeProjectDir(engine.iii)).toBe('/solo');
  });

  it('is empty when there is no compose daemon', async () => {
    expect(await composeProjectDir(fakeEngine({ noCompose: true }).iii)).toBe('');
  });
});

describe('preparing one session (every terminal start)', () => {
  it('runs in the folder it is given, with the plugin installed there', async () => {
    const engine = fakeEngine();
    const h = await host(engine);
    const spec = await prepareSession(engine.iii, config(), h, { cwd: '/work/app' });
    expect(spec.cwd).toBe('/work/app');
    expect(spec.program).toBe(CLAUDE);
    expect(spec.plugin).toMatchObject({ state: 'installed', folder: '/work/app' });
    expect(engine.claude.installs).toEqual([
      { id: 'iii@iii', scope: 'local', enabled: true, projectPath: '/work/app' },
    ]);
    expect(engine.claude.claudeCalls()).toContain('plugin install iii@iii --scope local --json');
  });

  it("without a folder, runs in the compose project's folder, never the ide worker's", async () => {
    const engine = fakeEngine();
    const h = await host(engine);
    const spec = await prepareSession(engine.iii, config(), h, {});
    expect(spec.cwd).toBe('/home/u/proj');
    expect(spec.project_dir).toBe('/home/u/proj');
    expect(spec.cwd).not.toContain('/ide-root');
  });

  it('refuses to start with no folder at all rather than fall back to the ide worker', async () => {
    const engine = fakeEngine({ noCompose: true });
    const h = await host(engine);
    await expect(prepareSession(engine.iii, config(), h, {})).rejects.toThrow(/folder/);
  });

  it('runs in the real folder: a symlinked path is resolved first', async () => {
    const engine = fakeEngine();
    engine.claude.realpaths['/tmp/app'] = '/private/tmp/app';
    const spec = await prepareSession(engine.iii, config(), await host(engine), {
      cwd: '/tmp/app',
    });
    expect(spec.cwd).toBe('/private/tmp/app');
    expect(engine.claude.installs[0].projectPath).toBe('/private/tmp/app');
  });

  it('refuses a folder that does not exist on the terminal host', async () => {
    const engine = fakeEngine();
    engine.claude.missing.add('/nope');
    await expect(
      prepareSession(engine.iii, config(), await host(engine), { cwd: '/nope' }),
    ).rejects.toThrow(/\/nope/);
  });

  it('leaves a folder whose plugin is already installed and enabled alone', async () => {
    const engine = fakeEngine({
      claude: fakeClaude({
        installs: [{ id: 'iii@iii', scope: 'local', enabled: true, projectPath: '/work/app' }],
      }),
    });
    const spec = await prepareSession(engine.iii, config(), await host(engine), {
      cwd: '/work/app',
    });
    expect(spec.plugin?.state).toBe('present');
    expect(engine.claude.claudeCalls()).toEqual(['plugin list --json']);
  });

  it('passes no --plugin-dir and writes no plugin copy into the folder or anywhere', async () => {
    const engine = fakeEngine();
    const spec = await prepareSession(engine.iii, config(), await host(engine), {
      cwd: '/work/app',
    });
    expect(spec.args).not.toContain('--plugin-dir');
    const written = Object.keys(engine.files);
    expect(written.filter((p) => p.startsWith('/work/app'))).toEqual([]);
    for (const p of written) {
      expect(p).not.toMatch(/hooks\.json|\.mcp\.json|agents\/|commands\/|SKILL\.md|\.iii-plugin/);
    }
  });

  it('points this process at mcp-server with III_MCP_URL, and configures nothing', async () => {
    const engine = fakeEngine();
    const spec = await prepareSession(engine.iii, config(), await host(engine), {
      cwd: '/work/app',
    });
    expect(spec.env.III_MCP_URL).toBe('http://127.0.0.1:3111/mcp');
    expect(spec.args).not.toContain('--settings');
    for (const call of engine.claude.claudeCalls()) expect(call).not.toContain('--config');
  });

  it('gives each session the http address of its start: no worker restart, no config change', async () => {
    // The bug: the workspace was prepared at worker start only, so a moved
    // http worker never reached new terminals.
    const engine = fakeEngine();
    const h = await host(engine);
    const first = await prepareSession(engine.iii, config(), h, { cwd: '/work/a' });
    engine.http.port = 4111;
    const second = await prepareSession(engine.iii, config(), h, { cwd: '/work/b' });
    expect(first.env.III_MCP_URL).toBe('http://127.0.0.1:3111/mcp');
    expect(second.env.III_MCP_URL).toBe('http://127.0.0.1:4111/mcp');
    expect(second.env.III_PLUGIN_MCP_URL).toBe('http://127.0.0.1:4111/mcp');
    // The host was prepared once; only the session read the new address.
    expect(h.env.III_MCP_URL).toBeUndefined();
  });

  it("passes the hooks the iii CLI of the terminal host as III_CLI, and the engine's address", async () => {
    const engine = fakeEngine();
    const spec = await prepareSession(engine.iii, config(), await host(engine), {
      cwd: '/work/app',
    });
    expect(spec.env.III_CLI).toBe('/usr/local/bin/iii');
    expect(spec.env.III_URL).toBeTruthy();
    expect(spec.activity_bridge).toBe('/usr/local/bin/iii');
  });

  it('writes default-mcp to the worker folder before every start and appends it from there', async () => {
    const engine = fakeEngine();
    const h = await host(engine);
    const file = `/ide-root/claude-code/${MODE_FILE}`;
    const spec = await prepareSession(engine.iii, config(), h, { cwd: '/work/app' });
    expect(flag(spec.args, '--append-system-prompt-file')).toBe(file);
    expect(engine.files[file]).toContain('The default-mcp prompt.');
    // Gone between two starts (a cleaned folder): the next start writes it again.
    delete engine.files[file];
    const again = await prepareSession(engine.iii, config(), h, { cwd: '/work/app' });
    expect(flag(again.args, '--append-system-prompt-file')).toBe(file);
    expect(engine.files[file]).toContain('The default-mcp prompt.');
  });

  it('never launches with a path to a default-mcp file that is not there', async () => {
    const engine = fakeEngine({ writesVanish: true });
    const spec = await prepareSession(engine.iii, config(), await host(engine), {
      cwd: '/work/app',
    });
    expect(spec.args).not.toContain('--append-system-prompt-file');
    expect(spec.detail).toContain('default-mcp');
  });

  it('appends no default-mcp when it cannot be read, and says so', async () => {
    const engine = fakeEngine({ noDirectory: true });
    const spec = await prepareSession(engine.iii, config(), await host(engine), {
      cwd: '/work/app',
    });
    expect(spec.args).not.toContain('--append-system-prompt-file');
    expect(spec.detail).toContain('default-mcp');
  });

  it('adds no default-mcp over an append the operator configured', async () => {
    const engine = fakeEngine();
    const c = config({ args: ['--append-system-prompt', 'house rules'] });
    const spec = await prepareSession(engine.iii, c, await prepareHost(engine.iii, c), {
      cwd: '/work/app',
    });
    expect(spec.args).not.toContain('--append-system-prompt-file');
    expect(spec.args.slice(0, 2)).toEqual(['--append-system-prompt', 'house rules']);
    expect(spec.detail).toContain('--append-system-prompt');
  });

  it('keeps the operator args first and untouched', async () => {
    const engine = fakeEngine();
    const c = config({ args: ['--settings', '/etc/claude.json', '--verbose'] });
    const spec = await prepareSession(engine.iii, c, await prepareHost(engine.iii, c), {
      cwd: '/work/app',
    });
    expect(spec.args.slice(0, 3)).toEqual(['--settings', '/etc/claude.json', '--verbose']);
    expect(spec.args.filter((a) => a === '--settings')).toHaveLength(1);
  });

  it("never starts the session as an agent: that would replace Claude Code's prompt", async () => {
    const engine = fakeEngine();
    const spec = await prepareSession(engine.iii, config(), await host(engine), { cwd: '/w' });
    expect(spec.args).not.toContain('--agent');
  });

  it('without mcp-server, still installs the plugin but appends nothing', async () => {
    const engine = fakeEngine({ noMcpServer: true });
    const spec = await prepareSession(engine.iii, config(), await host(engine), { cwd: '/w' });
    expect(spec.plugin?.state).toBe('installed');
    expect(spec.args).not.toContain('--append-system-prompt-file');
    expect(spec.detail).toContain('mcp-server is not running');
  });

  it('opens the terminal anyway when the install fails, saying why', async () => {
    const engine = fakeEngine();
    engine.claude.fail.add('plugin install');
    const spec = await prepareSession(engine.iii, config(), await host(engine), { cwd: '/w' });
    expect(spec.plugin?.state).toBe('failed');
    expect(spec.detail).toContain('plugin install failed');
    expect(spec.program).toBe(CLAUDE);
  });

  it('touches no plugin and appends nothing when setup_workspace is off', async () => {
    const engine = fakeEngine();
    const c = config({ setup_workspace: false });
    const spec = await prepareSession(engine.iii, c, await prepareHost(engine.iii, c), {
      cwd: '/w',
    });
    expect(spec.plugin).toBeNull();
    expect(engine.claude.claudeCalls()).toEqual([]);
    expect(spec.args).toEqual([]);
    expect(Object.keys(engine.files)).toEqual([]);
  });

  it('refuses to start without a claude binary, with the reason', async () => {
    const engine = fakeEngine({ whichClaude: '' });
    const c = config({ auto_install: false });
    const h: HostPrepared = await prepareHost(engine.iii, c);
    await expect(prepareSession(engine.iii, c, h, { cwd: '/w' })).rejects.toThrow(/auto_install/);
  });
});
