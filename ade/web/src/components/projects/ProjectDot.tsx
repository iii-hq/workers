import type { ProjectColor } from '@/lib/backend/projects'
import { cn } from '@/lib/utils'

/** Each colour's glyph tone, spelled out so Tailwind sees every class. */
const FILL: Record<ProjectColor, string> = {
  blue: 'bg-glyph-blue',
  purple: 'bg-glyph-purple',
  teal: 'bg-glyph-teal',
  green: 'bg-glyph-green',
  amber: 'bg-glyph-amber',
  rose: 'bg-glyph-rose',
}

/**
 * A project's colour as a dot (chat-projects §0.6): 8px beside its name, 16px
 * as a swatch in the project dialog. The colour is the glyph tone of the same
 * name, so it reads in both themes.
 */
export function ProjectDot({
  color,
  size = 'dot',
  className,
}: {
  color: ProjectColor
  size?: 'dot' | 'swatch'
  className?: string
}) {
  return (
    <span
      aria-hidden
      data-project-color={color}
      className={cn(
        'inline-block shrink-0 rounded-full',
        size === 'dot' ? 'size-2' : 'size-4',
        FILL[color],
        className,
      )}
    />
  )
}
