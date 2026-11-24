import { describe, expect, it, vi } from 'vitest';
import type { OpenSpec } from '../../src/agents.js';
import { registerTerminal } from '../../src/terminal/terminal.js';
import { fakeIii } from '../_helpers/fake-iii.js';

const prepared = {
  workspace: '/ws',
  executable: '/bin/claude',
  args: ['--plugin-dir', '/p'],
  env: { HOME: '/home/u' },
  detail: '',
  bridge: '/bin/iii',
  plugin: '/p',
};
const plain = {
  program: '/bin/claude',
  args: ['--plugin-dir', '/p'],
  cwd: '/ws',
  env: { HOME: '/home/u' },
  activity_bridge: '/bin/iii',
  detail: '',
};

function setup(current = prepared) {
  const fake = fakeIii();
  const resolveOpen = vi.fn<(id: string) => Promise<OpenSpec>>();
  registerTerminal(fake.iii, () => current, resolveOpen);
  const describeTerminal = async (payload?: unknown) =>
    fake.registered.get('claude::terminal::describe')?.(payload);
  return { fake, resolveOpen, describeTerminal };
}

describe('claude::terminal::describe', () => {
  it('answers the workspace session when no session is asked for', async () => {
    const { resolveOpen, describeTerminal, fake } = setup();
    for (const payload of [undefined, null, {}]) {
      expect(await describeTerminal(payload)).toEqual(plain);
    }
    expect(resolveOpen).not.toHaveBeenCalled();
    expect(fake.functions.get('claude::terminal::describe')?.options.metadata).toEqual({
      internal: true,
      trace_hidden: true,
    });
  });

  it('answers how to open the session asked for', async () => {
    const { resolveOpen, describeTerminal } = setup();
    resolveOpen.mockResolvedValueOnce({ args: ['attach', '7e64247f'], cwd: '/repo' });
    expect(await describeTerminal({ session: 's' })).toEqual({
      ...plain,
      args: ['attach', '7e64247f'],
      cwd: '/repo',
    });
    const copy = { args: ['--resume', 'x', '--fork-session'], cwd: '/repo', copy: true as const };
    resolveOpen.mockResolvedValueOnce(copy);
    expect(await describeTerminal({ session: 's' })).toEqual({ ...plain, ...copy });
    expect(resolveOpen).toHaveBeenCalledWith('s');
  });

  it('refuses with the reason the session cannot be opened', async () => {
    const { resolveOpen, describeTerminal } = setup();
    resolveOpen.mockRejectedValueOnce(new Error('no session with that id is listed'));
    await expect(describeTerminal({ session: 's' })).rejects.toThrow(
      'no session with that id is listed',
    );
  });

  it.each(['', 'x'.repeat(201), 7])('rejects the session %j', async (session) => {
    const { resolveOpen, describeTerminal } = setup();
    await expect(describeTerminal({ session })).rejects.toThrow();
    expect(resolveOpen).not.toHaveBeenCalled();
  });

  it('refuses without a claude on the terminal host', async () => {
    const { describeTerminal } = setup({ ...prepared, executable: '', detail: 'no claude' });
    await expect(describeTerminal({ session: 's' })).rejects.toThrow('no claude');
  });
});
