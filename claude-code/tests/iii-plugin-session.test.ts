import { describe, expect, it } from 'vitest';
import { PLUGIN_STATIC_FILES } from '../src/iii-plugin/assets.generated.js';
import { agentFile, MAX_CONTEXT, sessionStartOutput } from '../src/iii-plugin/session.js';
import { pluginFiles } from '../src/plugin.js';

const context = (out: string) => {
  const parsed = JSON.parse(out);
  expect(parsed.hookSpecificOutput.hookEventName).toBe('SessionStart');
  return parsed.hookSpecificOutput.additionalContext as string;
};

describe('the session-start hook loads default-mcp', () => {
  it('puts the profile in the session, naming it and its preloaded skills', () => {
    const text = context(
      sessionStartOutput({
        profile: { id: 'default-mcp', system_prompt: 'You are an iii agent.', skills: ['kanban'] },
      }),
    );
    expect(text).toContain('`default-mcp`');
    expect(text).toContain('You are an iii agent.');
    expect(text).toContain('iii_get_skill');
    expect(text).toContain('kanban');
    expect(text).toContain('/iii:profile');
  });

  it('points at iii_get_agent instead of pasting a prompt too long for a hook', () => {
    const text = context(
      sessionStartOutput({
        profile: { id: 'default-mcp', system_prompt: 'x'.repeat(MAX_CONTEXT) },
      }),
    );
    expect(text.length).toBeLessThan(MAX_CONTEXT);
    expect(text).not.toContain('xxxx');
    expect(text).toContain('iii_get_agent');
    expect(text).toContain('activate: true');
  });

  it('builds the iii agent from default-mcp, with nothing hand-written in its body', () => {
    const md = agentFile({
      id: 'default-mcp',
      system_prompt: 'You are an iii agent.\n\nUse iii_call.',
    });
    // First line: an HTML comment saying the file is generated.
    expect(md.split('\n')[0]).toMatch(/^<!-- .*generated.*overwritten.*default-mcp.* -->$/i);
    const [, fm, body] = md.replace(/^<!--.*-->\n/, '').match(/^---\n([\s\S]*?)\n---\n\n([\s\S]*)$/) ?? [];
    expect(fm).toContain('name: iii');
    expect(fm).toMatch(/description: .*default-mcp/);
    // A plain YAML scalar cannot hold `: `; the value is quoted.
    const value = fm.split('\n').find((l) => l.startsWith('description: '))?.slice(13) ?? '';
    expect(JSON.parse(value)).toContain('default-mcp');
    expect(body.trim()).toBe('You are an iii agent.\n\nUse iii_call.');
  });

  it('keeps the fallback small however many skills the profile preloads', () => {
    const skills = Array.from({ length: 200 }, (_, n) => `skill-${String(n).padStart(54, '0')}`);
    const text = context(
      sessionStartOutput({
        profile: { id: 'default-mcp', system_prompt: 'x'.repeat(MAX_CONTEXT), skills },
      }),
    );
    expect(text.length).toBeLessThan(1000);
    expect(text).not.toContain(skills[0]);
    expect(text).toContain('iii_get_skill');
  });

  it('says why when mcp-server does not answer', () => {
    const text = context(
      sessionStartOutput({ error: 'mcp-server is not reachable at http://127.0.0.1:3111/mcp' }),
    );
    expect(text).toContain('could not load');
    expect(text).toContain('mcp-server is not reachable at http://127.0.0.1:3111/mcp');
  });

  it('runs at session start in the standalone plugin, before the background sync', () => {
    const hooks = JSON.parse(
      PLUGIN_STATIC_FILES.find((f) => f.path === 'hooks/hooks.json')?.content ?? '{}',
    ).hooks;
    const commands = hooks.SessionStart.flatMap((e: { hooks: { command: string }[] }) =>
      e.hooks.map((h) => h.command),
    );
    expect(commands[0]).toBe('node "${CLAUDE_PLUGIN_ROOT}/cli/index.mjs" session-start');
    expect(commands[1]).toContain('/cli/index.mjs" sync');
    expect(commands).toHaveLength(2);
  });

  it('is not in worker installs: the worker appends default-mcp itself', () => {
    const file = pluginFiles({ cli: 'iii', context: '', mcpUrl: 'http://127.0.0.1:3211/mcp' }).find(
      (f) => f.path === 'hooks/hooks.json',
    );
    expect(file?.content).not.toContain('session-start');
  });
});
