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
import { useChatProjects } from '@/hooks/use-chat-projects'
import { useChatSettings } from '@/hooks/use-chat-settings'
import {
  type AutocompactSetting,
  autocompactOf,
  conversationAutocompact,
  formatTokens,
  inheritChoiceLabel,
  parseTokens,
  sideChatLine,
} from '@/lib/autocompact'
import { getDefaultBackend } from '@/lib/backend'
import {
  CLAUDE_CODE_PROVIDER,
  useClaudeAccounts,
} from '@/lib/backend/claude-accounts'
import { type ChatProject, NO_PROJECT } from '@/lib/backend/projects'
import { isSideChat } from '@/lib/chat-aside'
import { useChatCapabilities } from '@/lib/chat-capabilities'
import type { AccountResolution } from '@/lib/claude-accounts-words'
import {
  chatAccountResolution,
  lockedAccountOf,
} from '@/lib/conversation-account'
import {
  type ConversationDetailsDraft,
  claudeAccountOf,
  DESCRIPTION_COUNTER_FROM,
  DESCRIPTION_MAX,
  descriptionHistory,
  detailsDraftOf,
} from '@/lib/conversation-details'
import { effectiveProvider } from '@/lib/conversation-provider'
import { errText } from '@/lib/errors'
import { shortAge } from '@/lib/short-age'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'
import { ByAgentPill } from './ByAgentPill'
import { ConversationAccountRow } from './ConversationAccountRow'
import {
  type AutocompactChoice,
  ConversationAutocompactRow,
} from './ConversationAutocompactRow'

/**
 * A section the dialog can open on: the account chip's, and the
 * auto-compact chip's and the settings' overrides table's (phase 3).
 */
export type ConversationDetailsSection = 'account' | 'autocompact'

/** The size "Compact at…" starts at when the conversation had none. */
const AUTOCOMPACT_START = 160_000

function autocompactTextOf(setting: AutocompactSetting): string {
  return formatTokens(
    setting.mode === 'tokens' ? setting.tokens : AUTOCOMPACT_START,
  )
}

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
  > &
    Partial<Pick<Conversation, 'model' | 'parentId' | 'messages' | 'id'>>
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
  /** Open on this section, its control focused (the account chip). */
  initialSection?: ConversationDetailsSection
  /** A side chat: the account row is text only (its account is locked, D2). */
  sideChat?: boolean
  /**
   * The provider of the conversation's next turn as the chat reads it (the
   * account chip's); read from the conversation when absent.
   */
  provider?: string | null
  /**
   * A side chat's or a sub-agent's account as the chat resolved it (the
   * chip's and the limit card's); null for a conversation that chooses its
   * own. Read from the loaded conversations when absent.
   */
  lockedResolution?: AccountResolution | null
  /** The loaded conversations, where a sub-agent's parent is looked for. */
  conversations?: readonly Conversation[]
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
  initialSection,
  sideChat = false,
  provider,
  lockedResolution,
  conversations,
}: ConversationDetailsDialogProps) {
  const [draft, setDraft] = useState<ConversationDetailsDraft>(() =>
    detailsDraftOf(conversation),
  )
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const nameRef = useRef<HTMLInputElement>(null)
  const accountRef = useRef<HTMLButtonElement>(null)
  const autocompactRef = useRef<HTMLButtonElement>(null)
  const autocompactFieldRef = useRef<HTMLInputElement>(null)
  const ids = useId()
  // §4.3.5: the account row, for claude-code conversations on an engine
  // with the whole feature (§4.3.6).
  const realBackend = getDefaultBackend().id === 'real'
  const accounts = useClaudeAccounts(realBackend && open)
  const chatProjects = useChatProjects(realBackend && open)
  const showAccount =
    accounts.gate === 'full' &&
    (provider !== undefined ? provider : effectiveProvider(conversation)) ===
      CLAUDE_CODE_PROVIDER
  const locked = lockedAccountOf(conversation, sideChat)
  const projectList = projects ?? chatProjects.projects
  const resolution =
    lockedResolution !== undefined
      ? lockedResolution
      : showAccount && locked
        ? chatAccountResolution({
            conversation,
            sideChat,
            projects: projectList,
            global: accounts.claude?.global ?? null,
            conversations,
          })
        : null
  const openOnAccount = initialSection === 'account' && showAccount
  // §3.3.4: the Auto-compact row, on an engine with phase 3 (D168).
  const caps = useChatCapabilities(realBackend && open)
  const chatSettings = useChatSettings(realBackend && open)
  const showAutocompact = caps.autocompact
  const autocompactLocked = sideChat || isSideChat(conversation)
  const [autocompactChoice, setAutocompactChoice] =
    useState<AutocompactChoice>('inherit')
  const [autocompactText, setAutocompactText] = useState('')
  const [autocompactError, setAutocompactError] = useState<string | null>(null)
  const openOnAutocompact =
    initialSection === 'autocompact' && showAutocompact && !autocompactLocked
  const history = descriptionHistory(conversation.sessionMetadata)

  // Every open starts from the live values, with the name selected: most
  // edits replace it, so typing straight away is the fast path.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-seeded per open, not on every live update
  useEffect(() => {
    if (!open) return
    setDraft(detailsDraftOf(conversation))
    setError(null)
    setSaving(false)
    const own = autocompactOf(conversation)
    setAutocompactChoice(own.mode)
    setAutocompactText(autocompactTextOf(own))
    setAutocompactError(null)
    const frame = requestAnimationFrame(() => {
      if (openOnAutocompact && autocompactRef.current) {
        autocompactRef.current.focus()
        autocompactRef.current.scrollIntoView?.({ block: 'nearest' })
        return
      }
      if (openOnAccount && accountRef.current) {
        accountRef.current.focus()
        accountRef.current.scrollIntoView?.({ block: 'nearest' })
        return
      }
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

  /** The row's setting, or null while its size is not a size. */
  const autocompactDraft = (): AutocompactSetting | null => {
    if (autocompactChoice !== 'tokens') return { mode: autocompactChoice }
    const parsed = parseTokens(autocompactText)
    if (!parsed.ok) {
      setAutocompactError(parsed.error)
      autocompactFieldRef.current?.focus()
      return null
    }
    return { mode: 'tokens', tokens: parsed.tokens }
  }

  const submit = async () => {
    if (!canSave) return
    let next = draft
    if (showAutocompact && !autocompactLocked) {
      const autocompact = autocompactDraft()
      if (!autocompact) return
      next = { ...draft, autocompact }
    }
    setSaving(true)
    setError(null)
    try {
      await onSave(next)
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
        initialFocus={
          openOnAutocompact
            ? autocompactRef
            : openOnAccount
              ? accountRef
              : nameRef
        }
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

          {showAccount ? (
            <ConversationAccountRow
              value={
                draft.claudeAccount ??
                claudeAccountOf(conversation.sessionMetadata)
              }
              onChange={(claudeAccount) =>
                setDraft((current) => ({ ...current, claudeAccount }))
              }
              project={
                projectList.find((project) => project.id === draft.projectId) ??
                null
              }
              data={accounts}
              readOnly={locked !== null}
              resolution={resolution}
              triggerRef={accountRef}
            />
          ) : null}

          {showAutocompact ? (
            <ConversationAutocompactRow
              choice={autocompactChoice}
              text={autocompactText}
              onChoiceChange={(next) => {
                setAutocompactChoice(next)
                setAutocompactError(null)
              }}
              onTextChange={(next) => {
                setAutocompactText(next)
                setAutocompactError(null)
              }}
              inheritLabel={inheritChoiceLabel({
                project:
                  projectList.find(
                    (project) => project.id === draft.projectId,
                  ) ?? null,
                global: chatSettings.autocompact,
              })}
              error={autocompactError}
              lockedText={
                autocompactLocked
                  ? sideChatLine(
                      conversationAutocompact({
                        conversation,
                        projects: projectList,
                        global: chatSettings.autocompact,
                        conversations,
                      }).tokens,
                    )
                  : null
              }
              triggerRef={autocompactRef}
              fieldRef={autocompactFieldRef}
            />
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
