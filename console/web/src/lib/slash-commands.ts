/**
 * The composer's `/` command registry. Built-ins live in `SLASH_COMMANDS`;
 * the palette (`SlashCommandsPlugin`) merges in skills from iii-directory as
 * `/skill:<id>`. Both send paths in `ChatView` expand a leading invocation
 * into an attachment block via `expandSlashInvocation`.
 */

import { getSkill, skillBodyWithBaseDir } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'

export interface SlashCommand {
  command: string
  description: string
}

export const SLASH_COMMANDS: SlashCommand[] = [
  {
    command: '/compact',
    description: 'summarise this session and free up context',
  },
]

export function fuzzyFilterSlash(
  query: string,
  entries: SlashCommand[] = SLASH_COMMANDS,
  limit = 8,
): SlashCommand[] {
  const q = query.trim().toLowerCase()
  if (!q) return entries.slice(0, limit)
  return entries
    .filter(
      (c) =>
        c.command.toLowerCase().includes(q) ||
        c.description.toLowerCase().includes(q),
    )
    .slice(0, limit)
}

/** Built-ins first; dynamic entries shadowed by a built-in name are dropped. */
export function mergeSlashEntries(dynamic: SlashCommand[]): SlashCommand[] {
  const builtins = new Set(SLASH_COMMANDS.map((c) => c.command))
  return [...SLASH_COMMANDS, ...dynamic.filter((c) => !builtins.has(c.command))]
}

export type SlashInvocation = { kind: 'skill'; id: string }

const SKILL_INVOCATION = /^\/skill:([\w./-]+)(?:\s|$)/

/** A leading `/skill:<id>`, or null (ordinary text and built-ins). */
export function parseSlashInvocation(text: string): SlashInvocation | null {
  const skill = text.match(SKILL_INVOCATION)
  if (skill) return { kind: 'skill', id: skill[1] }
  return null
}

/** Wrap a resolved body as the attachment block riding with the send. */
export function slashAttachmentBlock(
  inv: SlashInvocation,
  body: string,
): string {
  return `<skill id="${inv.id}">\nThis skill is already loaded. Follow it directly; do not search for or reload it.\n\n${body}\n</skill>`
}

export function invocationCommand(inv: SlashInvocation): string {
  return `/skill:${inv.id}`
}

/**
 * Parse the header of a `<skill id="…">` block back into its invocation.
 * The exact inverse of `slashAttachmentBlock`.
 */
export function parseSlashBlockHeader(text: string): SlashInvocation | null {
  const skill = text.match(/^<skill id="([^"]+)">/)
  if (skill) return { kind: 'skill', id: skill[1] }
  return null
}

/**
 * The attachment chip a slash block collapses to — shared by the optimistic
 * send row and the transcript hydration path so both render identically.
 */
export function slashChip(
  inv: SlashInvocation,
  blockSize: number,
): { id: string; name: string; size: number; type: string } {
  const command = invocationCommand(inv)
  return {
    id: `slash-${command}`,
    name: command,
    size: blockSize,
    type: 'text/x-skill',
  }
}

/* The entries the palette last fetched. The submit-time expander resolves
 * ONLY invocations the palette actually offered, so ordinary slash-prefixed
 * prose never fires a directory RPC. */
let dynamicSlashEntries: SlashCommand[] | null = null

export function setDynamicSlashEntries(entries: SlashCommand[] | null): void {
  dynamicSlashEntries = entries
}

export type SlashExpansion =
  | { status: 'attached'; block: string; inv: SlashInvocation }
  | { status: 'failed'; command: string }

/**
 * Skill ids whose full body already rides in this conversation's context:
 * skill chips on messages after the last compaction marker. Compaction
 * summarises the earlier block away, so it stops counting as loaded.
 */
export function loadedSkillIds(
  messages: ReadonlyArray<{
    role: string
    kind?: string
    attachments?: ReadonlyArray<{ name: string; type: string }>
  }>,
): ReadonlySet<string> {
  const ids = new Set<string>()
  for (const m of messages) {
    if (m.role === 'system' && m.kind === 'compaction') {
      ids.clear()
      continue
    }
    for (const a of m.attachments ?? []) {
      if (a.type === 'text/x-skill' && a.name.startsWith('/skill:'))
        ids.add(a.name.slice('/skill:'.length))
    }
  }
  return ids
}

/**
 * Shared submit-time expansion for the fresh-send and queued-edit paths: a
 * leading palette-known `/skill:<id>` resolves its body into an
 * attachment block; `failed` means the caller should warn and send the text
 * as typed; null means the text is not an invocation (or not palette-known).
 *
 * A skill in `loaded` (see `loadedSkillIds`) expands to a short pointer
 * instead of refetching the body — the full block is already in context, so
 * a repeat invocation shouldn't cost another copy of it.
 */
export async function expandSlashInvocation(
  text: string,
  loaded?: ReadonlySet<string>,
): Promise<SlashExpansion | null> {
  const inv = parseSlashInvocation(text)
  if (!inv) return null
  const command = invocationCommand(inv)
  if (!dynamicSlashEntries?.some((e) => e.command === command)) return null
  if (inv.kind === 'skill' && loaded?.has(inv.id)) {
    return {
      status: 'attached',
      block: `<skill id="${inv.id}">\nThis skill was already loaded by an earlier message in this conversation. Follow that skill block directly; do not reload it.\n</skill>`,
      inv,
    }
  }
  try {
    const client = await getIiiClient()
    const body = skillBodyWithBaseDir(await getSkill(client, inv.id))
    return { status: 'attached', block: slashAttachmentBlock(inv, body), inv }
  } catch {
    return { status: 'failed', command }
  }
}
