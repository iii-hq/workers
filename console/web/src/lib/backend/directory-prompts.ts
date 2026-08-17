/**
 * Thin READ client over the iii-directory worker for the chat:
 *
 *  - `directory::system-prompts::*` — the new-session identity picker.
 *  - `directory::prompts::*` — command templates, offered as session addons
 *    on the welcome screen and as `/name` slash commands in the composer.
 *  - `directory::skills::*` — skills, offered as session addons and as
 *    `/skill:<id>` slash commands.
 *
 * The pattern everywhere: list the names, resolve one body at selection
 * time. Authoring is deliberately absent — the iii-directory UI's page owns
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
}

export interface SkillBody {
  id: string
  title: string
  body: string
  modified_at: string
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
