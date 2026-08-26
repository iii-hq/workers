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

    // Every hook posts the payload to this worker over the bus, exactly once
    // expanded — a prompt containing shell syntax stays data.
    const settings = JSON.parse(files['/hostroot/claude-code/.claude/settings.json']);
    expect(Object.keys(settings.hooks).sort()).toEqual([
      'PostToolUse',
      'PreToolUse',
      'SessionEnd',
      'SessionStart',
      'Stop',
      'UserPromptSubmit',
    ]);
    const command = settings.hooks.PreToolUse[0].hooks[0].command;
    // The CLI path is quoted: a `command -v` answer can carry a space.
    expect(command).toContain(`'/usr/bin/iii' trigger claude::terminal::activity --json "$(cat)"`);
    expect(settings.hooks.PreToolUse[0].matcher).toBe('*');
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

  it('keeps the operator half of settings.json and their own notes', async () => {
    const { iii, files } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      files: {
        '/hostroot/claude-code/.claude/settings.json': JSON.stringify({
          model: 'opus',
          hooks: { Notification: [{ hooks: [{ type: 'command', command: 'say hi' }] }] },
        }),
        '/hostroot/claude-code/CLAUDE.md': '# My own notes\n\nkeep me\n',
      },
    });
    await prepareWorkspace(iii, { ...DEFAULTS });

    const settings = JSON.parse(files['/hostroot/claude-code/.claude/settings.json']);
    expect(settings.model).toBe('opus');
    expect(settings.hooks.Notification[0].hooks[0].command).toBe('say hi');
    expect(settings.hooks.PreToolUse).toBeDefined();

    const notes = files['/hostroot/claude-code/CLAUDE.md'];
    expect(notes).toContain('# My own notes');
    expect(notes).toContain('keep me');
    expect(notes.indexOf('<!-- iii:begin')).toBeLessThan(notes.indexOf('# My own notes'));
  });

  it('keeps an operator hook on an event it also writes', async () => {
    // A formatter on PostToolUse is the operator's, and it is registered on an
    // event this worker also hooks. Replacing the event's array deleted it on
    // the next boot; the worker's own entry is the only one it may rewrite.
    const { iii, files } = fakeShell({
      whichClaude: '/usr/local/bin/claude',
      files: {
        '/hostroot/claude-code/.claude/settings.json': JSON.stringify({
          hooks: {
            PostToolUse: [
              { matcher: 'Edit', hooks: [{ type: 'command', command: 'biome check' }] },
            ],
            SessionStart: [
              {
                hooks: [{ type: 'command', command: 'old-iii trigger claude::terminal::activity' }],
              },
            ],
          },
        }),
      },
    });
    await prepareWorkspace(iii, { ...DEFAULTS });

    const settings = JSON.parse(files['/hostroot/claude-code/.claude/settings.json']);
    const post = settings.hooks.PostToolUse;
    expect(post).toHaveLength(2);
    expect(post[0].hooks[0].command).toBe('biome check');
    expect(post[1].hooks[0].command).toContain('claude::terminal::activity');
    // Its own entry from an earlier boot is replaced, not kept beside the new
    // one — the `iii` path it was baked with can move.
    expect(settings.hooks.SessionStart).toHaveLength(1);
    expect(settings.hooks.SessionStart[0].hooks[0].command).toContain("'/usr/bin/iii'");
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
    expect(files['/hostroot/claude-code/.claude/settings.json']).toContain(
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
