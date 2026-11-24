import type { IIIClient } from 'iii-sdk';
import { describe, expect, it } from 'vitest';
import { TerminalConfigSchema } from '../../src/config.js';
import { prepareWorkspace } from '../../src/terminal/workspace.js';

type Call = { function_id: string; payload: Record<string, unknown> };

/**
 * An ide worker (its `shell::*` functions) answering from a fake filesystem. Everything
 * `prepareWorkspace` does goes over the bus, so this is the whole boundary.
 */
function fakeShell(
  options: {
    files?: Record<string, string>;
    whichClaude?: string;
    noIiiCli?: boolean;
    /** No `iii-directory` on the bus: the context cannot be fetched. */
    noDirectory?: boolean;
    /** No mcp-server on the engine. */
    noMcpServer?: boolean;
    /** A read failure that is NOT "missing": a timeout, a read budget. */
    readError?: Error;
    /** The host cannot list a folder or test a file (a shell without find). */
    listingFails?: boolean;
  } = {},
) {
  const calls: Call[] = [];
  /** Reads of a file that is not there: each is a failed invocation on the bus. */
  const missedReads: string[] = [];
  const files: Record<string, string> = { ...options.files };
  const iii = {
    // biome-ignore lint/suspicious/noExplicitAny: minimal stand-in for the SDK client
    call: async (request: any): Promise<any> => {
      const payload = (request.payload ?? {}) as Record<string, unknown>;
      calls.push({ function_id: request.function_id, payload });
      switch (request.function_id) {
        case 'shell::exec': {
          const command = String(payload.command ?? '');
          if (command === 'pwd') return { stdout: '/hostroot\n', stderr: '', exit_code: 0 };
          if (command === 'id -un') return { stdout: 'tony\n', stderr: '', exit_code: 0 };
          if (command === 'command -v claude') {
            return options.whichClaude
              ? { stdout: `${options.whichClaude}\n`, stderr: '', exit_code: 0 }
              : { stdout: '', stderr: '', exit_code: 1 };
          }
          if (command === 'command -v iii') {
            return options.noIiiCli
              ? { stdout: '', stderr: '', exit_code: 1 }
              : { stdout: '/usr/bin/iii\n', stderr: '', exit_code: 0 };
          }
          // One listing of the plugin folder, so a missing file is never read.
          const listing = command.match(
            /^if \[ -d '([^']*)' \]; then find -L '[^']*' -type f; fi$/,
          );
          if (listing) {
            if (options.listingFails)
              return { stdout: '', stderr: 'find: not found', exit_code: 127 };
            const under = Object.keys(files).filter((f) => f.startsWith(`${listing[1]}/`));
            return { stdout: under.map((f) => `${f}\n`).join(''), stderr: '', exit_code: 0 };
          }
          const test = command.match(/^test -f '([^']*)'$/);
          if (test) {
            if (options.listingFails)
              return { stdout: '', stderr: 'test: not found', exit_code: 127 };
            return { stdout: '', stderr: '', exit_code: files[test[1]] === undefined ? 1 : 0 };
          }
          return { stdout: '', stderr: '', exit_code: 0 };
        }
        // The iii context lives in `iii-directory`; the worker fetches it
        // rather than carrying a copy, so the fake serves it.
        case 'directory::system-prompts::get':
          return options.noDirectory
            ? Promise.reject(new Error('function_not_found'))
            : { name: payload.name, body: '# iii runtime\n\nAsk the engine.' };
        case 'directory::skills::index':
          return options.noDirectory
            ? Promise.reject(new Error('function_not_found'))
            : { body: '# Skills index\n\n## shell', workers_count: 1 };
        // Profiles become /iii:<profile> commands; hidden ones do not.
        case 'directory::agents::get':
          return options.noDirectory
            ? Promise.reject(new Error('function_not_found'))
            : { id: payload.id, system_prompt: 'The default-mcp prompt.' };
        case 'http::status':
          return { host: '127.0.0.1', port: 3111, url: 'http://127.0.0.1:3111' };
        case 'engine::functions::list':
          return {
            functions: options.noMcpServer ? [] : [{ function_id: 'mcp-server::handler' }],
          };
        case 'directory::agents::list':
          return options.noDirectory
            ? Promise.reject(new Error('function_not_found'))
            : {
                agents: [
                  { id: 'writer', description: 'Writes things.' },
                  { id: 'helper', hidden: true },
                ],
              };
        case 'shell::fs::rm':
          delete files[String(payload.path)];
          return { removed: true, path: payload.path };
        case 'shell::fs::write':
          files[String(payload.path)] = String(payload.content);
          return { bytes_written: String(payload.content).length };
        case 'coder::read-file': {
          if (options.readError) throw options.readError;
          const content = files[String(payload.path)];
          // The shape the coder surface answers with: C211 is missing-or-denied,
          // and it is the only failure that means "there is nothing to preserve".
          if (content === undefined) {
            missedReads.push(String(payload.path));
            throw new Error('error[C211]: no such file');
          }
          return { content };
        }
        default:
          return {};
      }
    },
  } as unknown as IIIClient;
  return { iii, calls, files, missedReads };
}

/** A stored value an older schema wrote must not reach a session. */
const DEFAULTS = TerminalConfigSchema.parse({});

describe('config', () => {
  it('drops keys an older schema wrote and keeps the defaults', () => {
    expect(TerminalConfigSchema.parse({ args: ['-p'], nonsense: 1 })).toEqual({
      ...DEFAULTS,
      args: ['-p'],
    });
    expect(TerminalConfigSchema.parse({})).toEqual(DEFAULTS);
  });
});

describe('preparing the terminal host', () => {
  it('equips a fresh workspace and reports the binary', async () => {
    const { iii, calls, files } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared).toMatchObject({
      workspace: '/hostroot/claude-code',
      executable: '/usr/local/bin/claude',
      detail: '',
    });

    // The session carries who the terminal host is: Claude Code reads its
    // keychain login under the current user, and a worker environment does
    // not carry one — without this the CLI reports itself signed out.
    expect(prepared.env.USER).toBe('tony');
    expect(prepared.env.LOGNAME).toBe('tony');
    // A dark page wants a light-on-dark palette from whatever runs in it.
    expect(prepared.env.COLORFGBG).toBe('15;0');

    // No CLAUDE.md: what it said is the plugin's now (the iii-runtime skill,
    // default-mcp appended), and the workspace and engine are in the session.
    expect(files['/hostroot/claude-code/CLAUDE.md']).toBeUndefined();
    expect(prepared.env.III_URL).toBeTruthy();

    // The hooks and the iii skill ride in ONE plugin directory, which is what
    // a session loads — the same directory a headless turn is given.
    const manifest = JSON.parse(
      files['/hostroot/claude-code/.iii-plugin/.claude-plugin/plugin.json'],
    );
    expect(manifest.name).toBe('iii');
    const hooks = JSON.parse(files['/hostroot/claude-code/.iii-plugin/hooks/hooks.json']).hooks;
    expect(Object.keys(hooks).sort()).toEqual([
      'PostToolUse',
      'PreToolUse',
      'SessionEnd',
      'SessionStart',
      'Stop',
      'StopFailure',
      'UserPromptSubmit',
    ]);
    const command = hooks.PreToolUse[0].hooks[0].command;
    // The CLI path is quoted: a `command -v` answer can carry a space, and the
    // payload expands exactly once so a prompt full of shell syntax stays data.
    expect(command).toContain(`'/usr/bin/iii' call claude::terminal::activity --json "$(cat)"`);
    expect(hooks.PreToolUse[0].matcher).toBe('*');

    // With mcp-server, default-mcp guides the session: the CLI-only runtime
    // skill would contradict it.
    expect(files['/hostroot/claude-code/.iii-plugin/skills/iii-runtime/SKILL.md']).toBeUndefined();

    // The same directory is the whole `iii` plugin: the committed CLI and
    // static commands ship inside the worker, the session's MCP server is the
    // engine's mcp-server, and each visible profile gets its own
    // /iii:<profile> command.
    const dir = '/hostroot/claude-code/.iii-plugin';
    expect(files[`${dir}/cli/index.mjs`]).toBeTruthy();
    expect(files[`${dir}/server/index.mjs`]).toBeUndefined();
    expect(JSON.parse(files[`${dir}/.mcp.json`])).toEqual({
      mcpServers: { iii: { type: 'http', url: 'http://127.0.0.1:3111/mcp' } },
    });
    expect(files[`${dir}/.build-id`]).toBeTruthy();
    expect(files[`${dir}/commands/profile.md`]).toContain('<profile> [task]');
    expect(files[`${dir}/commands/skills.md`]).toContain('<skill> [task]');
    expect(files[`${dir}/commands/writer.md`]).toContain('`writer`');
    expect(files[`${dir}/commands/helper.md`]).toBeUndefined();
    // The worker generates the commands itself, so the standalone SessionStart
    // `sync` hook is not installed beside the activity hooks.
    expect(files[`${dir}/hooks/hooks.json`]).not.toContain(' sync ');

    // And the session loads it, with the iii mode rules appended to Claude
    // Code's own system prompt (an agent would replace it).
    expect(prepared.args).toEqual([
      ...DEFAULTS.args,
      '--plugin-dir',
      dir,
      '--append-system-prompt-file',
      `${dir}/default-mcp.md`,
    ]);
    expect(files[`${dir}/default-mcp.md`]).toContain('default-mcp prompt');
    expect(files[`${dir}/agents/iii.md`]).toContain('default-mcp prompt');
    expect(prepared.plugin).toBe('/hostroot/claude-code/.iii-plugin');

    // `.claude/settings.json` is the operator's file and is never written.
    expect(files['/hostroot/claude-code/.claude/settings.json']).toBeUndefined();
  });

  it('installs the CLI when it is missing, and says so when it cannot', async () => {
    const installing = fakeShell();
    const prepared = await prepareWorkspace(installing.iii, { ...DEFAULTS });
    expect(
      installing.calls.some((c) =>
        String(c.payload.command ?? '').includes('claude.ai/install.sh'),
      ),
    ).toBe(true);
    expect(prepared.executable).toBe('');
    expect(prepared.detail).toContain('could not be installed');

    const refusing = fakeShell();
    const second = await prepareWorkspace(refusing.iii, { ...DEFAULTS, auto_install: false });
    expect(
      refusing.calls.some((c) => String(c.payload.command ?? '').includes('claude.ai/install.sh')),
    ).toBe(false);
    expect(second.detail).toContain('auto_install is off');
  });

  it("leaves the operator's settings.json alone and keeps their notes", async () => {
    // The hooks used to be merged into this file on every boot, which meant
    // reasoning about whatever else lived in it. They are the plugin's now, so
    // the file is simply not this worker's business.
    const { iii, files } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      files: {
        '/hostroot/claude-code/.claude/settings.json': JSON.stringify({
          model: 'opus',
          hooks: {
            PostToolUse: [
              { matcher: 'Edit', hooks: [{ type: 'command', command: 'biome check' }] },
            ],
          },
        }),
        '/hostroot/claude-code/CLAUDE.md': '# My own notes\n\nkeep me\n',
      },
    });
    await prepareWorkspace(iii, { ...DEFAULTS });

    const settings = JSON.parse(files['/hostroot/claude-code/.claude/settings.json']);
    expect(settings).toEqual({
      model: 'opus',
      hooks: {
        PostToolUse: [{ matcher: 'Edit', hooks: [{ type: 'command', command: 'biome check' }] }],
      },
    });

    expect(files['/hostroot/claude-code/CLAUDE.md']).toBe('# My own notes\n\nkeep me\n');
  });

  it('reads no file that is not there, so a fresh workspace logs no failed calls', async () => {
    // A miss answers C211, which the installer reads as "absent", but the
    // engine still records each one as a failed invocation: one per plugin
    // file on every fresh workspace. The installer lists the folder once and
    // reads only what it found.
    const { iii, calls, files, missedReads } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared.detail).toBe('');
    expect(missedReads).toEqual([]);
    const dir = '/hostroot/claude-code/.iii-plugin';
    expect(files[`${dir}/hooks/hooks.json`]).toBeTruthy();
    expect(files['/hostroot/claude-code/CLAUDE.md']).toBeUndefined();
    const listings = calls.filter(
      (c) => c.function_id === 'shell::exec' && String(c.payload.command).includes('find'),
    );
    expect(listings).toHaveLength(1);
  });

  it('on a second start, reads only what is there and rewrites nothing unchanged', async () => {
    const { iii, calls, files, missedReads } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    await prepareWorkspace(iii, { ...DEFAULTS });
    const before = { ...files };
    calls.length = 0;
    missedReads.length = 0;

    await prepareWorkspace(iii, { ...DEFAULTS });

    expect(missedReads).toEqual([]);
    expect(files).toEqual(before);
    expect(calls.filter((c) => c.function_id === 'shell::fs::write')).toEqual([]);
    // The existing files are still read, so a hand-edited one is put back.
    expect(calls.some((c) => c.function_id === 'coder::read-file')).toBe(true);
  });

  it('puts back a plugin file someone changed', async () => {
    const { iii, files } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    await prepareWorkspace(iii, { ...DEFAULTS });
    // A generated file: the static ones are rewritten when the build changes.
    const path = '/hostroot/claude-code/.iii-plugin/default-mcp.md';
    const original = files[path];
    files[path] = 'edited';

    await prepareWorkspace(iii, { ...DEFAULTS });

    expect(files[path]).toBe(original);
  });

  it('falls back to reading each file when the host cannot list the folder', async () => {
    // No listing says nothing about the files, so every one is read as before:
    // a miss still answers C211, and nothing the operator wrote is lost.
    const { iii, files, missedReads } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      listingFails: true,
      files: { '/hostroot/claude-code/CLAUDE.md': '# My own notes\n' },
    });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared.detail).toBe('');
    expect(missedReads.length).toBeGreaterThan(0);
    expect(files['/hostroot/claude-code/.iii-plugin/.claude-plugin/plugin.json']).toBeTruthy();
    expect(files['/hostroot/claude-code/CLAUDE.md']).toBe('# My own notes\n');
  });

  it('quotes a workspace path with spaces and quotes in the listing', async () => {
    const { iii, calls } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    await prepareWorkspace(iii, { ...DEFAULTS, workspace_dir: "/home/o'neil/my space" });

    const listing = calls.find(
      (c) => c.function_id === 'shell::exec' && String(c.payload.command).includes('find'),
    );
    const quoted = `'/home/o'\\''neil/my space/.iii-plugin'`;
    expect(String(listing?.payload.command)).toBe(
      // -L: a symlinked plugin folder is listed through the link, not as empty.
      `if [ -d ${quoted} ]; then find -L ${quoted} -type f; fi`,
    );
  });

  // The block earlier versions wrote, as they wrote it: first, then the
  // operator's text after a blank line.
  const LEGACY =
    '<!-- iii:begin — written by the claude-code worker on every boot; edits inside are lost -->\n' +
    '# You run inside an iii engine\n\nthe `shell` worker owns the session.\n<!-- iii:end -->';

  it('takes the old block out of CLAUDE.md and keeps the operator text', async () => {
    const { iii, files } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      files: { '/hostroot/claude-code/CLAUDE.md': `${LEGACY}\n\n# My own notes\n\nkeep me\n` },
    });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared.detail).toBe('');
    expect(files['/hostroot/claude-code/CLAUDE.md']).toBe('# My own notes\n\nkeep me\n');
  });

  it('keeps operator text on both sides of the old block', async () => {
    const { iii, files } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      files: { '/hostroot/claude-code/CLAUDE.md': `# Top\n\n${LEGACY}\n\n# Bottom\n` },
    });
    await prepareWorkspace(iii, { ...DEFAULTS });

    expect(files['/hostroot/claude-code/CLAUDE.md']).toBe('# Top\n\n# Bottom\n');
  });

  it('removes a CLAUDE.md that holds nothing but the old block', async () => {
    const { iii, files, calls } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      files: { '/hostroot/claude-code/CLAUDE.md': `${LEGACY}\n` },
    });
    await prepareWorkspace(iii, { ...DEFAULTS });

    expect(files['/hostroot/claude-code/CLAUDE.md']).toBeUndefined();
    expect(
      calls.some(
        (c) =>
          c.function_id === 'shell::fs::rm' && c.payload.path === '/hostroot/claude-code/CLAUDE.md',
      ),
    ).toBe(true);
  });

  it('writes nothing to a CLAUDE.md without the old block', async () => {
    const { iii, calls } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      // An unterminated marker is not the block: the file is left as it is.
      files: { '/hostroot/claude-code/CLAUDE.md': '# Notes\n<!-- iii:begin but no end\n' },
    });
    await prepareWorkspace(iii, { ...DEFAULTS });

    const touched = calls.filter(
      (c) =>
        ['shell::fs::write', 'shell::fs::rm'].includes(c.function_id) &&
        c.payload.path === '/hostroot/claude-code/CLAUDE.md',
    );
    expect(touched).toEqual([]);
  });

  it('keeps CLAUDE.md and the terminal when a read fails for another reason', async () => {
    // A read that TIMED OUT says nothing about the file. Answering that as
    // "absent" is how a worker overwrites what a person wrote, so the notes are
    // left alone — and the terminal still opens, because a workspace that could
    // not be equipped is still a workspace Claude runs in.
    const { iii, files } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      files: { '/hostroot/claude-code/CLAUDE.md': '# My own notes\n' },
      readError: new Error('error[S303]: read timed out'),
    });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared.executable).toBe('/usr/local/bin/claude');
    expect(prepared.detail).toContain('could not be equipped');
    expect(files['/hostroot/claude-code/CLAUDE.md']).toBe('# My own notes\n');
  });

  it('leaves the workspace alone when setup is off', async () => {
    const { iii, calls } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    await prepareWorkspace(iii, { ...DEFAULTS, setup_workspace: false });
    expect(calls.some((c) => c.function_id === 'shell::fs::write')).toBe(false);
  });

  it('says out loud when the hooks have no way to reach the bus', async () => {
    // The terminal host is not necessarily this worker's host, and it will not
    // be one at all if the worker that owns the terminal is ever virtualized:
    // then `iii` may be missing there and every hook is a silent no-op.
    const { iii, files } = fakeShell({ whichClaude: '/usr/local/bin/claude', noIiiCli: true });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared.bridge).toBe('');
    expect(prepared.detail).toContain('cannot reach the bus');
    // Written anyway: a CLI installed later starts working with no rewrite.
    expect(files['/hostroot/claude-code/.iii-plugin/hooks/hooks.json']).toContain(
      'claude::terminal::activity',
    );
  });

  it('reports the bridge it found when there is one', async () => {
    const { iii } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });
    expect(prepared.bridge).toBe('/usr/bin/iii');
    expect(prepared.detail).toBe('');
  });

  it("never starts the session as an agent: that would replace Claude Code's prompt", async () => {
    const { iii } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });
    expect(prepared.args).not.toContain('--agent');
    expect(prepared.args).not.toContain('iii:iii');
  });

  it('keeps an --agent the operator configured', async () => {
    const { iii } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS, args: ['--agent=mine'] });
    expect(prepared.args).toContain('--agent=mine');
  });

  it('adds no default-mcp over an append the operator configured', async () => {
    for (const own of [
      ['--append-system-prompt', 'house rules'],
      ['--append-system-prompt-file=x.md'],
    ]) {
      const { iii } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
      const prepared = await prepareWorkspace(iii, { ...DEFAULTS, args: own });
      expect(prepared.args.filter((a) => a.startsWith('--append-system-prompt'))).toHaveLength(1);
      expect(prepared.detail).toContain('default-mcp was not appended');
    }
  });

  it('starts no iii mode when the workspace is not set up (no plugin)', async () => {
    const { iii } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS, setup_workspace: false });
    expect(prepared.args.some((a) => a.startsWith('--append-system-prompt'))).toBe(false);
  });

  it('without mcp-server, the session works through the iii CLI as before', async () => {
    const { iii, files } = fakeShell({ whichClaude: '/usr/local/bin/claude', noMcpServer: true });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });
    const dir = '/hostroot/claude-code/.iii-plugin';
    expect(prepared.plugin).toBe(dir);
    expect(prepared.args.some((a) => a.startsWith('--append-system-prompt'))).toBe(false);
    expect(files[`${dir}/.mcp.json`]).toBeUndefined();
    expect(files[`${dir}/hooks/hooks.json`]).not.toContain('session-start');
    expect(prepared.detail).toContain('mcp-server is not running on this engine');
    // The CLI runtime skill is then the session's guide.
    expect(files[`${dir}/skills/iii-runtime/SKILL.md`]).toContain('name: iii-runtime');
  });

  it('uses the configured workspace as given', async () => {
    const { iii } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, {
      ...DEFAULTS,
      workspace_dir: '/srv/agents/claude',
    });
    expect(prepared.workspace).toBe('/srv/agents/claude');
  });
});
