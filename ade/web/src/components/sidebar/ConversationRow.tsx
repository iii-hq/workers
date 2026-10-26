import uiClasses from '@iii-dev/console-ui/ui-classes'
import { Bot, ChevronRight, Pencil, X } from 'lucide-react'
import type { CSSProperties } from 'react'
import { useRef, useState } from 'react'
import { SUBAGENT_ICON_COMPONENTS } from '@/components/chat/ActiveSubagentChips'
import { StatusDot } from '@/components/ui/StatusDot'
import { TriggerIcon } from '@/components/ui/TriggerIcon'
import { CONVERSATION_KIND_TAGS } from '@/lib/conversation-view'
import { cn } from '@/lib/utils'
import type {
  Conversation,
  ConversationKind,
  SubagentColor,
} from '@/types/chat'
import { MarqueeLabel } from './MarqueeLabel'
import { RenameConversationDialog } from './RenameConversationDialog'

const rowActionClassName = `${uiClasses.treeItemAction} pointer-coarse:min-h-12 pointer-coarse:min-w-12`

/**
 * Renaming is a desktop affordance — it pairs with double-click and F2.
 * `data-pointer="fine"` drops the pencil on coarse pointers, which also
 * returns touch rows to the single trailing action the tree recipe is built
 * around. No coarse hit area here on purpose: it is never shown to one.
 */
const renameActionClassName = uiClasses.treeItemAction

interface ConversationRowProps {
  conversation: Conversation
  active: boolean
  onSelect: () => void
  onRename: (title: string) => void
  onRemove: () => void
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

function formatRelative(ts: number): string {
  const delta = Date.now() - ts
  if (delta < 60_000) return 'now'
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m`
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h`
  return `${Math.floor(delta / 86_400_000)}d`
}

interface RowGlyph {
  /** The mark; absent, the row opens with a status dot instead. */
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
 * shows its origin (trigger bolt or the spawning agent's appearance); a
 * plain chat opens with a dot that carries its status (see `RowDot`), so
 * every label starts on the same column without a column of chat marks.
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
  if (depth > 0 && conversation.spawnedBy === 'trigger') {
    return { Icon: TriggerIcon, title: 'spawned by a trigger' }
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

/**
 * A plain chat's bullet. Ghost while idle, ink on the open conversation,
 * the accent pulsing while the session works and the alert tone on an
 * error, so the list shows at a glance which chats need a look.
 */
function RowDot({
  conversation,
  active,
  title,
}: {
  conversation: Conversation
  active: boolean
  title?: string
}) {
  if (conversation.status === 'working') {
    return <StatusDot tone="accent" pulse className="size-2" title="working" />
  }
  if (conversation.status === 'error') {
    return (
      <StatusDot
        tone="alert"
        className="size-2"
        title={conversation.statusReason ?? 'error'}
      />
    )
  }
  return (
    <StatusDot
      tone={active ? 'ink' : 'ghost'}
      className="size-2"
      title={title}
    />
  )
}

/** Conversation tree row with touch rename actions and explicit-exit focus recovery. */
export function ConversationRow({
  conversation,
  active,
  onSelect,
  onRename,
  onRemove,
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
  const rowRef = useRef<HTMLDivElement>(null)

  const glyph = resolveGlyph(conversation, depth)
  const kindTag = kind ? CONVERSATION_KIND_TAGS[kind] : undefined

  return (
    // biome-ignore lint/a11y/useSemanticElements: row hosts nested editing/caret/delete <button>s; using a real <button> here would nest interactive elements.
    <div
      ref={rowRef}
      role="button"
      tabIndex={0}
      aria-current={active ? 'page' : undefined}
      aria-label={`open ${conversation.title}`}
      className={uiClasses.treeItem}
      style={{ '--iii-ui-tree-depth': depth } as CSSProperties}
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
      <span
        className={uiClasses.treeItemIcon}
        data-color={glyph.color}
        title={glyph.title}
      >
        {glyph.Icon ? (
          <glyph.Icon aria-hidden />
        ) : (
          <RowDot
            conversation={conversation}
            active={active}
            title={glyph.title}
          />
        )}
      </span>
      {/* A long title fades at its right edge and slides into view while the
          row is hovered (MarqueeLabel), so the whole name is readable
          without widening the sidebar. */}
      <MarqueeLabel className={uiClasses.treeItemLabel}>
        {conversation.title}
      </MarqueeLabel>
      <span className={uiClasses.treeItemTrailing}>
        {/* A row that opens with a dot already shows its status there. */}
        {!glyph.Icon ? null : conversation.status === 'working' ? (
          <StatusDot tone="accent" pulse title="working" />
        ) : conversation.status === 'error' ? (
          <StatusDot
            tone="alert"
            title={conversation.statusReason ?? 'error'}
          />
        ) : null}
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
        <span className={uiClasses.treeItemMeta}>
          {formatRelative(conversation.updatedAt)}
        </span>
        <span className={uiClasses.treeItemActions}>
          <button
            type="button"
            className={renameActionClassName}
            data-pointer="fine"
            aria-label={`rename ${conversation.title}`}
            title="Rename conversation"
            onClick={(e) => {
              e.stopPropagation()
              setRenaming(true)
            }}
          >
            <Pencil aria-hidden />
          </button>
          <button
            type="button"
            className={rowActionClassName}
            data-tone="alert"
            aria-label={`delete ${conversation.title}`}
            onClick={(e) => {
              e.stopPropagation()
              onRemove()
            }}
          >
            <X aria-hidden />
          </button>
        </span>
      </span>
      {/* Last in the row: the hover actions widen the trailing cluster, and a
          caret before it would slide left under the pointer as they appear. */}
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
