import {
  Activity,
  AtSign,
  Bot,
  Calendar,
  CircleCheck,
  CircleDot,
  Database,
  FileText,
  Folder,
  Hash,
  Link2,
  type LucideIcon,
  Mail,
  MessageCircle,
  MessageSquare,
  Ticket,
  User,
  Users,
  Waypoints,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import './mentions.css'

/**
 * The icon vocabulary a mention provider (or one of its items) may name.
 * Curated rather than "any lucide name" so the bundle stays small and every
 * provider draws from the same visual language; an unknown name falls back
 * to the provider's icon, then to `@`.
 */
export const MENTION_ICONS: Readonly<Record<string, LucideIcon>> = {
  ticket: Ticket,
  issue: CircleDot,
  task: CircleCheck,
  session: MessageSquare,
  chat: MessageSquare,
  message: MessageCircle,
  post: MessageCircle,
  trace: Waypoints,
  span: Activity,
  activity: Activity,
  event: Calendar,
  calendar: Calendar,
  email: Mail,
  mail: Mail,
  user: User,
  person: User,
  team: Users,
  group: Users,
  channel: Hash,
  hash: Hash,
  tweet: AtSign,
  mention: AtSign,
  file: FileText,
  doc: FileText,
  document: FileText,
  folder: Folder,
  link: Link2,
  agent: Bot,
  bot: Bot,
  database: Database,
  table: Database,
}

/** The palette a mention's `color` names (theme-aware glyph tokens). */
export const MENTION_COLORS = [
  'neutral',
  'blue',
  'purple',
  'teal',
  'green',
  'amber',
  'rose',
] as const

export type MentionColor = (typeof MENTION_COLORS)[number]

/** The first known icon among the candidates (item, then provider). */
export function mentionIcon(
  ...candidates: Array<string | undefined>
): LucideIcon {
  for (const name of candidates) {
    if (name && Object.hasOwn(MENTION_ICONS, name)) return MENTION_ICONS[name]
  }
  return AtSign
}

/** The first known color among the candidates (item, then provider). */
export function mentionColor(
  ...candidates: Array<string | undefined>
): MentionColor {
  for (const name of candidates) {
    if (name && (MENTION_COLORS as readonly string[]).includes(name)) {
      return name as MentionColor
    }
  }
  return 'neutral'
}

/** Text color for a preview field's `tone`. */
export function mentionToneClass(tone?: string): string {
  switch (tone) {
    case 'success':
      return 'text-ok'
    case 'warning':
      return 'text-warn'
    case 'danger':
      return 'text-alert'
    case 'info':
      return 'text-accent'
    default:
      return 'text-ink'
  }
}

/** A mention's icon in its color. */
export function MentionGlyph({
  icon,
  color,
  className,
}: {
  icon: LucideIcon
  color: MentionColor
  className?: string
}) {
  const Icon = icon
  return (
    <Icon
      aria-hidden
      data-color={color}
      className={cn('mention-tone shrink-0', className)}
      strokeWidth={2.25}
    />
  )
}
