import type { IIIClient } from 'iii-sdk';
import { describe, expect, it } from 'vitest';
import { TerminalConfigSchema } from '../../src/config.js';
import { prepareWorkspace } from '../../src/terminal/workspace.js';

type Call = { function_id: string; payload: Record<string, unknown> };

/**
 * A shell worker that answers from a fake filesystem. Everything
 * `prepareWorkspace` does goes over the bus, so this is the whole boundary.
 */
function fakeShell(
  options: {
    files?: Record<string, string>;
    whichClaude?: string;
    noIiiCli?: boolean;
    /** No `iii-directory` on the bus: the context cannot be fetched. */
    noDirectory?: boolean;
    /** A read failure that is NOT "missing": a timeout, a read budget. */
    readError?: Error;
  } = {},
) {
  const calls: Call[] = [];
  const files: Record<string, string> = { ...options.files };
  const iii = {
    // biome-ignore lint/suspicious/noExplicitAny: minimal stand-in for the SDK client
    trigger: async (request: any): Promise<any> => {
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
        case 'shell::fs::write':
          files[String(payload.path)] = String(payload.content);
          return { bytes_written: String(payload.content).length };
        case 'coder::read-file': {
          if (options.readError) throw options.readError;
          const content = files[String(payload.path)];
          // The shape the coder surface answers with: C211 is missing-or-denied,
          // and it is the only failure that means "there is nothing to preserve".
          if (content === undefined) throw new Error('error[C211]: no such file');
          return { content };
        }
        default:
          return {};
      }
    },
  } as unknown as IIIClient;
  return { iii, calls, files };
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

    // The notes live in one marked block Claude reads on startup.
    const notes = files['/hostroot/claude-code/CLAUDE.md'];
    expect(notes).toContain('<!-- iii:begin');
    expect(notes).toContain('<!-- iii:end -->');
    expect(notes).toContain('/hostroot/claude-code');

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
      'UserPromptSubmit',
    ]);
    const command = hooks.PreToolUse[0].hooks[0].command;
    // The CLI path is quoted: a `command -v` answer can carry a space, and the
    // payload expands exactly once so a prompt full of shell syntax stays data.
    expect(command).toContain(`'/usr/bin/iii' trigger claude::terminal::activity --json "$(cat)"`);
    expect(hooks.PreToolUse[0].matcher).toBe('*');

    // The iii runtime text arrives as a SKILL, from iii-directory.
    expect(files['/hostroot/claude-code/.iii-plugin/skills/iii-runtime/SKILL.md']).toContain(
      'name: iii-runtime',
    );

    // And the session is told to load it.
    expect(prepared.args).toEqual([
      ...DEFAULTS.args,
      '--plugin-dir',
      '/hostroot/claude-code/.iii-plugin',
    ]);
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

    const notes = files['/hostroot/claude-code/CLAUDE.md'];
    expect(notes).toContain('# My own notes');
    expect(notes).toContain('keep me');
    expect(notes.indexOf('<!-- iii:begin')).toBeLessThan(notes.indexOf('# My own notes'));
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

  it('uses the configured workspace as given', async () => {
    const { iii } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, {
      ...DEFAULTS,
      workspace_dir: '/srv/agents/claude',
    });
    expect(prepared.workspace).toBe('/srv/agents/claude');
  });
});
