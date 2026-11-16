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
  HEADLESS_TOOLS,
  headlessTools,
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
  const m = md.replace(/^<!--.*-->\n/, '').match(/^---\n([\s\S]*?)\n---\n/);
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
    // Quoted: a plain YAML scalar cannot hold `: `.
    expect(JSON.parse(fm.description)).toContain('default-mcp');
    // iii profiles and operators pick models; the mode must not pin one.
    expect(fm.model).toBeUndefined();
  });

  it('ships only a stub that loads default-mcp until sync replaces it', () => {
    // `---` opens the file (Claude Code reads frontmatter only then); the
    // generated note is the first line inside it, as a YAML comment.
    const lines = file('agents/iii.md').split('\n');
    expect(lines[0]).toBe('---');
    expect(lines[1]).toMatch(/^# .*generated.*overwritten/i);
    const body = file('agents/iii.md').replace(/^---\n[\s\S]*?\n---\n/, '');
    expect(body).not.toMatch(/generated/i);
    expect(body).toContain('default-mcp');
    expect(body).toContain('iii_get_agent');
    expect(body).toContain('activate: true');
    // No second, hand-written copy of the profile's rules.
    expect(body.length).toBeLessThan(800);
    expect(body).not.toContain('iii_observations_poll');
  });
});

describe('the read-only approval hook', () => {
  const entry = approveHookEntry();

  it('approves every read-only plugin tool', () => {
    for (const name of APPROVED_TOOLS) expect(matches(entry.matcher, tool(name)), name).toBe(true);
  });

  it('never approves calling or observing: those stay with the permission mode', () => {
    expect(NEVER_APPROVED_TOOLS).toEqual(['iii_call', 'iii_observe', 'iii_secret_store']);
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
    expect(hooks.SessionStart[0].hooks[1].command).toContain(' sync ');
  });

  it('is in the worker hooks too, after the activity hook', () => {
    const hooksFile = pluginFiles({ cli: 'iii', context: '', mcpUrl: 'http://x/mcp' }).find(
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
      'iii_observations_poll',
      'iii_unobserve',
      'iii_observers',
      'iii:iii',
      'mcp-server',
      'mcp_url',
      'active_profile',
    ]) {
      expect(md).toContain(t);
    }
  });
});

describe('the plugin uses mcp-server tool names', () => {
  it("never names the old embedded server's tools", () => {
    for (const f of PLUGIN_STATIC_FILES.filter((x) => !x.path.startsWith('cli/'))) {
      expect(f.content, f.path).not.toContain('iii_events');
    }
    expect(profileCommand({ id: 'writer' }).content).not.toContain('iii_events');
    expect(APPROVED_TOOLS).not.toContain('iii_events');
  });

  it('pre-approves reading observations and statuses, never storing a secret', () => {
    expect(APPROVED_TOOLS).toEqual(
      expect.arrayContaining([
        'iii_observations_poll',
        'iii_observations_sse',
        'iii_observers',
        'iii_status',
      ]),
    );
    expect(APPROVED_TOOLS).not.toContain('iii_secret_store');
  });

  it('worker installs point the session at the mcp-server they found', () => {
    const mcp = pluginFiles({ cli: 'iii', context: '', mcpUrl: 'http://127.0.0.1:3211/mcp' }).find(
      (f) => f.path === '.mcp.json',
    );
    expect(JSON.parse(mcp?.content ?? '{}')).toEqual({
      mcpServers: { iii: { type: 'http', url: 'http://127.0.0.1:3211/mcp' } },
    });
  });
});

describe('the active profile', () => {
  it('/iii:profile with no arguments shows the active profile first, from iii_status', () => {
    const md = file('commands/profile.md');
    const noArgs = md.slice(md.indexOf('**No arguments:**'), md.indexOf('**Otherwise**'));
    expect(noArgs).toContain('iii_status');
    expect(noArgs).toContain('active_profile');
    expect(noArgs).toContain('default-mcp');
    expect(noArgs.indexOf('active_profile')).toBeLessThan(noArgs.indexOf('iii_list_agents'));
    // mcp-server tracks it per MCP session: a renewed session starts at null
    // while the conversation may still follow a profile.
    expect(noArgs).toContain('renewed');
    expect(noArgs).toContain('30 minutes');
    expect(frontmatter(md)['allowed-tools']).toContain(tool('iii_status'));
  });

  it('loading a profile activates it', () => {
    expect(file('commands/profile.md')).toContain('activate: true');
    expect(profileCommand({ id: 'writer' }).content).toContain('activate: true');
  });

  it('the iii agent starts from default-mcp', () => {
    const body = file('agents/iii.md');
    expect(body).toContain('default-mcp');
    expect(body).toContain('activate: true');
  });

  it('every command says what to do when mcp-server is not reachable', () => {
    for (const md of [
      file('commands/profile.md'),
      file('commands/skills.md'),
      profileCommand({ id: 'writer' }).content,
    ]) {
      expect(md).toContain('mcp-server');
      expect(md).toContain('mcp_url');
    }
  });
});

describe('headless turns', () => {
  it('may run the iii tools: rbac-proxy, not a permission prompt nobody answers, is the boundary', () => {
    for (const t of [
      'iii_call',
      'iii_observe',
      'iii_observations_poll',
      'iii_unobserve',
      'iii_status',
    ])
      expect(HEADLESS_TOOLS).toContain(tool(t));
    expect(HEADLESS_TOOLS).not.toContain(tool('iii_secret_store'));
  });
});

describe('headless turns with the approval gate on', () => {
  it('leave calling and observing to the gate', () => {
    expect(headlessTools(false)).toEqual(HEADLESS_TOOLS);
    const gated = headlessTools(true);
    expect(gated).not.toContain(tool('iii_call'));
    expect(gated).not.toContain(tool('iii_observe'));
    expect(gated).toContain(tool('iii_status'));
  });
});
