import { Bot, Check, Plus } from 'lucide-react'
import { useEffect, useState } from 'react'
import { CopyCommandButton } from '@/components/chat/sandbox/terminal/CopyCommandButton'
import { Terminal } from '@/components/chat/sandbox/terminal/Terminal'
import { Button } from '@/components/ui/Button'
import { Wordmark } from '@/components/ui/Wordmark'
import type { InstallStage } from '@/hooks/use-harness-status'
import { type AgentEntry, listAgents } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import { requestPanelOpen } from '@/lib/panel-context'
import { normalizeErrorMessage } from '@/lib/providers'
import { cn } from '@/lib/utils'
import type {
  AgentProfileSnapshot,
  SubagentColor,
  SubagentIcon,
} from '@/types/chat'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import { DirectoryPicker, type WorktreePickerOptions } from './DirectoryPicker'
import {
  agentIdFromSystemPrompt,
  type SystemPromptState,
  withAgentChoice,
} from './system-prompt-selection'
import './EmptyState.css'

/**
 * The chat empty state, as a small set of presentational variants:
 *
 *   - `ready`          — harness present + a model available (the hero copy)
 *   - `no-provider`    — harness present, no model configured -> configure CTA
 *   - `no-harness`     — harness worker missing -> install CTA + cli hint
 *   - `installing`     — `compose::add` in flight -> live console
 *   - `install-failed` — add failed -> console + retry
 *
 * `MessageList` derives the variant from `ConversationsContext` (harness
 * presence + model catalog) and passes the callbacks. The ready state loads
 * agent profiles from Directory unless a deterministic catalog is supplied by a
 * story or test.
 */

export type EmptyStateVariant =
  | 'ready'
  | 'no-provider'
  | 'no-harness'
  | 'installing'
  | 'install-failed'

export interface EmptyStateProps {
  variant: EmptyStateVariant
  density?: 'route' | 'dock'
  /** Console lines for `installing` / `install-failed`. */
  stages?: InstallStage[]
  /** Surfaced under the console when an add fails with no stage detail. */
  errorMessage?: string | null
  /** `no-harness` primary CTA. */
  onInstallHarness?: () => void
  /** `install-failed` retry CTA. */
  onRetryInstall?: () => void
  /** `no-provider` CTA (opens the model/provider picker). */
  onConfigureProvider?: () => void
  /** `ready` project context and shared directory-picker behavior. */
  workingDir?: string | null
  onWorkingDirChange?: (next: string) => void
  workingDirError?: string | null
  defaultWorkingDir?: string | null
  worktreePicker?: WorktreePickerOptions
  /** Session instructions selected before the first message. */
  systemPrompt?: SystemPromptState
  onSystemPromptChange?: (next: SystemPromptState) => void
  /** Optional deterministic catalog for stories/tests; omitted loads Directory. */
  agentEntries?: AgentEntry[] | null
  /** Frozen identity/configuration for the selected Directory agent. */
  agentProfile?: AgentProfileSnapshot
  onAgentProfileChange?: (next: AgentProfileSnapshot | undefined) => void
}

const HARNESS_INSTALL_COMMAND = 'iii trigger compose::add worker=harness'

const HEADING_CLASS =
  'text-balance font-sans text-3xl font-semibold tracking-tight text-ink'
const BODY_CLASS = 'text-pretty font-sans text-base/7 text-ink-faint'

export function EmptyState({
  variant,
  density = 'route',
  stages = [],
  errorMessage,
  onInstallHarness,
  onRetryInstall,
  onConfigureProvider,
  workingDir,
  onWorkingDirChange,
  workingDirError,
  defaultWorkingDir,
  worktreePicker,
  systemPrompt,
  onSystemPromptChange,
  agentEntries,
  agentProfile,
  onAgentProfileChange,
}: EmptyStateProps) {
  const emptyPad = density === 'dock' ? 'px-3 sm:px-4' : 'px-3 sm:px-6 lg:px-9'
  const eyebrow = variant === 'no-provider' ? 'New session' : 'Setup'

  return (
    <div
      className={cn(
        'flex-1 min-h-0 overflow-y-auto flex justify-center',
        emptyPad,
      )}
    >
      <div
        className={cn(
          'my-auto flex w-full flex-col py-6',
          variant === 'ready'
            ? 'max-w-[680px] items-center gap-5 text-center'
            : 'max-w-[520px] gap-6',
        )}
      >
        {variant === 'ready' ? (
          <ReadyBody
            workingDir={workingDir}
            onWorkingDirChange={onWorkingDirChange}
            workingDirError={workingDirError}
            defaultWorkingDir={defaultWorkingDir}
            worktreePicker={worktreePicker}
            systemPrompt={systemPrompt}
            onSystemPromptChange={onSystemPromptChange}
            agentEntries={agentEntries}
            agentProfile={agentProfile}
            onAgentProfileChange={onAgentProfileChange}
          />
        ) : (
          <div className="font-sans text-base font-medium text-ink-faint sm:text-sm">
            {eyebrow}
          </div>
        )}
        {variant === 'no-provider' ? (
          <NoProviderBody onConfigureProvider={onConfigureProvider} />
        ) : null}
        {variant === 'no-harness' ? (
          <NoHarnessBody onInstallHarness={onInstallHarness} />
        ) : null}
        {variant === 'installing' || variant === 'install-failed' ? (
          <InstallingBody
            failed={variant === 'install-failed'}
            stages={stages}
            errorMessage={errorMessage}
            onRetryInstall={onRetryInstall}
          />
        ) : null}
      </div>
    </div>
  )
}

/* ---------------- ready (welcome hero) ---------------- */

function ReadyBody({
  workingDir,
  onWorkingDirChange,
  workingDirError,
  defaultWorkingDir,
  worktreePicker,
  systemPrompt,
  onSystemPromptChange,
  agentEntries,
  agentProfile,
  onAgentProfileChange,
}: {
  workingDir?: string | null
  onWorkingDirChange?: (next: string) => void
  workingDirError?: string | null
  defaultWorkingDir?: string | null
  worktreePicker?: WorktreePickerOptions
  systemPrompt?: SystemPromptState
  onSystemPromptChange?: (next: SystemPromptState) => void
  agentEntries?: AgentEntry[] | null
  agentProfile?: AgentProfileSnapshot
  onAgentProfileChange?: (next: AgentProfileSnapshot | undefined) => void
}) {
  const projectName = workingDir
    ? (workingDir.split('/').filter(Boolean).at(-1) ?? workingDir)
    : 'a project'

  return (
    <>
      <Wordmark appearance="inset" />
      <div className="flex w-full max-w-full flex-col items-center gap-2.5">
        <div className="max-w-full font-sans text-xl font-medium text-ink-faint sm:text-lg">
          What should we build in{' '}
          {onWorkingDirChange ? (
            <DirectoryPicker
              value={workingDir ?? null}
              onChange={onWorkingDirChange}
              externalError={workingDirError}
              defaultDir={defaultWorkingDir}
              worktrees={worktreePicker}
              triggerAppearance="inline"
              emptyLabel="a project"
              className="max-w-[min(18rem,70vw)] align-baseline"
            />
          ) : (
            <span className="font-medium text-ink">{projectName}</span>
          )}
          {'?'}
        </div>
        {systemPrompt && onSystemPromptChange ? (
          <SessionSetupControls
            systemPrompt={systemPrompt}
            onSystemPromptChange={onSystemPromptChange}
            agentEntries={agentEntries}
            agentProfile={agentProfile}
            onAgentProfileChange={onAgentProfileChange}
          />
        ) : null}
      </div>
    </>
  )
}

function SessionSetupControls({
  systemPrompt,
  onSystemPromptChange,
  agentEntries,
  agentProfile,
  onAgentProfileChange,
}: {
  systemPrompt: SystemPromptState
  onSystemPromptChange: (next: SystemPromptState) => void
  agentEntries?: AgentEntry[] | null
  agentProfile?: AgentProfileSnapshot
  onAgentProfileChange?: (next: AgentProfileSnapshot | undefined) => void
}) {
  const selectedAgentId =
    agentProfile?.id ?? agentIdFromSystemPrompt(systemPrompt)
  const catalog = useAgentCatalog(agentEntries)
  const agents = catalog.entries ?? []

  return (
    <section aria-label="session setup" className="w-full max-w-[40rem]">
      <AgentGallery
        entries={agents}
        loading={catalog.entries === null}
        error={catalog.error}
        selectedId={selectedAgentId}
        onSelect={(entry) => {
          const profile: AgentProfileSnapshot = {
            id: entry.id,
            name: entry.name.trim() || entry.id,
            ...(entry.model ? { model: entry.model } : {}),
            ...(entry.reasoning_effort
              ? { reasoningEffort: entry.reasoning_effort }
              : {}),
            ...(entry.icon ? { icon: entry.icon as SubagentIcon } : {}),
            ...(entry.color ? { color: entry.color as SubagentColor } : {}),
          }
          if (onAgentProfileChange) onAgentProfileChange(profile)
          else onSystemPromptChange(withAgentChoice(systemPrompt, entry.id))
        }}
      />
    </section>
  )
}

function useAgentCatalog(provided: AgentEntry[] | null | undefined): {
  entries: AgentEntry[] | null
  error: boolean
} {
  const [entries, setEntries] = useState<AgentEntry[] | null>(null)
  const [error, setError] = useState(false)

  useEffect(() => {
    if (provided !== undefined) return
    let cancelled = false
    setError(false)
    void getIiiClient()
      .then((client) => listAgents(client))
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
  }, [provided])

  return provided === undefined
    ? { entries, error }
    : { entries: provided, error: false }
}

function AgentGallery({
  entries,
  loading,
  error,
  selectedId,
  onSelect,
}: {
  entries: AgentEntry[]
  loading: boolean
  error: boolean
  selectedId: string | null
  onSelect: (entry: AgentEntry) => void
}) {
  return (
    <div className="pb-3 text-left">
      {loading ? (
        <div
          className="grid grid-cols-1 gap-3 p-1 @lg:grid-cols-2 @3xl:grid-cols-3"
          aria-hidden
        >
          <div className="h-40 rounded-lg bg-panel-raised shadow-raised" />
          <div className="h-40 rounded-lg bg-panel-raised shadow-raised" />
          <div className="h-40 rounded-lg bg-panel-raised shadow-raised" />
        </div>
      ) : error ? (
        <p className="rounded-md bg-panel-raised px-3 py-4 font-sans text-base text-ink-faint shadow-raised sm:text-sm">
          Agent profiles are unavailable right now. Try again in a moment.
        </p>
      ) : (
        <ul
          // biome-ignore lint/a11y/noRedundantRoles: keep list semantics when CSS resets remove markers.
          role="list"
          className={cn(
            'grid grid-cols-1 gap-3 p-1 @lg:grid-cols-2',
            entries.length >= 2 && '@3xl:grid-cols-3',
          )}
        >
          {entries.map((entry) => (
            <li key={entry.id} className="min-w-0">
              <AgentChoiceCard
                entry={entry}
                selected={selectedId === entry.id}
                onSelect={() => onSelect(entry)}
              />
            </li>
          ))}
          <li className="min-w-0">
            <CreateAgentCard />
          </li>
        </ul>
      )}
    </div>
  )
}

function CreateAgentCard() {
  return (
    <button
      type="button"
      aria-label="Create a new agent profile"
      onClick={() =>
        requestPanelOpen({
          pageId: 'directory',
          context: { collection: 'agents', action: 'create' },
        })
      }
      className="group/create flex h-full w-full cursor-pointer flex-col items-start justify-between gap-6 rounded-lg bg-surface/30 p-4 text-left hover:bg-surface-hover/30 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent @lg:min-h-40"
    >
      <div className="flex min-w-0 flex-col gap-1">
        <div className="font-sans text-base font-medium text-ink-faint sm:text-sm">
          Create a new agent
        </div>
        <p className="text-pretty font-sans text-base/6 text-ink-ghost sm:text-sm/5">
          Save a reusable set of instructions, a model, and skills.
        </p>
      </div>
      <div className="flex items-center gap-1.5 font-sans text-base font-medium text-ink-faint group-hover/create:text-ink sm:text-sm">
        <Plus aria-hidden className="size-4 h-lh shrink-0" />
        <span>Create agent profile</span>
      </div>
    </button>
  )
}

function AgentChoiceCard({
  entry,
  selected,
  onSelect,
}: {
  entry: AgentEntry
  selected: boolean
  onSelect: () => void
}) {
  const Icon =
    (entry.icon && SUBAGENT_ICON_COMPONENTS[entry.icon as SubagentIcon]) || Bot
  const title = entry.name.trim() || entry.id
  const color = (entry.color ?? 'neutral') as SubagentColor

  return (
    <button
      type="button"
      aria-label={`Use ${title} agent profile`}
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(
        'group/agent relative flex h-full w-full cursor-pointer flex-col rounded-lg bg-panel-raised p-4 text-left shadow-raised ring-1 ring-rule-2 transition-[transform,box-shadow] duration-150 hover:-translate-y-px hover:shadow-floating focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent @lg:min-h-40',
        selected && 'ring-2 ring-accent',
      )}
    >
      {selected ? (
        <span className="absolute right-3 top-3 flex size-5 items-center justify-center rounded-full bg-accent text-white shadow-xs">
          <Check aria-hidden className="size-4" strokeWidth={3} />
        </span>
      ) : null}
      <div className="flex min-w-0 flex-1 flex-col gap-2 @lg:gap-3">
        <div className="flex min-w-0 items-center gap-3 @lg:block">
          <div
            className="agent-choice-avatar active-subagent-chip flex size-12 shrink-0 items-center justify-center rounded-lg @lg:size-11"
            data-color={color}
          >
            <Icon aria-hidden className="size-5" strokeWidth={2.25} />
          </div>
          <div className="min-w-0 font-sans text-base font-semibold text-ink @lg:hidden">
            {title}
          </div>
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="hidden font-sans text-sm font-semibold text-ink @lg:block">
            {title}
          </div>
          <p className="line-clamp-3 text-pretty font-sans text-base leading-6 text-ink-faint sm:text-sm sm:leading-5">
            {entry.description.trim() || 'No description provided.'}
          </p>
        </div>
      </div>
    </button>
  )
}

/* ---------------- no provider configured ---------------- */

function NoProviderBody({
  onConfigureProvider,
}: {
  onConfigureProvider?: () => void
}) {
  return (
    <>
      <h1 className={HEADING_CLASS}>Configure a provider.</h1>
      <p className={BODY_CLASS}>
        the harness worker is running, but no model providers are set up yet.
        add an api key — or point at a local server — then pick a model and
        send.
      </p>
      <div>
        <Button variant="primary" onClick={onConfigureProvider}>
          configure a provider
        </Button>
      </div>
    </>
  )
}

/* ---------------- harness not installed ---------------- */

function NoHarnessBody({
  onInstallHarness,
}: {
  onInstallHarness?: () => void
}) {
  return (
    <>
      <h1 className={HEADING_CLASS}>Install the harness worker.</h1>
      <p className={BODY_CLASS}>
        chat runs on the <span className="text-ink">harness</span> worker, and
        it isn't connected yet. add it to start a session.
      </p>
      <div className="flex flex-col gap-4">
        <div>
          <Button variant="primary" onClick={onInstallHarness}>
            add harness
          </Button>
        </div>
        <div className="flex flex-col gap-1.5">
          <span className="font-mono text-[12px] text-ink-ghost lowercase">
            prefer the terminal? run:
          </span>
          <Terminal
            command={HARNESS_INSTALL_COMMAND}
            chips={<CopyCommandButton text={HARNESS_INSTALL_COMMAND} />}
          />
        </div>
      </div>
    </>
  )
}

/* ---------------- installing / install failed ---------------- */

function InstallingBody({
  failed,
  stages,
  errorMessage,
  onRetryInstall,
}: {
  failed: boolean
  stages: InstallStage[]
  errorMessage?: string | null
  onRetryInstall?: () => void
}) {
  return (
    <>
      <h1 className={HEADING_CLASS}>
        {failed ? 'install failed.' : 'installing harness…'}
      </h1>
      <p className={BODY_CLASS}>
        {failed
          ? "the worker manager couldn't finish adding harness. you can retry here, or run it from your terminal."
          : 'adding the harness worker. this mirrors what the cli does — you could run the same command in your terminal.'}
      </p>
      <InstallConsole stages={stages} running={!failed} />
      {failed ? (
        <div className="flex flex-col gap-3">
          {errorMessage && stages.length === 0 ? (
            <span className="font-mono text-[12.5px] text-alert lowercase break-all">
              {errorMessage}
            </span>
          ) : null}
          <div>
            <Button variant="primary" onClick={onRetryInstall}>
              retry
            </Button>
          </div>
        </div>
      ) : null}
    </>
  )
}

/**
 * Live install console — `Terminal` chrome over the streamed `worker`
 * lifecycle stages, framed as the equivalent `iii trigger compose::add worker=harness` run.
 */
function InstallConsole({
  stages,
  running,
}: {
  stages: InstallStage[]
  running: boolean
}) {
  return (
    <Terminal command={HARNESS_INSTALL_COMMAND}>
      <div className="bg-bg px-3 py-3 font-mono text-[12.5px] leading-[1.6] flex flex-col gap-0.5">
        {stages.length === 0 ? (
          <div className="text-ink-faint italic animate-pulse">
            · connecting to the worker manager…
          </div>
        ) : (
          stages.map((s, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: append-only log; entries never reorder or get removed
            <StageLine key={`${s.stage}-${s.at}-${i}`} stage={s} />
          ))
        )}
        {running && stages.length > 0 ? (
          <div className="text-ink-ghost animate-pulse">· working…</div>
        ) : null}
      </div>
    </Terminal>
  )
}

function StageLine({ stage }: { stage: InstallStage }) {
  const failed = stage.stage === 'failed'
  const done = stage.stage === 'done'
  const tone = failed ? 'text-alert' : done ? 'text-accent' : 'text-ink-faint'
  const symbol = failed ? '×' : done ? '✓' : '→'
  const showProgress = typeof stage.progress === 'number' && !done && !failed
  return (
    <div className={cn('flex items-baseline gap-2', tone)}>
      <span aria-hidden className="text-ink-ghost">
        {symbol}
      </span>
      <span className="break-all">{stageLabel(stage)}</span>
      {showProgress ? (
        <span className="ml-auto text-ink-ghost tabular-nums">
          {Math.round((stage.progress ?? 0) * 100)}%
        </span>
      ) : null}
    </div>
  )
}

function stageLabel(stage: InstallStage): string {
  switch (stage.stage) {
    case 'started':
      return 'resolving harness from the registry…'
    case 'downloading':
      return 'downloading harness…'
    case 'downloaded':
      return 'downloaded'
    case 'done':
      return 'done · harness ready'
    case 'failed':
      return stage.error
        ? `failed · ${normalizeErrorMessage(stage.error)}`
        : 'failed'
    default:
      return stage.stage
  }
}
