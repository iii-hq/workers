import { ShadcnAvatar, ShadcnAvatarFallback } from '@iii-dev/console-ui'
import { CircleUserRound } from 'lucide-react'
import type { AgentProfile } from '../shared'
import { agentTone, humanize, initials } from './tone'

/** What an assignee reads as: the profile's name, else the id made readable. */
export function assigneeName(id: string | null, agents: readonly AgentProfile[]): string {
  if (!id) return 'Unassigned'
  return agents.find((agent) => agent.id === id)?.name ?? humanize(id)
}

/**
 * An assignee as a round avatar: initials on the agent's hue (a profile colour, else one derived from the id).
 * Nobody assigned is a dashed ring, the board's "unclaimed" mark.
 */
export function AssigneeAvatar({
  id,
  agents,
  size = 'sm',
}: {
  id: string | null
  agents: readonly AgentProfile[]
  size?: 'sm' | 'default'
}) {
  const name = assigneeName(id, agents)
  if (!id) {
    return (
      <span aria-label={name} className="kb-avatar kb-avatar--none" data-size={size} role="img" title={name}>
        <CircleUserRound size={16} />
      </span>
    )
  }
  const agent = agents.find((candidate) => candidate.id === id)
  return (
    <ShadcnAvatar
      aria-label={name}
      className="kb-avatar"
      data-size={size}
      data-tone={agentTone(id, agent?.color)}
      role="img"
      size={size}
      title={name}
    >
      <ShadcnAvatarFallback className="kb-avatar__initials">{initials(agent?.name ?? id)}</ShadcnAvatarFallback>
    </ShadcnAvatar>
  )
}
