/**
 * Agent profiles and skills as the plugin sees them (read through mcp-server,
 * see `./mcp.ts`), and how they are written out as Claude Code files.
 */

export type AgentRow = {
  id: string;
  name?: string;
  description?: string;
  hidden?: boolean;
  extends?: string | null;
  model?: string | null;
  reasoning_effort?: string | null;
  skill_count?: number | null;
  function_count?: number;
};

export type AgentProfile = AgentRow & {
  system_prompt?: string;
  skills?: string[];
  functions?: string[];
  unknown_skills?: string[];
  unknown_functions?: string[];
  inheritance_error?: string;
};

export type SkillRow = { id: string; title?: string; type?: string | null; description?: string };
export type Skill = SkillRow & { body?: string };

/** A slug Claude Code accepts as an agent or skill name. */
export function slug(id: string): string {
  return id
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

function yamlString(value: string): string {
  return JSON.stringify(value.replace(/\s+/g, ' ').trim());
}

/**
 * An agent profile as a Claude Code subagent file (`agents/<name>.md`).
 *
 * The model is not carried over: iii profiles name models across every
 * provider, and a subagent with a model Claude Code cannot run fails to start.
 * Preloaded skills become an instruction to read them, because their bodies
 * live in the directory and stay there.
 */
export function renderSubagent(profile: AgentProfile): string {
  const name = `iii-${slug(profile.id)}`;
  const description = profile.description || profile.name || profile.id;
  const skills = profile.skills ?? [];
  const preload = skills.length
    ? `\n\n## Preloaded skills\n\nBefore starting, read each of these with the \`iii_get_skill\` tool: ${skills
        .map((s) => `\`${s}\``)
        .join(', ')}.`
    : '';
  return `---
name: ${name}
description: ${yamlString(`${description} (iii agent profile \`${profile.id}\`)`)}
---

${(profile.system_prompt ?? '').trim()}${preload}
`;
}

/** A directory skill as a Claude Code skill file (`skills/<name>/SKILL.md`). */
export function renderSkill(skill: Skill): string {
  const description = skill.description || skill.title || skill.id;
  return `---
name: ${slug(skill.id)}
description: ${yamlString(description)}
---

${(skill.body ?? '').trim()}
`;
}

/**
 * The directory each skill exports to, by its slug. Refuses ids that land in
 * the same directory, so no `SKILL.md` silently replaces another.
 */
export function skillDirectories(ids: string[]): Map<string, string> {
  const dirs = new Map<string, string>();
  for (const id of ids) {
    const dir = slug(id);
    const taken = dirs.get(dir);
    if (taken !== undefined) {
      throw new Error(`skills ${taken} and ${id} would both export to skills/${dir}; rename one`);
    }
    dirs.set(dir, id);
  }
  return dirs;
}
