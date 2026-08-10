/**
 * Thin READ client over `directory::system-prompts::*` for the chat's
 * new-session picker: list the names, resolve one body at selection time.
 * Authoring is deliberately absent — the iii-directory UI's system-prompts
 * tab owns `create` / `update`.
 */

import type { IiiClient } from '@/lib/iii-client'

export interface PromptEntry {
  name: string
  description: string
  modified_at: string
}

export interface PromptBody extends PromptEntry {
  body: string
}

export async function listPrompts(client: IiiClient): Promise<PromptEntry[]> {
  const res = await client.trigger<{ prompts: PromptEntry[] }>(
    'directory::system-prompts::list',
    {},
  )
  return res.prompts
}

export async function getPrompt(
  client: IiiClient,
  name: string,
): Promise<PromptBody> {
  return client.trigger<PromptBody>('directory::system-prompts::get', {
    name,
  })
}
