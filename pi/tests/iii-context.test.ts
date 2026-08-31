import type { IIIClient } from 'iii-sdk';
import { beforeEach, describe, expect, it } from 'vitest';
import { fetchIiiContext, resetIiiContextCache } from '../src/iii-context.js';

function directory(answers: { prompt?: string | Error; skills?: string | Error }): {
  iii: IIIClient;
  calls: string[];
} {
  const calls: string[] = [];
  const iii = {
    // biome-ignore lint/suspicious/noExplicitAny: minimal stand-in for the SDK client
    trigger: async (req: any): Promise<any> => {
      calls.push(req.function_id);
      if (req.function_id === 'directory::system-prompts::get') {
        if (answers.prompt instanceof Error) throw answers.prompt;
        return { name: req.payload.name, body: answers.prompt ?? '' };
      }
      if (req.function_id === 'directory::skills::index') {
        if (answers.skills instanceof Error) throw answers.skills;
        return { body: answers.skills ?? '', workers_count: 1 };
      }
      return null;
    },
  } as unknown as IIIClient;
  return { iii, calls };
}

describe('the iii context comes from iii-directory', () => {
  beforeEach(() => {
    resetIiiContextCache();
  });

  it('asks for the runtime prompt and the skills index, and joins them', async () => {
    const { iii, calls } = directory({ prompt: '# iii runtime', skills: '# Skills index' });
    const context = await fetchIiiContext(iii);

    expect(calls).toEqual(['directory::system-prompts::get', 'directory::skills::index']);
    expect(context.text).toBe('# iii runtime\n\n# Skills index');
    expect(context.detail).toBe('');
  });

  it('says which half is missing instead of inventing one', async () => {
    // No fallback copy lives in this worker on purpose: a prompt compiled in
    // here would be a second source of truth, and it would go stale.
    const { iii } = directory({
      prompt: new Error('function_not_found'),
      skills: '# Skills index',
    });
    const context = await fetchIiiContext(iii);

    expect(context.text).toBe('# Skills index');
    expect(context.detail).toContain('iii-runtime');
    expect(context.detail).toContain('function_not_found');
  });

  it('reports an empty prompt as missing, not as an empty context', async () => {
    const { iii } = directory({ prompt: '   ', skills: '' });
    const context = await fetchIiiContext(iii);

    expect(context.text).toBe('');
    expect(context.detail).toContain('is empty in iii-directory');
  });

  it('asks once per minute, not once per turn', async () => {
    const { iii, calls } = directory({ prompt: '# iii runtime', skills: '# Skills index' });
    await fetchIiiContext(iii);
    await fetchIiiContext(iii);
    expect(calls).toHaveLength(2);
  });
});
