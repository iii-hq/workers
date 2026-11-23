import {
  Bot,
  Check,
  ChevronDown,
  ChevronRight,
  CircleArrowUp,
  Loader2,
  Lock,
  Plus,
  Settings2,
  SlidersHorizontal,
} from 'lucide-react'
import {
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import {
  Popover,
  PopoverContent,
  PopoverTitle,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import {
  type AgentEntry,
  type KitUpdateEntry,
  listAgents,
  listKitUpdates,
} from '@/lib/backend/directory-prompts'
import { serialRefresh } from '@/lib/backend/serial-refresh'
import { getIiiClient } from '@/lib/iii-client'
import {
  type KitInstall,
  onboardingKitsSnapshot,
  retryOnboardingKit,
  subscribeOnboardingKits,
} from '@/lib/onboarding/kits'
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
  /** Deterministic kit updates for stories/tests; omitted reads the Directory. */
  kitUpdates?: KitUpdateEntry[]
  /** An established conversation keeps its frozen profile; details remain accessible. */
  selectionLocked?: boolean
  details?: ReactNode
}

/**
 * The profile's glyph: a plain 16px icon on the first line of the row, ink
 * when chosen and muted otherwise. No tile and no identity tint: the name
 * and description carry the row, the check carries the choice.
 */
function AgentGlyph({
  icon,
  active = false,
}: {
  icon: string | null
  active?: boolean
}) {
  const Icon = (icon && SUBAGENT_ICON_COMPONENTS[icon as SubagentIcon]) || Bot
  return (
    <span aria-hidden className="flex h-5 shrink-0 items-center">
      <Icon
        className={cn(
          'size-4 transition-colors duration-100',
          active ? 'text-ink' : 'text-ink-faint',
        )}
      />
    </span>
  )
}

const KITS_ON_CHANGE = 'directory::kits::on-change'
const AGENTS_ON_CHANGE = 'directory::agents::on-change'

type CatalogRead = { ok: true; agents: AgentEntry[] } | { ok: false }

/**
 * The Directory's agent profiles, read on mount and again whenever its
 * profiles change (a kit apply, a download, an edit in Directory) and
 * whenever the picker opens. The composer can mount while the setup wizard
 * is still installing the template's kits (Deep Work's profiles land minutes
 * later, after Compose adds the kit's workers); a single read on mount kept
 * them out of the picker until a reload.
 *
 * A failed first read shows the error with Retry; a failed later read keeps
 * the list already shown.
 */
function useAgentCatalog(provided: AgentEntry[] | null | undefined) {
  const [entries, setEntries] = useState<AgentEntry[] | null>(null)
  const [error, setError] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const refreshRef = useRef<() => void>(() => {})
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  // biome-ignore lint/correctness/useExhaustiveDependencies: attempt explicitly retries a failed catalog request.
  useEffect(() => {
    if (provided !== undefined) return
    let cancelled = false
    let loaded = false
    const offs: Array<() => void> = []
    setError(false)
    void getIiiClient()
      .then((client) => {
        if (cancelled) return
        // One read in flight, so an older list never lands after a newer one.
        const { refresh: read, reset } = serialRefresh(
          (): Promise<CatalogRead> =>
            listAgents(client).then(
              (agents) => ({ ok: true, agents }),
              () => ({ ok: false }),
            ),
          (result) => {
            if (result.ok) {
              loaded = true
              setEntries(result.agents)
              setError(false)
            } else if (!loaded) {
              setEntries([])
              setError(true)
            }
          },
        )
        offs.push(reset)
        refreshRef.current = read
        // Per-mount id: the route chat and the dock can both show a picker.
        const fnId = `ade::agent-catalog::${instanceId}`
        try {
          offs.push(client.on(fnId, () => read()))
          offs.push(
            client.observe(
              { function_id: AGENTS_ON_CHANGE },
              `${fnId}::${client.browserId}`,
            ),
          )
        } catch {
          // The client refused the observer: reads on open still refresh.
        }
        read()
      })
      .catch(() => {
        if (!cancelled) {
          setEntries([])
          setError(true)
        }
      })
    return () => {
      cancelled = true
      refreshRef.current = () => {}
      for (const off of offs) off()
    }
  }, [provided, instanceId, attempt])

  return {
    catalog: provided === undefined ? entries : provided,
    error: provided === undefined && error,
    retry: () => setAttempt((value) => value + 1),
    refresh: () => refreshRef.current(),
  }
}

/**
 * Installed kits with a newer release, for the profiles they own. Read
 * again whenever the Directory applies a kit or its update check changes
 * the set, so an update applied from the panel this picker opened clears.
 */
function useKitUpdates(provided: KitUpdateEntry[] | undefined) {
  const [loaded, setLoaded] = useState<KitUpdateEntry[]>([])
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (provided !== undefined) return
    let cancelled = false
    const offs: Array<() => void> = []
    void getIiiClient()
      .then((client) => {
        if (cancelled) return
        // One read in flight, so an older list never lands after a newer one.
        // A failed read (no kits support in this Directory) changes nothing.
        const { refresh: read, reset } = serialRefresh(
          () => listKitUpdates(client),
          setLoaded,
        )
        offs.push(reset)
        // Per-mount id: the route chat and the dock can both show a picker.
        const fnId = `ade::kit-updates::${instanceId}`
        try {
          offs.push(
            client.on<{ op?: unknown }>(fnId, (change) => {
              if (change?.op === 'apply' || change?.op === 'updates') read()
            }),
          )
          offs.push(
            client.observe(
              {
                function_id: KITS_ON_CHANGE,
                filters: { 'input.op': { $in: ['apply', 'updates'] } },
              },
              `${fnId}::${client.browserId}`,
            ),
          )
        } catch {
          // The client refused the observer: the first read stands.
        }
        read()
      })
      .catch(() => {})
    return () => {
      cancelled = true
      for (const off of offs) off()
    }
  }, [provided, instanceId])

  return provided ?? loaded
}

/**
 * Deepwork is always offered, whether or not its kit is installed yet: the
 * setup wizard installs it in the background (it lands minutes after a new
 * project opens) and an older project may never have had it. Until the
 * Directory serves the profile, the picker shows this row with the kit's
 * install state, and a click installs the kit.
 */
const ALWAYS_LISTED = {
  id: 'deepwork',
  kit: 'iii/deepwork',
  name: 'Deepwork',
  description:
    'Use for big goals: plans tickets, runs agents in parallel, reviews the work.',
} as const

/** The kit's install, as the setup wizard or a click here started it. */
function useKitInstall(kit: string): KitInstall | undefined {
  const installs = useSyncExternalStore(
    subscribeOnboardingKits,
    onboardingKitsSnapshot,
    onboardingKitsSnapshot,
  )
  return installs.find(
    (entry) => entry.kit === kit || entry.kit.startsWith(`${kit}@`),
  )
}

function KitAgentRow({
  install,
  disabled,
  onInstall,
  onReview,
}: {
  install: KitInstall | undefined
  disabled: boolean
  onInstall: () => void
  onReview: () => void
}) {
  const state = install?.state
  const installing = state === 'installing'
  const action =
    state === 'review' ? 'Review' : state === 'failed' ? 'Try again' : 'Install'
  const label = installing
    ? `${ALWAYS_LISTED.name} agent is installing`
    : state === 'review'
      ? `Review the ${ALWAYS_LISTED.name} kit install`
      : `${action === 'Try again' ? 'Try installing' : 'Install'} the ${ALWAYS_LISTED.name} agent`
  return (
    <button
      type="button"
      aria-label={label}
      title={state === 'failed' ? install?.message : undefined}
      disabled={disabled || installing}
      className="group/kit flex w-full items-start gap-3 rounded-md px-2.5 py-2 text-left transition-colors duration-100 hover:bg-surface-hover/60 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none disabled:cursor-default disabled:hover:bg-transparent"
      onClick={state === 'review' ? onReview : onInstall}
    >
      <AgentGlyph icon="agent" />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-sm leading-5 font-medium text-ink">
          {ALWAYS_LISTED.name}
        </span>
        <span className="line-clamp-2 text-sm leading-5 text-pretty text-ink-faint">
          {ALWAYS_LISTED.description}
        </span>
      </span>
      <span className="flex h-5 shrink-0 items-center gap-1.5 text-sm text-ink-faint">
        {installing ? (
          <>
            <Loader2 aria-hidden className="size-4 motion-safe:animate-spin" />
            <span role="status">Installing…</span>
          </>
        ) : (
          <span className="rounded-md px-1.5 text-ink transition-colors duration-100 group-hover/kit:bg-surface-hover">
            {action}
          </span>
        )}
      </span>
    </button>
  )
}

export function AgentProfilePicker({
  systemPrompt,
  onSystemPromptChange,
  agentEntries,
  agentProfile,
  onAgentProfileChange,
  preselectDefaultAgent = false,
  kitUpdates,
  selectionLocked = false,
  details,
}: AgentProfilePickerProps) {
  const [open, setOpen] = useState(false)
  const updates = useKitUpdates(kitUpdates)
  const markerBase = useId()
  const updateFor = (entry: AgentEntry | undefined) =>
    entry?.source?.kit
      ? updates.find((update) => update.kit === entry.source?.kit)
      : undefined
  const detailsRef = useRef<HTMLDivElement>(null)
  const {
    catalog,
    error,
    retry,
    refresh: refreshCatalog,
  } = useAgentCatalog(agentEntries)
  const agents = (catalog ?? []).filter((entry) => !entry.hidden)
  const kitInstall = useKitInstall(ALWAYS_LISTED.kit)
  // A Directory without kit support answers the install with nothing.
  const [kitUnsupported, setKitUnsupported] = useState(false)
  // Clicked Install: select the profile once the Directory serves it.
  const [selectWhenInstalled, setSelectWhenInstalled] = useState(false)
  const showKitRow =
    agentEntries === undefined &&
    catalog !== null &&
    !error &&
    !kitUnsupported &&
    !(catalog ?? []).some((entry) => entry.id === ALWAYS_LISTED.id)
  const kitRowIndex = agents.findIndex(
    (entry) =>
      (entry.name.trim() || entry.id).localeCompare(ALWAYS_LISTED.name) > 0,
  )
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

  useEffect(() => {
    if (!selectWhenInstalled) return
    const installed = agents.find((entry) => entry.id === ALWAYS_LISTED.id)
    if (!installed) return
    setSelectWhenInstalled(false)
    if (!selectionLocked) selectRef.current(installed)
  }, [agents, selectWhenInstalled, selectionLocked])

  // An applied kit rings the agents doorbell; read once more in case the
  // observer was not registered yet.
  const kitInstalled = kitInstall?.state === 'installed'
  // biome-ignore lint/correctness/useExhaustiveDependencies: refresh is stable per mount; only the state flip matters.
  useEffect(() => {
    if (kitInstalled) refreshCatalog()
  }, [kitInstalled])

  const installKitAgent = () => {
    setSelectWhenInstalled(true)
    void retryOnboardingKit(ALWAYS_LISTED.kit).then(() => {
      if (
        !onboardingKitsSnapshot().some(
          (entry) => entry.kit === ALWAYS_LISTED.kit,
        )
      ) {
        setKitUnsupported(true)
        setSelectWhenInstalled(false)
      }
    })
  }
  const reviewKitAgent = () => {
    setOpen(false)
    requestPanelOpen({
      pageId: 'directory',
      context: {
        collection: 'kits',
        kit: ALWAYS_LISTED.kit,
        ...(kitInstall?.planId ? { plan_id: kitInstall.planId } : {}),
      },
    })
  }
  const kitRow = showKitRow ? (
    <KitAgentRow
      key={`kit:${ALWAYS_LISTED.kit}`}
      install={kitInstall}
      disabled={selectionLocked}
      onInstall={installKitAgent}
      onReview={reviewKitAgent}
    />
  ) : null

  const activeId = selectedId ?? preselect?.id
  const selected = agents.find((entry) => entry.id === activeId)
  // A kit with several profiles offers one action: on the chosen profile.
  const selectedUpdate = updateFor(selected)
  const name =
    agentProfile?.name ||
    selected?.name ||
    activeId ||
    (systemPrompt.choice === 'default'
      ? 'Choose agent'
      : systemPrompt.choice === 'custom'
        ? 'Custom instructions'
        : systemPrompt.choice.named)
  // Out of the menu, a profile called "Default" doesn't say what it is the
  // default of; the trigger names the noun.
  const label = /^default$/i.test(name.trim()) ? 'Default agent' : name

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
        if (next) refreshCatalog()
      }}
    >
      <PopoverTrigger
        aria-label={`Agent profile: ${label}`}
        title={label}
        className="composer-agent-trigger inline-flex min-w-0 items-center gap-1.5 rounded-lg px-2 py-2 font-inter text-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:ring-3 focus-visible:outline-none focus-visible:outline-ring"
      >
        <Bot aria-hidden className="size-4 shrink-0" />
        <span className="composer-agent-label max-w-32 truncate">{label}</span>
        {selectedUpdate ? (
          <span
            aria-hidden
            className={cn(
              'size-1.5 shrink-0 rounded-full',
              selectedUpdate.major ? 'bg-warn' : 'bg-accent',
            )}
          />
        ) : null}
        <ChevronDown aria-hidden className="size-4 shrink-0" />
      </PopoverTrigger>
      <PopoverContent
        keepMounted
        align="start"
        side="top"
        sideOffset={8}
        className="max-h-[min(32rem,var(--available-height))] w-[22rem] max-w-[calc(100vw-2rem)] gap-0 overflow-hidden p-0"
      >
        <div className="flex h-10 shrink-0 items-center justify-between gap-3 px-4">
          <PopoverTitle className="text-xs font-medium text-ink-faint">
            Agent
          </PopoverTitle>
          {selectionLocked ? (
            <span className="flex min-w-0 items-center gap-1.5 text-xs text-ink-faint">
              <Lock aria-hidden className="size-4 shrink-0" />
              <span className="truncate">Fixed for this chat</span>
            </span>
          ) : null}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-1.5 pb-1.5">
          {agentProfile && !selected ? (
            <div className="flex items-start gap-3 rounded-md bg-surface-hover px-2.5 py-2">
              <AgentGlyph icon={agentProfile.icon ?? null} active />
              <span className="min-w-0 flex-1 truncate text-sm leading-5 font-medium text-ink">
                {agentProfile.name}
              </span>
              <span className="flex h-5 shrink-0 items-center">
                <Check aria-hidden className="size-4 text-ink" />
              </span>
            </div>
          ) : null}
          {catalog === null ? (
            <p role="status" className="px-2.5 py-3 text-sm text-ink-faint">
              Loading agents…
            </p>
          ) : null}
          {error ? (
            <div
              role="status"
              className="flex items-center justify-between gap-2 px-2.5 py-2 text-sm text-ink-faint"
            >
              Unable to load agents.
              <button
                type="button"
                onClick={retry}
                className="rounded-md px-2 py-1 text-ink hover:bg-surface-hover focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
              >
                Retry
              </button>
            </div>
          ) : null}
          <fieldset
            className="flex min-w-0 flex-col gap-0.5"
            aria-label="Agent profiles"
          >
            {kitRowIndex === 0 ? kitRow : null}
            {agents.map((entry, index) => {
              const active = entry.id === activeId
              const update = active ? undefined : updateFor(entry)
              const markerId = `${markerBase}-${entry.id}`
              const description = entry.description.trim()
              return [
                index > 0 && index === kitRowIndex ? kitRow : null,
                <button
                  key={entry.id}
                  type="button"
                  aria-label={`Use ${entry.name.trim() || entry.id} agent profile`}
                  aria-pressed={active}
                  aria-describedby={update ? markerId : undefined}
                  disabled={selectionLocked}
                  className={cn(
                    'flex w-full items-start gap-3 rounded-md px-2.5 py-2 text-left transition-colors duration-100 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none disabled:cursor-default',
                    active
                      ? 'bg-surface-hover'
                      : 'hover:bg-surface-hover/60 disabled:hover:bg-transparent',
                  )}
                  onClick={() => {
                    select(entry)
                    setOpen(false)
                  }}
                >
                  <AgentGlyph icon={entry.icon} active={active} />
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="truncate text-sm leading-5 font-medium text-ink">
                      {entry.name.trim() || entry.id}
                    </span>
                    {description ? (
                      <span className="line-clamp-2 text-sm leading-5 text-pretty text-ink-faint">
                        {description}
                      </span>
                    ) : null}
                    {update ? (
                      <span
                        id={markerId}
                        className="flex items-center gap-1.5 text-xs text-ink-faint"
                      >
                        <CircleArrowUp
                          aria-hidden
                          className={cn(
                            'size-4 shrink-0',
                            update.major ? 'text-warn' : 'text-accent',
                          )}
                        />
                        <span>
                          {update.major ? 'Major update' : 'Update'}{' '}
                          <span className="tabular-nums">
                            {update.available}
                          </span>
                          <span className="sr-only"> available</span>
                        </span>
                      </span>
                    ) : null}
                  </span>
                  <span className="flex h-5 shrink-0 items-center">
                    <Check
                      aria-hidden
                      className={cn(
                        'size-4 text-ink transition-opacity duration-100',
                        active ? 'opacity-100' : 'opacity-0',
                      )}
                    />
                  </span>
                </button>,
              ]
            })}
            {kitRowIndex === -1 ? kitRow : null}
          </fieldset>
          {selectedUpdate ? (
            <button
              type="button"
              title={`${selectedUpdate.kit} ${selectedUpdate.current} → ${selectedUpdate.available} · opens in Directory`}
              className="mt-0.5 flex h-9 w-full shrink-0 items-center gap-3 rounded-md px-2.5 text-left text-sm text-ink transition-colors duration-100 hover:bg-surface-hover/60 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
              onClick={() => {
                setOpen(false)
                requestPanelOpen({
                  pageId: 'directory',
                  context: { collection: 'kits', kit: selectedUpdate.kit },
                })
              }}
            >
              <CircleArrowUp
                aria-hidden
                className={cn(
                  'size-4 shrink-0',
                  selectedUpdate.major ? 'text-warn' : 'text-accent',
                )}
              />
              <span className="min-w-0 flex-1 truncate">
                {selectedUpdate.major ? (
                  'Review major update'
                ) : (
                  <>
                    Review update{' '}
                    <span className="tabular-nums">
                      {selectedUpdate.available}
                    </span>
                  </>
                )}
                <span className="sr-only">, {selectedUpdate.kit} kit</span>
              </span>
            </button>
          ) : null}
          {details ? (
            <details className="group/details mt-0.5 shrink-0">
              <summary className="flex h-9 cursor-pointer list-none items-center gap-3 rounded-md px-2.5 text-sm text-ink transition-colors duration-100 hover:bg-surface-hover/60 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none [&::-webkit-details-marker]:hidden">
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
        </div>
        <div className="flex h-11 shrink-0 items-center justify-between gap-2 border-t border-popover-edge px-1.5">
          <button
            type="button"
            className="flex h-8 min-w-0 items-center gap-2 rounded-md px-2.5 text-sm text-ink-faint transition duration-150 ease-out hover:bg-surface-hover/60 hover:text-ink focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none motion-safe:enabled:active:scale-[0.96]"
            onClick={() => {
              setOpen(false)
              requestPanelOpen({
                pageId: 'directory',
                context: { collection: 'agents' },
              })
            }}
          >
            <Settings2 aria-hidden className="size-4 shrink-0" />
            <span className="truncate">Manage agents</span>
          </button>
          <button
            type="button"
            title="Choose instructions, a model, and skills."
            className="flex h-8 min-w-0 items-center gap-2 rounded-md px-2.5 text-sm text-ink-faint transition duration-150 ease-out hover:bg-surface-hover/60 hover:text-ink focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none motion-safe:enabled:active:scale-[0.96]"
            onClick={() => {
              setOpen(false)
              requestPanelOpen({
                pageId: 'directory',
                context: { collection: 'agents', action: 'create' },
              })
            }}
          >
            <Plus aria-hidden className="size-4 shrink-0" />
            <span className="truncate">New agent</span>
          </button>
        </div>
      </PopoverContent>
    </Popover>
  )
}
