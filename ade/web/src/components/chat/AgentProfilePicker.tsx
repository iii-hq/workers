import {
  ArrowUpRight,
  Bot,
  Check,
  ChevronDown,
  ChevronRight,
  Lock,
  Settings2,
  SlidersHorizontal,
} from 'lucide-react'
import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import {
  Popover,
  PopoverContent,
  PopoverTitle,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import { type AgentEntry, listAgents } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import { requestPanelOpen } from '@/lib/panel-context'
import { cn } from '@/lib/utils'
import type {
  AgentProfileChangeOptions,
  AgentProfileSnapshot,
  SubagentIcon,
} from '@/types/chat'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import { agentProfileFromEntry, agentToPreselect } from './agent-defaults'
import {
  agentIdFromSystemPrompt,
  type SystemPromptState,
  withAgentChoice,
} from './system-prompt-selection'

export interface AgentProfilePickerProps {
  systemPrompt: SystemPromptState
  onSystemPromptChange: (next: SystemPromptState) => void
  agentEntries?: AgentEntry[] | null
  agentProfile?: AgentProfileSnapshot
  onAgentProfileChange?: (
    next: AgentProfileSnapshot | undefined,
    options?: AgentProfileChangeOptions,
  ) => void
  preselectDefaultAgent?: boolean
  /** An established conversation keeps its frozen profile; details remain accessible. */
  selectionLocked?: boolean
  details?: ReactNode
}

/** The profile's glyph on a small tile; the chosen profile's tile fills with ink. */
function AgentGlyph({
  icon,
  active = false,
}: {
  icon: string | null
  active?: boolean
}) {
  const Icon = (icon && SUBAGENT_ICON_COMPONENTS[icon as SubagentIcon]) || Bot
  return (
    <span
      aria-hidden
      className={cn(
        'flex size-8 shrink-0 items-center justify-center rounded-lg transition-colors duration-100',
        active
          ? 'bg-ink text-panel'
          : 'bg-panel text-ink-faint ring-1 ring-edge ring-inset',
      )}
    >
      <Icon className="size-4" />
    </span>
  )
}

export function AgentProfilePicker({
  systemPrompt,
  onSystemPromptChange,
  agentEntries,
  agentProfile,
  onAgentProfileChange,
  preselectDefaultAgent = false,
  selectionLocked = false,
  details,
}: AgentProfilePickerProps) {
  const [open, setOpen] = useState(false)
  const detailsRef = useRef<HTMLDivElement>(null)
  const [entries, setEntries] = useState<AgentEntry[] | null>(null)
  const [error, setError] = useState(false)
  const [attempt, setAttempt] = useState(0)
  // biome-ignore lint/correctness/useExhaustiveDependencies: attempt explicitly retries a failed catalog request.
  useEffect(() => {
    if (agentEntries !== undefined) return
    let cancelled = false
    setError(false)
    void getIiiClient()
      .then(listAgents)
      .then((next) => {
        if (!cancelled) setEntries(next)
      })
      .catch(() => {
        if (!cancelled) {
          setEntries([])
          setError(true)
        }
      })
    return () => {
      cancelled = true
    }
  }, [agentEntries, attempt])

  const catalog = agentEntries === undefined ? entries : agentEntries
  const agents = (catalog ?? []).filter((entry) => !entry.hidden)
  const selectedId = agentProfile?.id ?? agentIdFromSystemPrompt(systemPrompt)
  const preselect =
    preselectDefaultAgent && !selectionLocked
      ? agentToPreselect(agents, selectedId, systemPrompt)
      : null
  const select = (entry: AgentEntry, options?: AgentProfileChangeOptions) => {
    if (onAgentProfileChange)
      onAgentProfileChange(agentProfileFromEntry(entry), options)
    else onSystemPromptChange(withAgentChoice(systemPrompt, entry.id))
  }
  const selectRef = useRef(select)
  selectRef.current = select
  // Apply the identity the picker displays before a first send can run.
  useLayoutEffect(() => {
    if (preselect) selectRef.current(preselect, { keepThinkingLevel: true })
  }, [preselect])

  const activeId = selectedId ?? preselect?.id
  const selected = agents.find((entry) => entry.id === activeId)
  const label =
    agentProfile?.name ||
    selected?.name ||
    activeId ||
    (systemPrompt.choice === 'default'
      ? 'Choose agent'
      : systemPrompt.choice === 'custom'
        ? 'Custom instructions'
        : systemPrompt.choice.named)

  return (
    <Popover
      open={open}
      onOpenChange={(next, event) => {
        // Worker settings own their dismissal and unsaved-change guards.
        // Their portals can use a separate overlay tree, so let an open
        // child handle Escape/outside presses before closing its parent.
        if (
          !next &&
          detailsRef.current?.querySelector(
            '[aria-haspopup][aria-expanded="true"]',
          )
        ) {
          event.cancel()
          return
        }
        setOpen(next)
      }}
    >
      <PopoverTrigger
        aria-label={`Agent profile: ${label}`}
        title={label}
        className="composer-agent-trigger inline-flex min-w-0 items-center gap-1.5 rounded-lg px-2 py-2 font-inter text-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-rule-focus"
      >
        <Bot aria-hidden className="size-4 shrink-0" />
        <span className="max-w-32 truncate">{label}</span>
        <ChevronDown aria-hidden className="size-4 shrink-0" />
      </PopoverTrigger>
      <PopoverContent
        keepMounted
        align="start"
        side="top"
        sideOffset={8}
        className="max-h-[min(30rem,var(--available-height))] w-80 max-w-[calc(100vw-2rem)] gap-0 overflow-y-auto overscroll-contain p-1.5"
      >
        <PopoverTitle className="px-2.5 pt-1.5 pb-2 text-xs font-medium text-ink-faint">
          Agent profile
        </PopoverTitle>
        {selectionLocked ? (
          <p className="flex items-center gap-2 px-2.5 pb-2 text-sm text-ink-faint">
            <Lock aria-hidden className="size-4 shrink-0" />
            Fixed for this conversation
          </p>
        ) : null}
        {agentProfile && !selected ? (
          <div className="flex items-center gap-3 rounded-lg bg-surface-hover/60 px-2.5 py-2">
            <AgentGlyph icon={null} active />
            <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">
              {agentProfile.name}
            </span>
            <Check aria-hidden className="size-4 shrink-0 text-ink" />
          </div>
        ) : null}
        {catalog === null ? (
          <p role="status" className="px-2.5 py-3 text-sm text-ink-faint">
            Loading agent profiles…
          </p>
        ) : null}
        {error && agentEntries === undefined ? (
          <div
            role="status"
            className="flex items-center justify-between gap-2 px-2.5 py-2 text-sm text-ink-faint"
          >
            Could not load agent profiles.
            <button
              type="button"
              onClick={() => setAttempt((value) => value + 1)}
              className="rounded-md px-2 py-1 text-ink hover:bg-surface-hover/60 focus-visible:outline-2 focus-visible:outline-rule-focus"
            >
              Retry
            </button>
          </div>
        ) : null}
        <fieldset
          className="flex min-w-0 flex-col gap-0.5"
          aria-label="Agent profiles"
        >
          {agents.map((entry) => {
            const active = entry.id === activeId
            return (
              <button
                key={entry.id}
                type="button"
                aria-label={`Use ${entry.name.trim() || entry.id} agent profile`}
                aria-pressed={active}
                disabled={selectionLocked}
                className={cn(
                  'group/agent flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left transition-colors duration-100 focus-visible:outline-2 focus-visible:outline-rule-focus disabled:cursor-default',
                  active
                    ? 'bg-surface-hover/60'
                    : 'hover:bg-surface-hover/60 disabled:hover:bg-transparent',
                )}
                onClick={() => {
                  select(entry)
                  setOpen(false)
                }}
              >
                <AgentGlyph icon={entry.icon} active={active} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium text-ink">
                    {entry.name.trim() || entry.id}
                  </span>
                  {entry.description.trim() ? (
                    <span className="line-clamp-3 text-sm leading-snug text-ink-faint">
                      {entry.description}
                    </span>
                  ) : null}
                </span>
                <Check
                  aria-hidden
                  className={cn(
                    'size-4 shrink-0 text-ink transition-opacity duration-100',
                    active ? 'opacity-100' : 'opacity-0',
                  )}
                />
              </button>
            )
          })}
        </fieldset>
        <div aria-hidden className="mx-1 my-1.5 h-px shrink-0 bg-edge" />
        <button
          type="button"
          title="Choose instructions, a model, and skills."
          className="group/configure flex h-9 w-full shrink-0 items-center gap-3 rounded-lg px-2.5 text-left text-sm text-ink transition-colors duration-100 hover:bg-surface-hover/60 focus-visible:outline-2 focus-visible:outline-rule-focus"
          onClick={() => {
            setOpen(false)
            requestPanelOpen({
              pageId: 'directory',
              context: { collection: 'agents', action: 'create' },
            })
          }}
        >
          <Settings2 aria-hidden className="size-4 shrink-0 text-ink-faint" />
          <span className="min-w-0 flex-1 truncate">Configure an agent…</span>
          <ArrowUpRight
            aria-hidden
            className="size-4 shrink-0 text-ink-faint opacity-0 transition-opacity duration-100 group-hover/configure:opacity-100 group-focus-visible/configure:opacity-100"
          />
        </button>
        {details ? (
          <details className="group/details shrink-0">
            <summary className="flex h-9 cursor-pointer list-none items-center gap-3 rounded-lg px-2.5 text-sm text-ink transition-colors duration-100 hover:bg-surface-hover/60 focus-visible:outline-2 focus-visible:outline-rule-focus [&::-webkit-details-marker]:hidden">
              <SlidersHorizontal
                aria-hidden
                className="size-4 shrink-0 text-ink-faint"
              />
              <span className="min-w-0 flex-1">Agent details</span>
              <ChevronRight
                aria-hidden
                className="size-4 shrink-0 text-ink-faint transition-transform duration-150 group-open/details:rotate-90"
              />
            </summary>
            <div
              ref={detailsRef}
              className="flex flex-wrap items-center gap-2 px-2.5 pt-1 pb-2"
            >
              {details}
            </div>
          </details>
        ) : null}
      </PopoverContent>
    </Popover>
  )
}
