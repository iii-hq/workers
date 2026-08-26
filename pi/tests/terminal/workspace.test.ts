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
    whichPi?: string;
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
          if (command === 'command -v pi') {
            return options.whichPi
              ? { stdout: `${options.whichPi}\n`, stderr: '', exit_code: 0 }
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
  it('trusts the workspace by default, and drops keys an older schema wrote', () => {
    // Without `-a` pi asks about project trust every session and never loads
    // the extension that reports what it did.
    // `-a` trusts the workspace; the theme matches the dark terminal the
    // console page paints.
    expect(DEFAULTS.args).toEqual(['-a', '--use-theme', 'dark']);
    expect(TerminalConfigSchema.parse({ args: [], nonsense: 1 })).toEqual({
      ...DEFAULTS,
      args: [],
    });
    expect(TerminalConfigSchema.parse({})).toEqual(DEFAULTS);
  });
});

describe('preparing the terminal host', () => {
  it('equips a fresh workspace and installs the activity extension', async () => {
    const { iii, calls, files } = fakeShell({ whichPi: '/usr/local/bin/pi' });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared).toMatchObject({
      workspace: '/hostroot/pi',
      executable: '/usr/local/bin/pi',
      args: ['-a', '--use-theme', 'dark'],
      detail: '',
    });

    // The session carries who the terminal host is: pi finds a stored login
    // under the current user, and a worker environment does not carry one.
    expect(prepared.env.USER).toBe('tony');
    expect(prepared.env.COLORFGBG).toBe('15;0');

    // pi reads AGENTS.md, and the worker owns one marked block in it.
    const notes = files['/hostroot/pi/AGENTS.md'];
    expect(notes).toContain('<!-- iii:begin');
    expect(notes).toContain('/hostroot/pi');

    // The extension is discovered from the workspace, so a session started by
    // hand in the same directory reports its turns too.
    const extension = files['/hostroot/pi/.pi/extensions/iii-activity.ts'];
    expect(extension).toContain('pi::terminal::activity');
    expect(extension).toContain('"/usr/bin/iii"');
    expect(extension).toContain('tool_execution_end');
  });

  it('installs pi when it is missing, and says so when it cannot', async () => {
    const installing = fakeShell();
    const prepared = await prepareWorkspace(installing.iii, { ...DEFAULTS });
    expect(
      installing.calls.some((c) => String(c.payload.command ?? '').includes('pi.dev/install.sh')),
    ).toBe(true);
    expect(prepared.detail).toContain('could not be installed');

    const refusing = fakeShell();
    const second = await prepareWorkspace(refusing.iii, { ...DEFAULTS, auto_install: false });
    expect(
      refusing.calls.some((c) => String(c.payload.command ?? '').includes('pi.dev/install.sh')),
    ).toBe(false);
    expect(second.detail).toContain('auto_install is off');
  });

  it('rewrites only its own block in AGENTS.md', async () => {
    const { iii, files } = fakeShell({
      whichPi: '/usr/local/bin/pi',
      files: {
        '/hostroot/pi/AGENTS.md': '# House rules\n\nkeep me\n',
      },
    });
    await prepareWorkspace(iii, { ...DEFAULTS });
    const notes = files['/hostroot/pi/AGENTS.md'];
    expect(notes).toContain('# House rules');
    expect(notes).toContain('keep me');
  });

  it('says out loud when the extension has no way to reach the bus', async () => {
    // The terminal host is not necessarily this worker's host, and it will not
    // be one at all if the worker that owns the terminal is ever virtualized:
    // then `iii` may be missing there and every event is a silent no-op.
    const { iii, files } = fakeShell({ whichPi: '/usr/local/bin/pi', noIiiCli: true });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared.bridge).toBe('');
    expect(prepared.detail).toContain('cannot reach the bus');
    expect(files['/hostroot/pi/.pi/extensions/iii-activity.ts']).toContain(
      'pi::terminal::activity',
    );
  });

  it('keeps AGENTS.md and the terminal when a read fails for another reason', async () => {
    // A read that TIMED OUT says nothing about the file. Answering that as
    // "absent" is how a worker overwrites what a person wrote, so the notes are
    // left alone — and the terminal still opens, because a workspace that could
    // not be equipped is still a workspace pi runs in.
    const { iii, files } = fakeShell({
      whichPi: '/usr/local/bin/pi',
      files: { '/hostroot/pi/AGENTS.md': '# House rules\n' },
      readError: new Error('error[S303]: read timed out'),
    });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared.executable).toBe('/usr/local/bin/pi');
    expect(prepared.detail).toContain('could not be equipped');
    expect(files['/hostroot/pi/AGENTS.md']).toBe('# House rules\n');
  });

  it('leaves the workspace alone when setup is off', async () => {
    const { iii, calls } = fakeShell({ whichPi: '/usr/local/bin/pi' });
    await prepareWorkspace(iii, { ...DEFAULTS, setup_workspace: false });
    expect(calls.some((c) => c.function_id === 'shell::fs::write')).toBe(false);
  });
});
