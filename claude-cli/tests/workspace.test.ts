import type { IIIClient } from 'iii-sdk';
import { describe, expect, it } from 'vitest';
import { DEFAULTS, normalize } from '../src/config.js';
import { prepareWorkspace } from '../src/workspace.js';

type Call = { function_id: string; payload: Record<string, unknown> };

/**
 * A shell worker that answers from a fake filesystem. Everything
 * `prepareWorkspace` does goes over the bus, so this is the whole boundary.
 */
function fakeShell(options: { files?: Record<string, string>; whichClaude?: string } = {}) {
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
          if (command === 'command -v claude') {
            return options.whichClaude
              ? { stdout: `${options.whichClaude}\n`, stderr: '', exit_code: 0 }
              : { stdout: '', stderr: '', exit_code: 1 };
          }
          if (command === 'command -v iii')
            return { stdout: '/usr/bin/iii\n', stderr: '', exit_code: 0 };
          return { stdout: '', stderr: '', exit_code: 0 };
        }
        case 'shell::fs::write':
          files[String(payload.path)] = String(payload.content);
          return { bytes_written: String(payload.content).length };
        case 'coder::read-file': {
          const content = files[String(payload.path)];
          if (content === undefined) throw new Error('no such file');
          return { content };
        }
        default:
          return {};
      }
    },
  } as unknown as IIIClient;
  return { iii, calls, files };
}

describe('config', () => {
  it('drops keys an older schema wrote and keeps the defaults', () => {
    expect(normalize({ args: ['-p'], nonsense: 1 })).toEqual({ ...DEFAULTS, args: ['-p'] });
    expect(normalize(null)).toEqual(DEFAULTS);
  });
});

describe('preparing the terminal host', () => {
  it('equips a fresh workspace and reports the binary', async () => {
    const { iii, calls, files } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    const prepared = await prepareWorkspace(iii, { ...DEFAULTS });

    expect(prepared).toMatchObject({
      workspace: '/hostroot/claude-cli',
      executable: '/usr/local/bin/claude',
      detail: '',
    });

    // The workspace is its own npm project, or the skills CLI installs above it.
    expect(files['/hostroot/claude-cli/package.json']).toContain('claude-cli-workspace');
    expect(calls.some((c) => String(c.payload.command ?? '').startsWith('npx -y skills add'))).toBe(
      true,
    );
    expect(files['/hostroot/claude-cli/.iii/skills-installed']).toBeTruthy();

    // The notes live in one marked block Claude reads on startup.
    const notes = files['/hostroot/claude-cli/CLAUDE.md'];
    expect(notes).toContain('<!-- iii:begin');
    expect(notes).toContain('<!-- iii:end -->');
    expect(notes).toContain('/hostroot/claude-cli');

    // Every hook posts the payload to this worker over the bus, exactly once
    // expanded — a prompt containing shell syntax stays data.
    const settings = JSON.parse(files['/hostroot/claude-cli/.claude/settings.json']);
    expect(Object.keys(settings.hooks).sort()).toEqual([
      'PostToolUse',
      'PreToolUse',
      'SessionEnd',
      'SessionStart',
      'Stop',
      'UserPromptSubmit',
    ]);
    const command = settings.hooks.PreToolUse[0].hooks[0].command;
    expect(command).toContain('/usr/bin/iii trigger claude-cli::activity --json "$(cat)"');
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
        '/hostroot/claude-cli/.claude/settings.json': JSON.stringify({
          model: 'opus',
          hooks: { Notification: [{ hooks: [{ type: 'command', command: 'say hi' }] }] },
        }),
        '/hostroot/claude-cli/CLAUDE.md': '# My own notes\n\nkeep me\n',
        '/hostroot/claude-cli/.iii/skills-installed': '2026-01-01',
      },
    });
    await prepareWorkspace(iii, { ...DEFAULTS });

    const settings = JSON.parse(files['/hostroot/claude-cli/.claude/settings.json']);
    expect(settings.model).toBe('opus');
    expect(settings.hooks.Notification[0].hooks[0].command).toBe('say hi');
    expect(settings.hooks.PreToolUse).toBeDefined();

    const notes = files['/hostroot/claude-cli/CLAUDE.md'];
    expect(notes).toContain('# My own notes');
    expect(notes).toContain('keep me');
    expect(notes.indexOf('<!-- iii:begin')).toBeLessThan(notes.indexOf('# My own notes'));
  });

  it('leaves the workspace alone when setup is off', async () => {
    const { iii, calls } = fakeShell({ whichClaude: '/usr/local/bin/claude' });
    await prepareWorkspace(iii, { ...DEFAULTS, setup_workspace: false });
    expect(calls.some((c) => c.function_id === 'shell::fs::write')).toBe(false);
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
