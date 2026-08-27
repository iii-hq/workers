import { Bot, Check, X } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import { CopyCommandButton } from '@/components/chat/sandbox/terminal/CopyCommandButton'
import { Terminal } from '@/components/chat/sandbox/terminal/Terminal'
import { Button } from '@/components/ui/Button'
import { Wordmark } from '@/components/ui/Wordmark'
import type { InstallStage } from '@/hooks/use-harness-status'
import { type AgentEntry, listAgents } from '@/lib/backend/directory-prompts'
import { getIiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'
import { cn } from '@/lib/utils'
import type {
  AgentProfileSnapshot,
  SubagentColor,
  SubagentIcon,
} from '@/types/chat'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import { DirectoryPicker, type WorktreePickerOptions } from './DirectoryPicker'
import { SessionAddonsPicker } from './SessionAddonsPicker'
import { SystemPromptPicker } from './SystemPromptPicker'
import {
  agentIdFromSystemPrompt,
  type SkillSelection,
  type SystemPromptState,
  toggleSkillSelection,
  withAgentChoice,
  withoutAgentChoice,
} from './system-prompt-selection'
import './EmptyState.css'

/**
 * The chat empty state, as a small set of presentational variants:
 *
 *   - `ready`          — harness present + a model available (the hero copy)
 *   - `no-provider`    — harness present, no model configured -> configure CTA
 *   - `no-harness`     — harness worker missing -> install CTA + cli hint
 *   - `installing`     — `worker::add` in flight -> live console
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
  /** Exact model-invocable skill IDs curated for this session. */
  skills?: SkillSelection
  onSkillsChange?: (next: SkillSelection) => void
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
  skills,
  onSkillsChange,
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
            skills={skills}
            onSkillsChange={onSkillsChange}
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
  skills,
  onSkillsChange,
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
  skills?: SkillSelection
  onSkillsChange?: (next: SkillSelection) => void
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
            skills={skills}
            onSkillsChange={onSkillsChange}
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
  skills,
  onSkillsChange,
  agentEntries,
  agentProfile,
  onAgentProfileChange,
}: {
  systemPrompt: SystemPromptState
  onSystemPromptChange: (next: SystemPromptState) => void
  skills?: SkillSelection
  onSkillsChange?: (next: SkillSelection) => void
  agentEntries?: AgentEntry[] | null
  agentProfile?: AgentProfileSnapshot
  onAgentProfileChange?: (next: AgentProfileSnapshot | undefined) => void
}) {
  const selectedAgentId =
    agentProfile?.id ?? agentIdFromSystemPrompt(systemPrompt)
  const [manualOpen, setManualOpen] = useState(
    () =>
      selectedAgentId === null &&
      (systemPrompt.choice !== 'default' || Boolean(skills?.length)),
  )
  const panelId = useId()
  const catalog = useAgentCatalog(agentEntries)
  const agents = catalog.entries ?? []

  const toggleManual = () => {
    const next = !manualOpen
    setManualOpen(next)
    if (next && selectedAgentId !== null) {
      if (agentIdFromSystemPrompt(systemPrompt) !== null) {
        onSystemPromptChange(withoutAgentChoice(systemPrompt))
      }
      onAgentProfileChange?.(undefined)
    }
  }

  return (
    <section
      aria-label="session setup"
      className="t-acc w-full max-w-[40rem]"
      data-open={manualOpen}
    >
      <div className="empty-state-agent-panel">
        <div
          className="empty-state-agent-panel-inner"
          aria-hidden={manualOpen}
          inert={manualOpen}
        >
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
              else {
                onSystemPromptChange(withAgentChoice(systemPrompt, entry.id))
              }
            }}
          />
        </div>
      </div>

      <button
        type="button"
        className="t-acc-head empty-state-manual-trigger"
        aria-expanded={manualOpen}
        aria-controls={panelId}
        onClick={toggleManual}
      >
        <span>Configure manually</span>
        <span className="t-acc-chevron" aria-hidden>
          <svg
            className="size-4 fill-none stroke-current"
            viewBox="0 0 16 16"
            aria-hidden
          >
            <title>Toggle manual configuration</title>
            <path
              d="M4 6.5L8 10.5L12 6.5"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </span>
      </button>

      <div id={panelId} className="t-acc-panel">
        <div
          className="t-acc-panel-inner"
          aria-hidden={!manualOpen}
          inert={!manualOpen}
        >
          <ManualSessionSetupControls
            systemPrompt={systemPrompt}
            onSystemPromptChange={onSystemPromptChange}
            skills={skills}
            onSkillsChange={onSkillsChange}
          />
        </div>
      </div>
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
      <div className="mb-2 font-sans text-base font-medium text-ink-faint sm:text-sm">
        Choose an agent profile
      </div>
      {loading ? (
        <div
          className="grid grid-cols-1 gap-3 sm:grid-cols-2 md:grid-cols-3 p-1"
          aria-hidden
        >
          <div className="h-40 rounded-lg bg-panel-raised shadow-raised" />
          <div className="h-40 rounded-lg bg-panel-raised shadow-raised" />
          <div className="h-40 rounded-lg bg-panel-raised shadow-raised" />
        </div>
      ) : error ? (
        <p className="rounded-md bg-panel-raised px-3 py-4 font-sans text-base text-ink-faint shadow-raised sm:text-sm">
          Agent profiles are unavailable. Configure the session manually
          instead.
        </p>
      ) : entries.length === 0 ? (
        <p className="rounded-md bg-panel-raised px-3 py-4 font-sans text-base text-ink-faint shadow-raised sm:text-sm">
          No agent profiles are available yet.
        </p>
      ) : (
        <ul
          // biome-ignore lint/a11y/noRedundantRoles: keep list semantics when CSS resets remove markers.
          role="list"
          className={cn(
            'grid grid-cols-1 gap-3 sm:grid-cols-2 p-1',
            entries.length >= 3 && 'md:grid-cols-3',
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
        </ul>
      )}
    </div>
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
        'group/agent relative h-full min-h-40 w-full cursor-pointer rounded-lg bg-panel-raised p-4 text-left shadow-raised ring-1 ring-rule-2 transition-[transform,box-shadow] duration-150 hover:-translate-y-px hover:shadow-floating focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
        selected && 'ring-2 ring-accent',
      )}
    >
      {selected ? (
        <span className="absolute right-3 top-3 flex size-5 items-center justify-center rounded-full bg-accent text-white shadow-xs">
          <Check aria-hidden className="size-4" strokeWidth={3} />
        </span>
      ) : null}
      <div className="flex min-w-0 flex-col gap-3">
        <div
          className="agent-choice-avatar active-subagent-chip flex size-12 shrink-0 items-center justify-center rounded-lg sm:size-11"
          data-color={color}
        >
          <Icon aria-hidden className="size-5" strokeWidth={2.25} />
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="font-sans text-base font-semibold text-ink sm:text-sm">
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

function ManualSessionSetupControls({
  systemPrompt,
  onSystemPromptChange,
  skills,
  onSkillsChange,
}: {
  systemPrompt: SystemPromptState
  onSystemPromptChange: (next: SystemPromptState) => void
  skills?: SkillSelection
  onSkillsChange?: (next: SkillSelection) => void
}) {
  return (
    <div className="flex max-w-full flex-col items-center gap-2 pt-3">
      <div className="flex max-w-full min-w-0 items-baseline justify-center gap-1.5 font-sans text-base text-ink-faint sm:text-sm">
        <span>System prompt</span>
        <SystemPromptPicker
          value={systemPrompt}
          onChange={onSystemPromptChange}
          allowCustom={false}
          appearance="inline"
          className="max-w-48"
        />
      </div>

      {onSkillsChange ? (
        <div className="flex justify-center font-sans text-base text-ink-faint sm:text-sm">
          <SessionAddonsPicker
            value={skills}
            onChange={onSkillsChange}
            appearance="inline"
          />
        </div>
      ) : null}

      {skills?.length && onSkillsChange ? (
        <ul
          // biome-ignore lint/a11y/noRedundantRoles: keep list semantics when CSS resets remove markers.
          role="list"
          aria-label="skills selected for this session"
          className="flex max-w-full flex-wrap justify-center gap-1.5"
        >
          {skills.map((skill) => (
            <li key={skill} className="min-w-0 max-w-full">
              <button
                type="button"
                aria-label={`remove ${skill} from this session`}
                title={`Remove ${skill}`}
                onClick={() =>
                  onSkillsChange(toggleSkillSelection(skills, skill))
                }
                className="relative inline-flex h-9 max-w-full items-center gap-1 rounded-full bg-surface py-1 pr-2 pl-3 font-sans text-base font-medium text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-rule-focus sm:h-7 sm:text-[0.8125rem]"
              >
                <span className="truncate">{skill}</span>
                <X className="size-4 shrink-0 text-ink-faint/80" aria-hidden />
                <span
                  className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                  aria-hidden="true"
                />
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
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
