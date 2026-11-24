import type { RefObject } from 'react'
import { useEffect, useId, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import { Dialog } from '@/components/shadcn/dialog'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from '@/components/shadcn/select'
import { Switch } from '@/components/shadcn/switch'
import {
  FormDialogContent,
  FormDialogTitle,
  FormSelectTrigger,
} from '@/components/ui/FormDialog'
import { type ChatProject, NO_PROJECT } from '@/lib/backend/projects'
import {
  type ConversationDetailsDraft,
  DESCRIPTION_COUNTER_FROM,
  DESCRIPTION_MAX,
  descriptionHistory,
  detailsDraftOf,
} from '@/lib/conversation-details'
import { errText } from '@/lib/errors'
import { shortAge } from '@/lib/short-age'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'
import { ByAgentPill } from './ByAgentPill'

interface ConversationDetailsDialogProps {
  open: boolean
  conversation: Pick<
    Conversation,
    | 'title'
    | 'description'
    | 'descriptionBy'
    | 'descriptionAtMs'
    | 'projectId'
    | 'sessionMetadata'
  >
  onOpenChange: (open: boolean) => void
  /** Write the draft; a rejection keeps the dialog open with the error. */
  onSave: (draft: ConversationDetailsDraft) => Promise<void> | void
  /** Where focus returns on close (the row that opened it). */
  returnFocusTo?: RefObject<HTMLElement | null>
  /** The engine's projects for the Project select; absent hides the row. */
  projects?: readonly ChatProject[]
  /**
   * Show "Let the agent keep it up to date": the engine has
   * `harness::set-description` and Session descriptions is on.
   */
  agentSwitch?: boolean
  /** Clock for the relative times (tests). */
  now?: number
}

const FIELD =
  'w-full rounded-lg bg-muted px-3 text-sm text-foreground ring-1 ring-transparent outline-none transition-shadow duration-(--motion-duration-control) selection:bg-primary selection:text-primary-foreground placeholder:text-ink-ghost focus-visible:ring-ring/50'

/**
 * "Conversation details" (chat-projects §1.3.5), in place of the old Rename
 * dialog: the name, the one-line description with who wrote it, the agent's
 * switch, the recent descriptions and the project. F2 and a double-click on
 * the row open it with the name selected; Save writes everything at once.
 */
export function ConversationDetailsDialog({
  open,
  conversation,
  onOpenChange,
  onSave,
  returnFocusTo,
  projects,
  agentSwitch = false,
  now,
}: ConversationDetailsDialogProps) {
  const [draft, setDraft] = useState<ConversationDetailsDraft>(() =>
    detailsDraftOf(conversation),
  )
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const nameRef = useRef<HTMLInputElement>(null)
  const ids = useId()
  const history = descriptionHistory(conversation.sessionMetadata)

  // Every open starts from the live values, with the name selected: most
  // edits replace it, so typing straight away is the fast path.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-seeded per open, not on every live update
  useEffect(() => {
    if (!open) return
    setDraft(detailsDraftOf(conversation))
    setError(null)
    setSaving(false)
    const frame = requestAnimationFrame(() => {
      nameRef.current?.focus()
      nameRef.current?.select()
    })
    return () => cancelAnimationFrame(frame)
  }, [open])

  const canSave = draft.title.trim().length > 0 && !saving
  const byAgent =
    conversation.descriptionBy === 'agent' &&
    draft.description === (conversation.description ?? '')

  const editDescription = (text: string) =>
    setDraft((current) => ({
      ...current,
      // One line: a pasted newline becomes a space.
      description: text.replace(/[\r\n]+/g, ' ').slice(0, DESCRIPTION_MAX),
      // Editing it yourself turns the agent's switch off.
      agentKeepsIt: false,
    }))

  const submit = async () => {
    if (!canSave) return
    setSaving(true)
    setError(null)
    try {
      await onSave(draft)
      onOpenChange(false)
    } catch (err) {
      setError(errText(err))
    } finally {
      setSaving(false)
    }
  }

  const projectItems = projects
    ? [
        ...projects.map((project) => ({
          value: project.id,
          label: project.name,
        })),
        { value: NO_PROJECT, label: 'No project' },
      ]
    : []
  const projectValue = projectItems.some(
    (item) => item.value === draft.projectId,
  )
    ? draft.projectId
    : NO_PROJECT

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <FormDialogContent
        className="w-[min(30rem,calc(100vw-2rem))] [--dialog-max-width:30rem]"
        initialFocus={nameRef}
        finalFocus={returnFocusTo}
        // The dialog portals to the body but stays inside the row's React
        // tree, so pointer and key events would bubble to the row.
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.stopPropagation()}
      >
        <form
          className="flex max-h-[min(44rem,calc(100dvh-2rem))] flex-col gap-5 overflow-y-auto p-5"
          onSubmit={(e) => {
            e.preventDefault()
            void submit()
          }}
        >
          {/* Clear of the dialog's close button. */}
          <FormDialogTitle className="mr-8">
            Conversation details
          </FormDialogTitle>

          <div className="flex flex-col gap-1.5">
            <label
              htmlFor={`${ids}-name`}
              className="text-body font-medium text-foreground"
            >
              Name
            </label>
            <input
              ref={nameRef}
              id={`${ids}-name`}
              name="conversation-title"
              value={draft.title}
              onChange={(e) => {
                const title = e.currentTarget.value
                setDraft((current) => ({ ...current, title }))
              }}
              autoComplete="off"
              spellCheck={false}
              aria-describedby={`${ids}-name-hint`}
              className={cn(FIELD, 'h-10')}
            />
            <p id={`${ids}-name-hint`} className="text-xs text-ink-faint">
              Yours. The agent names a new conversation once, from its first
              message; it never renames one you named.
            </p>
          </div>

          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-2">
              <label
                htmlFor={`${ids}-description`}
                className="text-body font-medium text-foreground"
              >
                Description
              </label>
              {byAgent ? (
                <ByAgentPill atMs={conversation.descriptionAtMs} now={now} />
              ) : null}
              {draft.description.length >= DESCRIPTION_COUNTER_FROM ? (
                <span
                  className="ml-auto font-mono text-meta text-ink-faint tabular-nums"
                  aria-live="polite"
                >
                  {draft.description.length}/{DESCRIPTION_MAX}
                </span>
              ) : null}
            </div>
            <textarea
              id={`${ids}-description`}
              name="conversation-description"
              rows={2}
              value={draft.description}
              maxLength={DESCRIPTION_MAX}
              onChange={(e) => editDescription(e.currentTarget.value)}
              onKeyDown={(e) => {
                // One line of value: Enter saves, as in the name field.
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault()
                  void submit()
                }
              }}
              className={cn(FIELD, 'resize-none py-2 leading-5')}
            />
          </div>

          {agentSwitch ? (
            <div className="flex items-start gap-3">
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <label
                  htmlFor={`${ids}-agent`}
                  className="text-body font-medium text-foreground"
                >
                  Let the agent keep it up to date
                </label>
                <p id={`${ids}-agent-hint`} className="text-xs text-ink-faint">
                  It updates the description inside the turn it is already
                  running, never with a turn of its own. Editing it yourself
                  turns this off.
                </p>
              </div>
              <Switch
                id={`${ids}-agent`}
                aria-describedby={`${ids}-agent-hint`}
                checked={draft.agentKeepsIt}
                onCheckedChange={(checked) =>
                  setDraft((current) => ({
                    ...current,
                    agentKeepsIt: checked === true,
                  }))
                }
              />
            </div>
          ) : null}

          {history.length > 0 ? (
            <div className="flex flex-col gap-1">
              <span
                id={`${ids}-history`}
                className="text-body font-medium text-foreground"
              >
                Recent descriptions
              </span>
              <ul aria-labelledby={`${ids}-history`} className="flex flex-col">
                {history.map((entry) => (
                  <li key={`${entry.at_ms}:${entry.text}`}>
                    <button
                      type="button"
                      onClick={() => editDescription(entry.text)}
                      className="flex min-h-9 w-full items-center gap-2 rounded-md px-2 text-left text-sm hover:bg-surface-hover focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 pointer-coarse:min-h-11"
                    >
                      <span className="w-10 shrink-0 font-mono text-meta text-ink-faint">
                        {entry.by === 'agent' ? 'agent' : 'you'}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-ink">
                        {entry.text}
                      </span>
                      <span className="shrink-0 font-mono text-meta text-ink-faint tabular-nums">
                        {entry.at_ms ? shortAge(entry.at_ms, now) : ''}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}

          {projects ? (
            <div className="flex items-center gap-3">
              <label
                htmlFor={`${ids}-project`}
                className="flex-1 text-body font-medium text-foreground"
              >
                Project
              </label>
              <Select
                value={projectValue}
                onValueChange={(value) =>
                  setDraft((current) => ({
                    ...current,
                    projectId: String(value),
                  }))
                }
                items={projectItems}
              >
                <FormSelectTrigger
                  id={`${ids}-project`}
                  aria-label="Project"
                  className="h-9 min-w-44"
                >
                  <SelectValue />
                </FormSelectTrigger>
                <SelectContent align="end">
                  {projectItems.map((item) => (
                    <SelectItem key={item.value} value={item.value}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          ) : null}

          {error ? (
            <p role="alert" className="text-sm text-alert">
              {error}
            </p>
          ) : null}

          <div className="flex items-center justify-end gap-2">
            <Button
              type="button"
              variant="secondary"
              size="lg"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" size="lg" disabled={!canSave}>
              Save
            </Button>
          </div>
        </form>
      </FormDialogContent>
    </Dialog>
  )
}
