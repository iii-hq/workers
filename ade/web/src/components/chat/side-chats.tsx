import { Dialog as DialogPrimitive } from '@base-ui/react/dialog'
import {
  ArrowLeft,
  BookmarkPlus,
  EllipsisVertical,
  MessagesSquare,
  Sparkles,
  Trash2,
} from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { Sheet, SheetPortal } from '@/components/shadcn/sheet'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { IconButton } from '@/components/ui/IconButton'
import { useVisualViewportFit } from '@/hooks/use-visual-viewport-fit'
import type { SideChatLock } from '@/lib/chat-aside'
import { PortalScope } from '@/lib/ui-scope'
import type { ModelOption } from '@/types/chat'
import { CATALOG_MODEL_KEY_SEP } from '@/types/chat'
import {
  formatEffortLabel,
  formatModelLabel,
  modelFamily,
} from './model-picker-presentation'

/**
 * The engine side chat's pieces (chat-projects §2.3.2): the pane header, the
 * fork strip, the "⋯" menu, the phone page and the "Bring back" toast.
 * `ChatAside` puts them together around a `ChatView` in side mode.
 */

function clockTime(at: number): string {
  return new Date(at).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  })
}

/** How a side chat's lock reads: "Claude Opus 5.5 · High" (wide) and
 *  "Opus 5.5 · High" (phone), plus the catalog key to send with. */
export interface LockPresentation {
  label: string
  short: string
  /** The meta line above the composer: the label, then the account. */
  meta: string
  /** A catalog key for the composer's send gate; the send names none. */
  model?: string
}

export function lockPresentation(
  lock: SideChatLock | null,
  fallback: { model: string | null; thinkingLevel?: string | null },
  options: readonly ModelOption[],
): LockPresentation {
  const raw = lock?.model ?? fallback.model ?? ''
  const provider =
    lock?.provider ??
    (raw.includes(CATALOG_MODEL_KEY_SEP)
      ? raw.slice(0, raw.indexOf(CATALOG_MODEL_KEY_SEP))
      : undefined)
  const bare = raw.includes(CATALOG_MODEL_KEY_SEP)
    ? raw.slice(
        raw.indexOf(CATALOG_MODEL_KEY_SEP) + CATALOG_MODEL_KEY_SEP.length,
      )
    : raw
  const key = provider ? `${provider}${CATALOG_MODEL_KEY_SEP}${bare}` : bare
  const option =
    options.find((o) => o.id === key) ??
    options.find(
      (o) => o.id === bare || o.id.endsWith(`${CATALOG_MODEL_KEY_SEP}${bare}`),
    )
  // Phone: the composer's short form ("Opus 5.5"); wide: the family named
  // in front of it ("Claude Opus 5.5").
  const shortName = formatModelLabel(option?.label ?? (bare || 'Model'))
  const name =
    modelFamily(option?.id ?? key, option?.label ?? bare) === 'claude' &&
    !/^claude\b/i.test(shortName)
      ? `Claude ${shortName}`
      : shortName
  const effort = formatEffortLabel(
    lock ? (lock.thinkingLevel ?? '') : (fallback.thinkingLevel ?? ''),
  )
  const label = effort ? `${name} · ${effort}` : name
  const short = effort ? `${shortName} · ${effort}` : shortName
  const meta = lock?.account ? `${label} · ${lock.account}` : label
  return { label, short, meta, ...(key ? { model: option?.id ?? key } : {}) }
}

/** The fork strip's words: where it was forked, and that it reuses the cache. */
export function forkStripText({
  at,
  fromUser,
  parentTitle,
  compact,
}: {
  at: number
  fromUser: boolean
  parentTitle: string
  compact: boolean
}): string {
  const point = fromUser ? 'your message' : "the agent's reply"
  return compact
    ? `Forked from ${point} at ${clockTime(at)} · same model and effort, cached`
    : `Knows the conversation up to ${point} at ${clockTime(at)} and runs on its model and effort, so it reuses its cache. Nothing here goes into ${parentTitle}.`
}

export function SideChatForkStrip({ text }: { text: string }) {
  return (
    <p
      data-side-chat-strip=""
      className="mx-2 mb-1 flex shrink-0 items-start gap-2 rounded-lg bg-surface px-3 py-2 font-inter text-xs text-ink-faint"
    >
      <MessagesSquare aria-hidden className="mt-px size-4 shrink-0" />
      <span className="min-w-0">{text}</span>
    </p>
  )
}

/** The "⋯" menu: Summarize, Keep, Delete… (the last confirms). */
export function SideChatMenu({
  label = 'Side chat menu',
  busy,
  onSummarize,
  onKeep,
  onDelete,
  triggerClassName,
}: {
  label?: string
  busy?: boolean
  onSummarize: () => void
  onKeep: () => void
  onDelete: () => void
  triggerClassName?: string
}) {
  const [confirm, setConfirm] = useState(false)
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label={label}
          data-side-chat-menu=""
          className={
            triggerClassName ??
            'flex size-7 shrink-0 items-center justify-center rounded-md text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50'
          }
        >
          <EllipsisVertical aria-hidden className="size-4" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-56">
          <DropdownMenuItem disabled={busy} onClick={onSummarize}>
            <Sparkles aria-hidden />
            Summarize for the main chat
          </DropdownMenuItem>
          <DropdownMenuItem disabled={busy} onClick={onKeep}>
            <BookmarkPlus aria-hidden />
            Keep as a conversation
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            variant="destructive"
            onClick={() => setConfirm(true)}
          >
            <Trash2 aria-hidden />
            Delete…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <ConfirmSideChatDelete
        open={confirm}
        onOpenChange={setConfirm}
        onConfirm={onDelete}
      />
    </>
  )
}

/** "Delete this side chat?" with "Cancel" and "Delete" (§2.3.2). */
export function ConfirmSideChatDelete({
  open,
  onOpenChange,
  onConfirm,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: () => void
}) {
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Delete this side chat?"
      confirmLabel="Delete"
      cancelLabel="Cancel"
      tone="danger"
      onConfirm={() => {
        onOpenChange(false)
        onConfirm()
      }}
    />
  )
}

/** The pane's header while a side chat is open: back to Context, the title, ⋯. */
export function SideChatHeader({
  title,
  onBack,
  menu,
}: {
  title: string
  onBack: () => void
  menu: ReactNode
}) {
  return (
    <div
      className="flex min-w-0 flex-1 items-center gap-1 self-stretch font-inter"
      data-side-chat-header=""
    >
      <button
        type="button"
        onClick={onBack}
        aria-label="Back to context"
        className="flex h-7 shrink-0 items-center gap-1 rounded-md px-1.5 text-body text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
      >
        <ArrowLeft aria-hidden className="size-4" />
        Context
      </button>
      <MessagesSquare aria-hidden className="size-4 shrink-0 text-ink-faint" />
      <h2
        className="min-w-0 flex-1 truncate text-body font-medium text-ink"
        title={title}
      >
        {title}
      </h2>
      {menu}
    </div>
  )
}

/**
 * A side chat on a phone: a page over the conversation (§2.1), with
 * "Sidechat of <conversation>" and the title, a back arrow and the menu.
 * It follows the visual viewport, so a raised keyboard shrinks it.
 */
export function SideChatPage({
  open,
  onBack,
  parentTitle,
  title,
  menu,
  children,
}: {
  open: boolean
  onBack: () => void
  parentTitle: string
  title: string
  menu: ReactNode
  children: ReactNode
}) {
  return (
    <Sheet
      open={open}
      onOpenChange={(next) => {
        if (!next) onBack()
      }}
    >
      <SheetPortal>
        <PortalScope>
          <SideChatPageContent
            onBack={onBack}
            parentTitle={parentTitle}
            title={title}
            menu={menu}
          >
            {children}
          </SideChatPageContent>
        </PortalScope>
      </SheetPortal>
    </Sheet>
  )
}

function SideChatPageContent({
  onBack,
  parentTitle,
  title,
  menu,
  children,
}: {
  onBack: () => void
  parentTitle: string
  title: string
  menu: ReactNode
  children: ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)
  useVisualViewportFit(ref)
  return (
    <DialogPrimitive.Popup
      ref={ref}
      aria-describedby={undefined}
      aria-label={`Sidechat of ${parentTitle}`}
      data-side-chat-page=""
      className="fixed inset-0 z-50 flex h-dvh w-screen flex-col bg-background pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] font-sans text-ink transition-transform duration-[var(--motion-duration-panel)] ease-[var(--motion-ease-enter)] focus:outline-none data-ending-style:translate-x-full data-starting-style:translate-x-full motion-reduce:transition-none"
    >
      <header className="flex h-14 shrink-0 items-center gap-1 px-1">
        <IconButton
          label={`Back to ${parentTitle}`}
          className="size-12 shrink-0"
          onClick={onBack}
        >
          <ArrowLeft aria-hidden />
        </IconButton>
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="flex min-w-0 items-center gap-1 text-xs text-ink-faint">
            <MessagesSquare aria-hidden className="size-4 shrink-0" />
            <span className="truncate">Sidechat of {parentTitle}</span>
          </span>
          <span className="truncate text-base font-medium text-ink">
            {title}
          </span>
        </div>
        {menu}
      </header>
      <div className="flex min-h-0 flex-1 flex-col">{children}</div>
    </DialogPrimitive.Popup>
  )
}

const TOAST_MS = 5000

/** "Added to the <conversation> composer", with "Open" on a phone. */
export function SideChatToast({
  toast,
  onDismiss,
}: {
  toast: {
    seq: number
    text: string
    action?: { label: string; run: () => void }
  } | null
  onDismiss: () => void
}) {
  const dismissRef = useRef(onDismiss)
  dismissRef.current = onDismiss
  const seq = toast?.seq
  useEffect(() => {
    if (seq === undefined) return
    const timer = window.setTimeout(() => dismissRef.current(), TOAST_MS)
    return () => window.clearTimeout(timer)
  }, [seq])
  if (!toast || typeof document === 'undefined') return null
  return createPortal(
    <PortalScope>
      <div
        role="status"
        data-side-chat-toast=""
        className="fixed bottom-[calc(env(safe-area-inset-bottom)+5.5rem)] left-1/2 z-[60] flex max-w-[calc(100vw-2rem)] -translate-x-1/2 items-center gap-2 rounded-xl bg-popover py-2 pr-2 pl-3.5 font-inter text-sm text-ink shadow-floating ring-1 ring-popover-edge"
      >
        <span className="min-w-0 truncate">{toast.text}</span>
        {toast.action ? (
          <button
            type="button"
            className="h-11 shrink-0 rounded-lg px-3 text-sm font-medium text-ink hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 pointer-fine:h-8"
            onClick={() => {
              toast.action?.run()
              onDismiss()
            }}
          >
            {toast.action.label}
          </button>
        ) : null}
      </div>
    </PortalScope>,
    document.body,
  )
}
