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

export function fuzzyFilterSlash(query: string, limit = 8): SlashCommand[] {
  const q = query.trim().toLowerCase()
  if (!q) return SLASH_COMMANDS.slice(0, limit)
  return SLASH_COMMANDS.filter(
    (c) =>
      c.command.toLowerCase().includes(q) ||
      c.description.toLowerCase().includes(q),
  ).slice(0, limit)
}
