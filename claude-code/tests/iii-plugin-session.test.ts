import { describe, expect, it } from 'vitest';
import { PLUGIN_STATIC_FILES } from '../src/iii-plugin/assets.generated.js';
import { MAX_CONTEXT, modeOutput, sessionStartOutput } from '../src/iii-plugin/session.js';
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

  it('adds the iii mode rules in a hook of their own, so neither crowds out the profile', () => {
    const text = context(modeOutput('Use iii_call for the engine.'));
    expect(text).toBe('Use iii_call for the engine.');
    const profile = context(
      sessionStartOutput({ profile: { id: 'default-mcp', system_prompt: 'p'.repeat(7000) } }),
    );
    expect(profile).toContain('p'.repeat(7000));
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
    expect(commands[0]).toBe('node "${CLAUDE_PLUGIN_ROOT}/cli/index.mjs" mode');
    expect(commands[1]).toBe('node "${CLAUDE_PLUGIN_ROOT}/cli/index.mjs" session-start');
    expect(commands[2]).toContain('/cli/index.mjs" sync');
  });

  it('runs at session start in worker installs too, at the mcp-server the worker found', () => {
    const file = pluginFiles({ cli: 'iii', context: '', mcpUrl: 'http://127.0.0.1:3211/mcp' }).find(
      (f) => f.path === 'hooks/hooks.json',
    );
    const hooks = JSON.parse(file?.content ?? '{}').hooks;
    const commands = hooks.SessionStart.flatMap((e: { hooks: { command: string }[] }) =>
      e.hooks.map((h) => h.command),
    );
    expect(commands[0]).toContain('claude::terminal::activity');
    expect(commands).toContain(
      `node "\${CLAUDE_PLUGIN_ROOT}/cli/index.mjs" session-start --url 'http://127.0.0.1:3211/mcp'`,
    );
  });
});
