import { listPrompts } from '@/lib/prompts'

export interface SlashCommand {
  command: string
  description: string
  /**
   * Directory prompt name backing this entry. Builtins leave it unset;
   * prompt entries insert their BODY into the composer on select
   * (claude-code-style context injection, never the system prompt).
   */
  promptName?: string
}

export const SLASH_COMMANDS: SlashCommand[] = [
  {
    command: '/compact',
    description: 'summarise this session and free up context',
  },
]

/**
 * Directory prompts as slash entries: worker-shipped templates and user
 * library entries alike, labeled by kind. An absent directory worker (or
 * an older one) degrades to the builtins only.
 */
export async function loadPromptSlashCommands(): Promise<SlashCommand[]> {
  try {
    const prompts = await listPrompts()
    // Only command-kind templates inject into the message; system prompts
    // apply through the composer's prompt picker instead.
    return prompts
      .filter((p) => p.kind === 'command')
      .map((p) => ({
        command: `/${p.name}`,
        description: `${p.kind} prompt - ${p.description}`,
        promptName: p.name,
      }))
  } catch {
    return []
  }
}

export function fuzzyFilterSlash(
  query: string,
  commands: SlashCommand[] = SLASH_COMMANDS,
  limit = 8,
): SlashCommand[] {
  const q = query.trim().toLowerCase()
  if (!q) return commands.slice(0, limit)
  return commands
    .filter(
      (c) =>
        c.command.toLowerCase().includes(q) ||
        c.description.toLowerCase().includes(q),
    )
    .slice(0, limit)
}
