/**
 * What the plugin's SessionStart hook adds to a new session: the default
 * profile, `default-mcp`, read from mcp-server. mcp-server's instructions ask
 * the agent to load it, but Claude Code defers MCP tools until they are
 * searched for, so a session that starts on a small question never does. A
 * hook's `additionalContext` puts the profile in every session for sure.
 */

import type { AgentProfile } from './directory.js';

export const DEFAULT_PROFILE = 'default-mcp';
/** Claude Code keeps hook output under 10,000 characters in context. */
export const MAX_CONTEXT = 9_500;

function output(additionalContext: string): string {
  return JSON.stringify({
    hookSpecificOutput: { hookEventName: 'SessionStart', additionalContext },
  });
}

/** The iii mode rules: the `iii` agent's body, without its frontmatter. */
export function modeRules(agentMarkdown: string): string {
  return agentMarkdown.replace(/^---\n[\s\S]*?\n---\n/, '').trim();
}

/** The hook's stdout for the iii mode rules: a hook of their own, so the
 * rules and the profile each get the whole context budget of a hook. */
export function modeOutput(rules: string): string {
  return output(rules.trim());
}

/** The hook's stdout, for the profile mcp-server returned or the error it hit. */
export function sessionStartOutput(result: { profile: AgentProfile } | { error: string }): string {
  if ('error' in result) {
    return output(
      `The iii plugin could not load the \`${DEFAULT_PROFILE}\` agent profile at session start: ${result.error}`,
    );
  }
  const { profile } = result;
  const id = profile.id || DEFAULT_PROFILE;
  const skills = profile.skills ?? [];
  const header = `Your iii agent profile is \`${id}\`, loaded by the iii plugin at session start from the engine's mcp-server. Follow it for this session, until /iii:profile or /iii:<profile> loads another.`;
  const preload = skills.length
    ? `\n\nBefore your first task, read each of its skills with \`iii_get_skill\`: ${skills.map((s) => `\`${s}\``).join(', ')}.`
    : '';
  const body = `${header}${preload}\n\n<profile id="${id}">\n${profile.system_prompt ?? ''}\n</profile>`;
  if (body.length < MAX_CONTEXT) return output(body);
  return output(
    // Bounded whatever the profile holds: no prompt, no skill list.
    `${header} It is too long to include here: load it now with \`iii_get_agent\` (\`activate: true\`, id \`${id}\`), follow its \`system_prompt\`, and read each id in its \`skills\` with \`iii_get_skill\`.`,
  );
}
