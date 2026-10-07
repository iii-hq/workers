import type { RefObject } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { useMentionProviders } from '@/lib/mentions/providers'
import { useMentionView } from '@/lib/mentions/views'
import { cn } from '@/lib/utils'
import { MentionGlyph, mentionColor, mentionIcon } from './appearance'
import { MentionPreviewCard } from './MentionPreviewCard'
import { useOpenMention } from './open-mention'

interface WorkerMentionPillProps {
  /** Provider token name (`kanban`). */
  name: string
  id: string
  /** Visible-selected state (the composer's decorator only). */
  selected?: boolean
  /** Click-target ref; the composer's decorator scopes selection with it. */
  pillRef?: RefObject<HTMLSpanElement | null>
  /**
   * Clicking opens the mention's target (rendered messages). The composer
   * leaves this off: there a click selects the pill.
   */
  openOnClick?: boolean
  /** Hovering shows the preview card (default). Off inside another control. */
  hoverCard?: boolean
}

function shortId(id: string): string {
  return id.length > 10 ? `${id.slice(0, 8)}…` : id
}

/**
 * An inline `@<name>(id="…")` mention: the item's icon in its color and its
 * name, resolved through the provider's get function (cached tab-wide).
 * While resolving it shows the provider and a short id; an id the provider
 * no longer knows reads as struck through. Hovering shows the preview card.
 */
export function WorkerMentionPill({
  name,
  id,
  selected,
  pillRef,
  openOnClick,
  hoverCard = true,
}: WorkerMentionPillProps) {
  const state = useMentionView(name, id)
  const provider = useMentionProviders().byName.get(name)
  const open = useOpenMention()
  const view = state.status === 'ready' ? state.view : null
  const icon = mentionIcon(view?.icon, provider?.icon)
  const color = mentionColor(view?.color, provider?.color)
  const missing =
    state.status === 'missing' || state.status === 'unknown-provider'
  const clickable = Boolean(openOnClick && view?.open)

  const title =
    state.status === 'unknown-provider'
      ? `No installed worker provides @${name} mentions`
      : state.status === 'missing'
        ? `@${name}: ${id} was not found`
        : state.status === 'error'
          ? `@${name}: ${state.message}`
          : undefined

  const pillClass = cn(
    'inline-flex min-w-0 max-w-[24rem] items-center gap-1 overflow-hidden px-1.5 h-[20px] -mt-[2px] rounded-xs align-middle text-[13px] text-ink select-none transition-colors',
    selected ? 'bg-surface-selected' : 'bg-surface',
    (pillRef || clickable) && 'cursor-pointer',
    clickable &&
      'hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
  )
  const pillData = {
    'data-worker-mention': name,
    'data-mention-id': id,
    'data-mention-state': state.status,
  }
  const content = (
    <>
      <MentionGlyph icon={icon} color={color} className="size-4" />
      {view ? (
        <>
          {view.hint ? (
            <span className="shrink-0 font-mono text-[12px] leading-none text-ink-faint">
              {view.hint}
            </span>
          ) : null}
          <span className="min-w-0 truncate font-sans font-medium leading-none">
            {view.label}
          </span>
        </>
      ) : (
        <span
          className={cn(
            'min-w-0 truncate font-mono text-[12px] leading-none text-ink-faint',
            missing && 'line-through',
          )}
        >
          {name} · {shortId(id)}
        </span>
      )}
    </>
  )

  /* In a rendered message the pill is a button onto its target; in the
     composer a click selects it (the decorator owns that), so it stays a
     plain inline element there. */
  const pill = clickable ? (
    <button
      {...pillData}
      type="button"
      title={title}
      onClick={(event) => {
        event.preventDefault()
        open(view?.open)
      }}
      className={pillClass}
    >
      {content}
    </button>
  ) : (
    <span
      {...pillData}
      ref={pillRef}
      contentEditable={false}
      title={title}
      className={pillClass}
    >
      {content}
    </span>
  )

  if (!view || !hoverCard) return pill
  return (
    <TooltipProvider delayDuration={450}>
      <Tooltip>
        <TooltipTrigger asChild>{pill}</TooltipTrigger>
        <TooltipContent
          side="top"
          className="w-[22rem] max-w-[calc(100vw-1rem)] bg-transparent p-0 shadow-none"
        >
          <MentionPreviewCard name={name} id={id} />
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}
