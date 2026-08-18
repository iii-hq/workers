/**
 * The composer's `/` command registry. Built-ins live in `SLASH_COMMANDS`;
 * the palette (`SlashCommandsPlugin`) merges in dynamic entries from the
 * iii-directory worker — command prompts as `/name`, skills as
 * `/skill:<id>` — and both send paths in `ChatView` expand a leading
 * invocation into an attachment block via `expandSlashInvocation`.
 *
 * Prompt names are strict `[a-z0-9_-]` slugs (iii-directory
 * `validate_name`), so they can never collide with the `skill:` prefix.
 * Built-ins always win a name collision.
 */

import { getCommandPrompt, getSkill } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'

export interface SlashCommand {
  command: string
  description: string
  kind?: 'builtin' | 'prompt' | 'skill'
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

export type SlashInvocation =
  | { kind: 'prompt'; name: string }
  | { kind: 'skill'; id: string }

const SKILL_INVOCATION = /^\/skill:([\w./-]+)(?:\s|$)/
/* Exact server charset from iii-directory's `validate_name`. The `(?:\s|$)`
 * guard means a message starting with an absolute path (`/home/x/y`) never
 * parses as an invocation — a `/` follows the first segment. */
const PROMPT_INVOCATION = /^\/([a-z0-9_-]+)(?:\s|$)/

/** A leading `/name` / `/skill:<id>`, or null (plain text and built-ins). */
export function parseSlashInvocation(text: string): SlashInvocation | null {
  const skill = text.match(SKILL_INVOCATION)
  if (skill) return { kind: 'skill', id: skill[1] }
  const prompt = text.match(PROMPT_INVOCATION)
  if (!prompt) return null
  if (SLASH_COMMANDS.some((c) => c.command === `/${prompt[1]}`)) return null
  return { kind: 'prompt', name: prompt[1] }
}

/** Wrap a resolved body as the attachment block riding with the send. */
export function slashAttachmentBlock(
  inv: SlashInvocation,
  body: string,
): string {
  return inv.kind === 'prompt'
    ? `<command name="${inv.name}">\n${body}\n</command>`
    : `<skill id="${inv.id}">\n${body}\n</skill>`
}

export function invocationCommand(inv: SlashInvocation): string {
  return inv.kind === 'prompt' ? `/${inv.name}` : `/skill:${inv.id}`
}

/**
 * Parse the header of a `<command name="…">` / `<skill id="…">` block back
 * into its invocation. The exact inverse of `slashAttachmentBlock` — names
 * are strict slugs and skill ids path charsets, so no attribute escaping
 * exists to undo.
 */
export function parseSlashBlockHeader(text: string): SlashInvocation | null {
  const command = text.match(/^<command name="([^"]+)">/)
  if (command) return { kind: 'prompt', name: command[1] }
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
    type: inv.kind === 'prompt' ? 'text/x-slash-command' : 'text/x-skill',
  }
}

/* The entries the palette last fetched. The submit-time expander resolves
 * ONLY invocations the palette actually offered, so prose that merely
 * starts with a slash ("/etc is full") never fires a directory RPC. */
let dynamicSlashEntries: SlashCommand[] | null = null

export function setDynamicSlashEntries(entries: SlashCommand[] | null): void {
  dynamicSlashEntries = entries
}

export type SlashExpansion =
  | { status: 'attached'; block: string; inv: SlashInvocation }
  | { status: 'failed'; command: string }

/**
 * Shared submit-time expansion for the fresh-send and queued-edit paths: a
 * leading palette-known `/name` / `/skill:<id>` resolves its body into an
 * attachment block; `failed` means the caller should warn and send the text
 * as typed; null means the text is not an invocation (or not palette-known).
 */
export async function expandSlashInvocation(
  text: string,
): Promise<SlashExpansion | null> {
  const inv = parseSlashInvocation(text)
  if (!inv) return null
  const command = invocationCommand(inv)
  if (!dynamicSlashEntries?.some((e) => e.command === command)) return null
  try {
    const client = await getIiiClient()
    const body =
      inv.kind === 'prompt'
        ? (await getCommandPrompt(client, inv.name)).body
        : (await getSkill(client, inv.id)).body
    return { status: 'attached', block: slashAttachmentBlock(inv, body), inv }
  } catch {
    return { status: 'failed', command }
  }
}
