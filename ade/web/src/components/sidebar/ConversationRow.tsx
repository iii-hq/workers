import uiClasses from '@iii-dev/console-ui/ui-classes'
import {
  ArrowDown,
  ArrowUp,
  Bot,
  ChevronRight,
  Ellipsis,
  Pencil,
  Pin,
  PinOff,
  Trash2,
} from 'lucide-react'
import type { CSSProperties, DragEvent } from 'react'
import { useRef, useState } from 'react'
import { SUBAGENT_ICON_COMPONENTS } from '@/components/chat/ActiveSubagentChips'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { Hint } from '@/components/ui/Hint'
import { ObserverIcon } from '@/components/ui/ObserverIcon'
import { StatusDot } from '@/components/ui/StatusDot'
import { isAwaitingAnswer } from '@/lib/chat-activity'
import { CONVERSATION_KIND_TAGS } from '@/lib/conversation-view'
import { cn } from '@/lib/utils'
import type {
  Conversation,
  ConversationKind,
  SubagentColor,
} from '@/types/chat'
import { MarqueeLabel } from './MarqueeLabel'
import { RenameConversationDialog } from './RenameConversationDialog'

/* The menu trigger is the one action that survives on touch, so it carries
   the 48px coarse-pointer target the tree recipe is built around. */
const menuActionClassName = `${uiClasses.treeItemAction} pointer-coarse:min-h-12 pointer-coarse:min-w-12`

/* The pin shortcut is a hover affordance for a pointer; touch reaches Pin
   through the menu, so the shortcut drops off coarse pointers. A pinned
   chat's pin stays on show in the same place, hovered or not. */
const pinActionClassName = `${uiClasses.treeItemAction} conversation-row__pin`

/* Menu rows on the console's popover canon: Inter 14px, 36px tall, an
   icon then its label, generous inset so the list breathes. */
const MENU_ITEM_CLASS = 'gap-2.5 rounded-lg px-2.5 py-2 text-sm'

interface ConversationRowProps {
  conversation: Conversation
  active: boolean
  onSelect: () => void
  onRename: (title: string) => void
  onRemove: () => void
  /** Whether the reader pinned this chat to the top of the list. */
  pinned?: boolean
  onTogglePin?: () => void
  /**
   * Drag to rearrange (pinned chats): the row is draggable, and `dropEdge`
   * draws where a dragged row would land.
   */
  reorder?: {
    dragging: boolean
    dropEdge?: 'before' | 'after'
    onDragStart: (event: DragEvent<HTMLDivElement>) => void
    onDragOver: (event: DragEvent<HTMLDivElement>) => void
    onDrop: (event: DragEvent<HTMLDivElement>) => void
    onDragEnd: () => void
    /** The keyboard way to rearrange: one step up or down, from the menu. */
    onMove: (step: -1 | 1) => void
    canMoveUp: boolean
    canMoveDown: boolean
  }
  /** Tree nesting level; 0 = root. Drives the shared tree indent. */
  depth?: number
  /** True when this row has nested sub-agent rows (renders a toggle caret). */
  hasChildren?: boolean
  /** True when this row's subtree is currently collapsed. */
  treeCollapsed?: boolean
  /** Some row in the list has a caret: a row without one keeps its column empty. */
  caretColumn?: boolean
  /** Toggle this row's subtree (only wired when `hasChildren`). */
  onToggleTree?: () => void
  /**
   * The kind this row is listed under, when the list mixes kinds and the
   * row needs telling apart. Omitted (or `user`) draws no tag.
   */
  kind?: ConversationKind
}

const DAY_MS = 86_400_000

/** The row's age, one short unit wide: `now`, `5m`, `2h`, `3d`, then `2w`,
 *  `4mo`, `1y` so an old chat never reads as `143d`. */
function formatRelative(ts: number, now = Date.now()): string {
  const delta = now - ts
  if (delta < 60_000) return 'now'
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m`
  if (delta < DAY_MS) return `${Math.floor(delta / 3_600_000)}h`
  const days = Math.floor(delta / DAY_MS)
  if (days < 14) return `${days}d`
  if (days < 60) return `${Math.floor(days / 7)}w`
  if (days < 365) return `${Math.floor(days / 30)}mo`
  return `${Math.floor(days / 365)}y`
}

interface RowGlyph {
  /** The mark; absent, the row has no leading slot and the title sits flush. */
  Icon?: typeof Bot
  /** Glyph tone; `neutral`/absent keeps the ghost ink. */
  color?: SubagentColor
  /** Native tooltip naming the session's origin or profile. */
  title?: string
}

/**
 * Which mark opens the row. A sub-agent whose spawn picked an icon shows
 * it, so phases run on one profile stay apart (the harness already falls
 * back to the profile's icon when the spawn named none); otherwise a
 * selected agent profile owns the glyph at every depth; a spawned child
 * shows its origin (observer bolt or the spawning agent's appearance); a
 * plain chat has no mark at all, so its title starts at the row's edge and
 * the list reads as a list of names.
 */
function resolveGlyph(conversation: Conversation, depth: number): RowGlyph {
  const profile = conversation.agentProfile
  const appearance = conversation.subagentAppearance
  if (depth > 0 && conversation.spawnedBy === 'agent' && appearance?.icon) {
    return {
      Icon: SUBAGENT_ICON_COMPONENTS[appearance.icon],
      color: appearance.color ?? profile?.color,
      title: profile?.name ?? appearance.name,
    }
  }
  if (profile) {
    const chosen = profile.icon && SUBAGENT_ICON_COMPONENTS[profile.icon]
    // A root chat on a profile that picked no glyph keeps the quiet chat
    // mark: the robot fallback is for spawned agents, and a column of them
    // on the stock profile reads as noise, not identity.
    if (!chosen && depth === 0) {
      return { title: profile.name }
    }
    return {
      Icon: chosen || Bot,
      color: profile.color,
      title: profile.name,
    }
  }
  if (depth > 0 && conversation.spawnedBy === 'observer') {
    return { Icon: ObserverIcon, title: 'spawned by an observer' }
  }
  if (depth > 0 && conversation.spawnedBy === 'agent') {
    return {
      Icon:
        (appearance?.icon && SUBAGENT_ICON_COMPONENTS[appearance.icon]) || Bot,
      color: appearance?.color,
      title: appearance?.name ?? 'spawned by an agent',
    }
  }
  return {}
}

/** Conversation tree row: title, quiet metadata, and a row menu on hover. */
export function ConversationRow({
  conversation,
  active,
  onSelect,
  onRename,
  onRemove,
  pinned = false,
  onTogglePin,
  reorder,
  depth = 0,
  hasChildren = false,
  treeCollapsed = false,
  caretColumn = false,
  onToggleTree,
  kind,
}: ConversationRowProps) {
  // Renaming opens a centred dialog (the row is too narrow to show a whole
  // title while it is edited); the dialog hands focus back to the row.
  const [renaming, setRenaming] = useState(false)
  const [menuOpen, setMenuOpen] = useState(false)
  const rowRef = useRef<HTMLDivElement>(null)

  const glyph = resolveGlyph(conversation, depth)
  // An inactive sub-agent (done for now; an observation or the main agent
  // can start it again): a hollow mark and a quieter title, so the live ones
  // above it read first. A main chat at rest is not "done".
  const inactive =
    depth > 0 &&
    conversation.status === 'done' &&
    !isAwaitingAnswer(conversation)
  const kindTag = kind ? CONVERSATION_KIND_TAGS[kind] : undefined
  const PinIcon = pinned ? PinOff : Pin

  return (
    // biome-ignore lint/a11y/useSemanticElements: row hosts nested menu/caret/pin <button>s; using a real <button> here would nest interactive elements.
    <div
      ref={rowRef}
      role="button"
      tabIndex={0}
      aria-current={active ? 'page' : undefined}
      aria-label={`open ${conversation.title}`}
      className={uiClasses.treeItem}
      style={{ '--iii-ui-tree-depth': depth } as CSSProperties}
      data-menu-open={menuOpen || undefined}
      data-pinned={pinned || undefined}
      data-inactive={inactive || undefined}
      data-dragging={reorder?.dragging || undefined}
      data-drop={reorder?.dropEdge}
      draggable={reorder ? !renaming : undefined}
      onDragStart={reorder?.onDragStart}
      onDragOver={reorder?.onDragOver}
      onDrop={reorder?.onDrop}
      onDragEnd={reorder?.onDragEnd}
      onClick={() => !renaming && onSelect()}
      onDoubleClick={(e) => {
        if (!(e.target as HTMLElement).closest('button')) setRenaming(true)
      }}
      onKeyDown={(e) => {
        if (renaming || e.target !== e.currentTarget) return
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onSelect()
        } else if (e.key === 'F2') {
          e.preventDefault()
          setRenaming(true)
        }
      }}
    >
      {glyph.Icon ? (
        <span
          className={uiClasses.treeItemIcon}
          data-color={glyph.color}
          title={glyph.title}
        >
          <glyph.Icon aria-hidden />
        </span>
      ) : null}
      {/* The live state leads the title, where the eye starts reading the
          row: a pulsing dot while the agent works, an amber one while it
          waits on the reader's answer to its question, red on error, and a
          hollow ring while a sub-agent is inactive. */}
      {conversation.status === 'working' ? (
        <span className="conversation-row__status">
          <StatusDot tone="accent" pulse title="working" />
        </span>
      ) : isAwaitingAnswer(conversation) ? (
        <span
          className="conversation-row__status"
          data-awaiting-answer=""
          title="waiting for your answer"
        >
          <StatusDot tone="warn" />
          <span className="sr-only">waiting for your answer</span>
        </span>
      ) : conversation.status === 'error' ? (
        <span className="conversation-row__status">
          <StatusDot
            tone="alert"
            title={conversation.statusReason ?? 'error'}
          />
        </span>
      ) : inactive ? (
        <span
          className="conversation-row__status"
          data-inactive-mark=""
          title="inactive"
        >
          <span
            aria-hidden
            className="inline-block size-1.5 shrink-0 rounded-full ring-1 ring-ink-ghost"
          />
          <span className="sr-only">inactive</span>
        </span>
      ) : null}
      {/* A long title fades at its right edge and slides into view while the
          row is hovered (MarqueeLabel), so the whole name is readable
          without widening the sidebar. */}
      <MarqueeLabel className={uiClasses.treeItemLabel}>
        {conversation.title}
      </MarqueeLabel>
      {/* The trailing edge keeps every control where it was: the pin has its
          own place (on show while pinned, on hover otherwise), and the time
          and the ⋯ share one cell at the far right, the ⋯ taking the time's
          place on hover. Nothing slides under the pointer. */}
      <span className="conversation-row__trailing">
        {kindTag ? (
          <span
            className={cn(
              uiClasses.treeItemMeta,
              'rounded-xs bg-surface px-1 font-mono',
            )}
            title={kindTag.title}
          >
            {kindTag.label}
          </span>
        ) : null}
        {/* Each control stops its click so the row does not also open. */}
        {onTogglePin ? (
          <Hint label={pinned ? 'Unpin' : 'Pin'}>
            <button
              type="button"
              className={pinActionClassName}
              data-pointer={pinned ? undefined : 'fine'}
              data-pinned-mark={pinned ? '' : undefined}
              aria-label={`${pinned ? 'unpin' : 'pin'} ${conversation.title}`}
              onClick={(e) => {
                e.stopPropagation()
                onTogglePin()
              }}
              onDoubleClick={(e) => e.stopPropagation()}
              onKeyDown={(e) => e.stopPropagation()}
            >
              <Pin aria-hidden />
            </button>
          </Hint>
        ) : pinned ? (
          <Pin
            aria-hidden
            className="conversation-row__pinned size-4 shrink-0"
            data-pinned-mark=""
          />
        ) : null}
        <span className="conversation-row__slot">
          <span className={uiClasses.treeItemMeta}>
            {formatRelative(conversation.updatedAt)}
          </span>
          {/* biome-ignore lint/a11y/noStaticElementInteractions: a propagation boundary around the menu trigger, not a control. */}
          <span
            className="inline-flex"
            onClick={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => e.stopPropagation()}
          >
            <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
              <Hint label="More">
                <DropdownMenuTrigger
                  className={menuActionClassName}
                  aria-label={`more actions for ${conversation.title}`}
                >
                  <Ellipsis aria-hidden />
                </DropdownMenuTrigger>
              </Hint>
              <DropdownMenuContent
                align="end"
                sideOffset={6}
                className="conversation-row-menu min-w-44 p-1.5"
              >
                <DropdownMenuItem
                  className={MENU_ITEM_CLASS}
                  onClick={() => setRenaming(true)}
                >
                  <Pencil aria-hidden />
                  Rename
                </DropdownMenuItem>
                <DropdownMenuItem
                  className={MENU_ITEM_CLASS}
                  disabled={!onTogglePin}
                  onClick={() => onTogglePin?.()}
                >
                  <PinIcon aria-hidden />
                  {pinned ? 'Unpin' : 'Pin'}
                </DropdownMenuItem>
                {reorder ? (
                  <>
                    <DropdownMenuItem
                      className={MENU_ITEM_CLASS}
                      disabled={!reorder.canMoveUp}
                      onClick={() => reorder.onMove(-1)}
                    >
                      <ArrowUp aria-hidden />
                      Move up
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      className={MENU_ITEM_CLASS}
                      disabled={!reorder.canMoveDown}
                      onClick={() => reorder.onMove(1)}
                    >
                      <ArrowDown aria-hidden />
                      Move down
                    </DropdownMenuItem>
                  </>
                ) : null}
                <DropdownMenuSeparator className="my-1.5" />
                <DropdownMenuItem
                  variant="destructive"
                  className={MENU_ITEM_CLASS}
                  onClick={onRemove}
                >
                  <Trash2 aria-hidden />
                  Delete
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </span>
        </span>
      </span>
      {/* Last in the row, after the trailing cluster, so nothing that
          appears on hover can slide into the caret's place. */}
      {hasChildren ? (
        <button
          type="button"
          className={uiClasses.treeItemCaret}
          aria-expanded={!treeCollapsed}
          aria-label={
            treeCollapsed ? 'expand sub-agents' : 'collapse sub-agents'
          }
          onClick={(e) => {
            e.stopPropagation()
            onToggleTree?.()
          }}
        >
          <ChevronRight aria-hidden />
        </button>
      ) : caretColumn ? (
        <span aria-hidden className={uiClasses.treeItemCaret} data-slot="" />
      ) : null}
      <RenameConversationDialog
        open={renaming}
        title={conversation.title}
        onOpenChange={setRenaming}
        onSave={onRename}
        returnFocusTo={rowRef}
      />
    </div>
  )
}
