/**
 * The iii mode: an `iii` agent the whole session runs as, a hook that
 * pre-approves the plugin's read-only tools, and instructions in every
 * plugin-owned prompt to reach the engine through iii_call / iii_observe.
 */

import { describe, expect, it } from 'vitest';
import { PLUGIN_STATIC_FILES } from '../src/iii-plugin/assets.generated.js';
import { profileCommand } from '../src/iii-plugin/commands.js';
import {
  APPROVED_TOOLS,
  approveHookEntry,
  III_AGENT,
  NEVER_APPROVED_TOOLS,
} from '../src/iii-plugin/mode.js';
import { pluginFiles } from '../src/plugin.js';

const file = (path: string) => {
  const f = PLUGIN_STATIC_FILES.find((x) => x.path === path);
  if (!f) throw new Error(`${path} is not embedded in the plugin`);
  return f.content;
};

function frontmatter(md: string): Record<string, string> {
  const m = md.match(/^---\n([\s\S]*?)\n---\n/);
  if (!m) throw new Error('no frontmatter');
  return Object.fromEntries(
    m[1].split('\n').map((line) => {
      const i = line.indexOf(':');
      return [line.slice(0, i).trim(), line.slice(i + 1).trim()];
    }),
  );
}

/** How Claude Code applies a hook matcher: a regex over the tool name. */
const matches = (matcher: string, tool: string) => new RegExp(matcher).test(tool);
const tool = (name: string) => `mcp__plugin_iii_iii__${name}`;

describe('the iii agent (iii mode)', () => {
  it('is addressed as iii:iii', () => {
    expect(III_AGENT).toBe('iii:iii');
  });

  it('ships as agents/iii.md with a name and a description', () => {
    const fm = frontmatter(file('agents/iii.md'));
    expect(fm.name).toBe('iii');
    expect(fm.description.length).toBeGreaterThan(20);
    // iii profiles and operators pick models; the mode must not pin one.
    expect(fm.model).toBeUndefined();
  });

  it('instructs the agent to use iii_call and iii_observe for the engine', () => {
    const body = file('agents/iii.md');
    for (const t of ['iii_call', 'iii_observe', 'iii_events', 'iii_unobserve'])
      expect(body).toContain(t);
    expect(body).toMatch(/instead of .*`iii call`/i);
    expect(body).toContain('/iii:profile');
    expect(body).toContain('/iii:skills');
  });
});

describe('the read-only approval hook', () => {
  const entry = approveHookEntry();

  it('approves every read-only plugin tool', () => {
    for (const name of APPROVED_TOOLS) expect(matches(entry.matcher, tool(name)), name).toBe(true);
  });

  it('never approves calling or observing: those stay with the permission mode', () => {
    expect(NEVER_APPROVED_TOOLS).toEqual(['iii_call', 'iii_observe']);
    for (const name of NEVER_APPROVED_TOOLS)
      expect(matches(entry.matcher, tool(name)), name).toBe(false);
  });

  it('does not match other servers or look-alike names', () => {
    expect(matches(entry.matcher, 'mcp__other__iii_status')).toBe(false);
    expect(matches(entry.matcher, `${tool('iii_status')}_x`)).toBe(false);
    expect(matches(entry.matcher, 'Bash')).toBe(false);
  });

  it('answers with an allow decision in the PreToolUse output format', () => {
    const command = entry.hooks[0].command;
    const json = command.match(/'(\{.*\})'/)?.[1] ?? '';
    expect(JSON.parse(json)).toEqual({
      hookSpecificOutput: {
        hookEventName: 'PreToolUse',
        permissionDecision: 'allow',
        permissionDecisionReason: expect.stringContaining('read-only'),
      },
    });
  });

  it('is in the standalone hooks file, beside the SessionStart sync', () => {
    const hooks = JSON.parse(file('hooks/hooks.json')).hooks;
    expect(hooks.PreToolUse).toEqual([entry]);
    expect(hooks.SessionStart[0].hooks[0].command).toContain(' sync ');
  });

  it('is in the worker hooks too, after the activity hook', () => {
    const hooksFile = pluginFiles({ cli: 'iii', context: '' }).find(
      (f) => f.path === 'hooks/hooks.json',
    );
    const hooks = JSON.parse(hooksFile?.content ?? '{}').hooks;
    expect(hooks.PreToolUse[0].hooks[0].command).toContain('claude::terminal::activity');
    expect(hooks.PreToolUse[1]).toEqual(entry);
  });
});

describe('plugin prompts point at iii_call and iii_observe', () => {
  it('/iii:profile', () => {
    const md = file('commands/profile.md');
    expect(md).toContain('iii_call');
    expect(md).toContain('iii_observe');
  });

  it('/iii:skills', () => {
    const md = file('commands/skills.md');
    expect(md).toContain('iii_call');
    expect(md).toContain('iii_observe');
  });

  it('generated /iii:<profile> commands', () => {
    const md = profileCommand({ id: 'writer' }).content;
    expect(md).toContain('iii_call');
    expect(md).toContain('iii_observe');
  });

  it('the plugin README documents the tools and the mode', () => {
    const md = file('README.md');
    for (const t of [
      'iii_call',
      'iii_observe',
      'iii_events',
      'iii_unobserve',
      'iii_observers',
      'iii:iii',
    ]) {
      expect(md).toContain(t);
    }
  });
});
