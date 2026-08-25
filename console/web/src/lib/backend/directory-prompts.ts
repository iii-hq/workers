/**
 * Thin READ client over the iii-directory worker for the chat:
 *
 *  - `directory::system-prompts::*` — the new-session identity picker.
 *  - `directory::prompts::*` — command templates, offered as session addons
 *    on the welcome screen and as `/name` slash commands in the composer.
 *  - `directory::skills::*` — listed by the session ID filter and offered as
 *    `/skill:<id>` slash commands (only the manual command resolves a body).
 *
 * Authoring is deliberately absent — the iii-directory UI's page owns
 * `create` / `update` / `delete`.
 */

import type { IiiClient } from '@/lib/iii-client'

/* These fetches sit on interactive surfaces (pickers, palette, send path);
 * a hung directory worker must fail them fast, not hold the UI to the
 * client's 5-minute default trigger timeout. */
const FETCH_TIMEOUT_MS = 10_000

export interface PromptEntry {
  name: string
  description: string
  modified_at: string
}

export interface PromptBody extends PromptEntry {
  body: string
}

export interface SkillEntry {
  id: string
  title: string
  description: string
  modified_at: string
  disable_model_invocation: boolean
}

export interface SkillBody {
  id: string
  title: string
  body: string
  disable_model_invocation: boolean
  /** Absolute on-disk path of the skill file; its parent directory is the
   * skill's base directory (where `scripts/`, `reference/` payload lives).
   * Absent on directory workers that predate the field. */
  path?: string
  modified_at: string
}

/**
 * The skill body with its base directory announced. Payload skills
 * (agent-skills convention: SKILL.md beside scripts/ and reference/)
 * reference their own files by relative path and need the runtime to say
 * where they live — without this line a session scoped to the user's
 * project can never find them. Body-only when the worker sends no path.
 */
export function skillBodyWithBaseDir(skill: SkillBody): string {
  if (!skill.path) return skill.body
  // Both separators: the worker ships Windows binaries, so `path` can be a
  // `C:\...\SKILL.md`. A missing separator must fall through to body-only —
  // `slice(0, -1)` would otherwise hand the model a path with its last
  // character shaved off, which is worse than saying nothing.
  const cut = Math.max(
    skill.path.lastIndexOf('/'),
    skill.path.lastIndexOf('\\'),
  )
  if (cut <= 0) return skill.body
  const dir = skill.path.slice(0, cut)
  return `${skill.body}\n\nSkill base directory: ${dir} — resolve the skill's relative paths (scripts/, reference/, …) against it; keep the working directory at the user's project.`
}

export async function listPrompts(client: IiiClient): Promise<PromptEntry[]> {
  const res = await client.trigger<{ prompts: PromptEntry[] }>(
    'directory::system-prompts::list',
    {},
    { timeoutMs: FETCH_TIMEOUT_MS },
  )
  return res.prompts
}

export async function getPrompt(
  client: IiiClient,
  name: string,
): Promise<PromptBody> {
  return client.trigger<PromptBody>(
    'directory::system-prompts::get',
    { name },
    { timeoutMs: FETCH_TIMEOUT_MS },
  )
}

export async function listCommandPrompts(
  client: IiiClient,
): Promise<PromptEntry[]> {
  const res = await client.trigger<{ prompts: PromptEntry[] }>(
    'directory::prompts::list',
    {},
    { timeoutMs: FETCH_TIMEOUT_MS },
  )
  return res.prompts
}

export async function getCommandPrompt(
  client: IiiClient,
  name: string,
): Promise<PromptBody> {
  return client.trigger<PromptBody>(
    'directory::prompts::get',
    { name },
    { timeoutMs: FETCH_TIMEOUT_MS },
  )
}

export async function listSkills(client: IiiClient): Promise<SkillEntry[]> {
  const res = await client.trigger<{ skills: SkillEntry[] }>(
    'directory::skills::list',
    { include_description: true },
    { timeoutMs: FETCH_TIMEOUT_MS },
  )
  return res.skills
}

export async function getSkill(
  client: IiiClient,
  id: string,
): Promise<SkillBody> {
  return client.trigger<SkillBody>(
    'directory::skills::get',
    { id },
    { timeoutMs: FETCH_TIMEOUT_MS },
  )
}

/* ── agent profiles (`directory::agents::*`) — the new-session picker ── */

export interface AgentEntry {
  id: string
  name: string
  description: string
  logo: string | null
  icon: string | null
  model: string | null
  skill_count: number | null
  /** true = a specialist meant to be spawned, not to front a session.
   * Absent on directory workers that predate the field. */
  leaf?: boolean
  modified_at: string
}

export interface AgentProfile extends AgentEntry {
  system_prompt: string
  skills: string[]
  unknown_skills: string[]
}

export async function listAgents(client: IiiClient): Promise<AgentEntry[]> {
  const res = await client.trigger<{ agents: AgentEntry[] }>(
    'directory::agents::list',
    {},
    { timeoutMs: FETCH_TIMEOUT_MS },
  )
  return res.agents
}

export async function getAgent(
  client: IiiClient,
  id: string,
): Promise<AgentProfile> {
  return client.trigger<AgentProfile>(
    'directory::agents::get',
    { id },
    { timeoutMs: FETCH_TIMEOUT_MS },
  )
}
