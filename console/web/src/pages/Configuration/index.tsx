import {
  AppWindow,
  ArrowLeft,
  BookOpen,
  Bot,
  Box,
  Braces,
  Brain,
  BrainCircuit,
  Cable,
  Clock3,
  Code2,
  Database,
  FilePenLine,
  FileText,
  FolderTree,
  GitBranch,
  Globe2,
  HardDrive,
  KeyRound,
  LayoutTemplate,
  ListTodo,
  type LucideIcon,
  Mail,
  MessagesSquare,
  Monitor,
  Network,
  Palette,
  Radio,
  Route,
  ScanSearch,
  Search,
  Send,
  Settings2,
  ShieldCheck,
  Sparkles,
  Terminal,
  Webhook,
  Workflow,
  X,
} from 'lucide-react'
import { useMemo, useRef, useState } from 'react'
import { Input } from '@/components/ui/Input'
import { Skeleton } from '@/components/ui/Skeleton'
import { useContainerNarrow } from '@/hooks/use-container-narrow'
import {
  useWorkersConfigurationRoute,
  type WorkersConfigurationRoute,
} from '@/hooks/use-hash-route'
import type { Theme } from '@/hooks/use-theme'
import { configurationFormFamily } from '@/lib/configuration-family'
import { cn } from '@/lib/utils'
import { isOperatorConfiguration } from './lib/configuration-catalog'
import { ConsoleSettingsTab } from './tabs/ConsoleSettingsTab'
import type { ConfigurationSchemaView } from './tabs/WorkersTab/api'
import { EditorEmptyState } from './tabs/WorkersTab/EmptyState'
import {
  useConfigurationsList,
  useWorkerRegistryReactivity,
} from './tabs/WorkersTab/hooks'
import { WorkerEditor } from './tabs/WorkersTab/WorkerEditor'

const NARROW_BELOW = 720

const CONFIGURATION_ICONS: Record<string, LucideIcon> = {
  a2ui: LayoutTemplate,
  'approval-gate': ShieldCheck,
  bridge: Cable,
  browser: Globe2,
  canvas: Palette,
  'claude-code': Bot,
  'code-runner': Code2,
  codex: Bot,
  computer: Monitor,
  console: AppWindow,
  'context-manager': BrainCircuit,
  cron: Clock3,
  cursor: Bot,
  database: Database,
  devin: Bot,
  document: FileText,
  editor: FilePenLine,
  email: Mail,
  fp: Workflow,
  github: GitBranch,
  grok: Bot,
  harness: Network,
  http: Webhook,
  'iii-directory': FolderTree,
  'llm-router': Route,
  memory: Brain,
  'memory-consolidate': BrainCircuit,
  opencode: Bot,
  openwiki: BookOpen,
  pdf: FileText,
  pi: Bot,
  'provider-xai': Sparkles,
  pubsub: Radio,
  queue: ListTodo,
  'rbac-proxy': KeyRound,
  'sandbox-code-runner': Code2,
  scrapling: Globe2,
  'security-scan': ScanSearch,
  'session-manager': MessagesSquare,
  shell: Terminal,
  slack: MessagesSquare,
  state: Braces,
  storage: HardDrive,
  tailscale: Network,
  'telegram-bot': Send,
  vscode: Code2,
  web: Globe2,
  workflow: Workflow,
  worktree: GitBranch,
}

/** Stable operator-facing names. Registration metadata remains useful for the
 * description, but several workers historically reused generic names such as
 * "Console"; the settings catalog should never present indistinguishable
 * navigation rows. */
const CONFIGURATION_NAMES: Record<string, string> = {
  a2ui: 'A2UI',
  'approval-gate': 'Approval Gate',
  bridge: 'Bridge',
  browser: 'Browser',
  canvas: 'Canvas',
  'claude-code': 'Claude Code',
  'code-runner': 'Code Runner',
  codex: 'Codex',
  computer: 'Computer',
  console: 'Console',
  'context-manager': 'Context Manager',
  cron: 'Cron',
  cursor: 'Cursor',
  database: 'Database',
  devin: 'Devin',
  document: 'Document',
  editor: 'Editor',
  email: 'Email',
  fp: 'FP',
  github: 'GitHub',
  grok: 'Grok',
  harness: 'Harness',
  http: 'HTTP',
  'iii-directory': 'III Directory',
  'llm-router': 'LLM Router',
  memory: 'Memory',
  'memory-consolidate': 'Memory Consolidation',
  opencode: 'OpenCode',
  openwiki: 'OpenWiki',
  pdf: 'PDF',
  pi: 'Pi',
  'provider-xai': 'xAI Provider',
  pubsub: 'Pub/Sub',
  queue: 'Queue',
  'rbac-proxy': 'RBAC Proxy',
  'sandbox-code-runner': 'Sandbox Code Runner',
  scrapling: 'Scrapling',
  'security-scan': 'Security Scan',
  'session-manager': 'Session Manager',
  shell: 'Shell',
  slack: 'Slack',
  state: 'State',
  storage: 'Storage',
  tailscale: 'Tailscale',
  'telegram-bot': 'Telegram Bot',
  vscode: 'VS Code',
  web: 'Web',
  workflow: 'Workflow',
  worktree: 'Worktree',
}

interface ConfigurationProps {
  theme: Theme
  onThemeChange: (next: Theme) => void
  onDirtyChange: (dirty: boolean) => void
  tryNavigate: (action: () => void) => boolean
}

/**
 * One settings workspace for console preferences and every registered worker.
 * Selection is URL-owned so global links and worker pane actions can land on a
 * precise form. The sidebar collapses to a drill-in menu when the dialog is
 * narrow; the route remains the source of truth in both layouts.
 */
export function Configuration({
  theme,
  onThemeChange,
  onDirtyChange,
  tryNavigate,
}: ConfigurationProps) {
  const [route, navigateWorker] = useWorkersConfigurationRoute()
  const configurationsQuery = useConfigurationsList()
  useWorkerRegistryReactivity()

  const entries = useMemo(
    () =>
      (configurationsQuery.data ?? [])
        .filter(isOperatorConfiguration)
        .sort((left, right) =>
          configurationName(left).localeCompare(configurationName(right)),
        ),
    [configurationsQuery.data],
  )
  const selectedEntry = route.configurationId
    ? (entries.find((entry) => entry.id === route.configurationId) ?? null)
    : null
  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)

  const workerMenuOpen = route.open && route.configurationId === null
  const showNavigation = !narrow || workerMenuOpen
  const showContent = !narrow || !workerMenuOpen

  const selectGeneral = () => {
    if (!route.open) return
    tryNavigate(() => {
      window.location.hash = '#/configuration'
    })
  }
  const selectWorker = (configurationId: string) => {
    if (configurationId === route.configurationId) return
    tryNavigate(() => navigateWorker(configurationId))
  }
  const showMobileNavigation = () => {
    tryNavigate(() => navigateWorker(null))
  }

  return (
    <div
      ref={rootRef}
      className="configuration-surface flex min-h-0 min-w-0 flex-1 gap-px bg-edge font-sans"
    >
      {showNavigation ? (
        <SettingsNavigation
          entries={entries}
          route={route}
          onSelectGeneral={selectGeneral}
          onSelectWorker={selectWorker}
          isLoading={configurationsQuery.isLoading}
          isError={configurationsQuery.isError}
          errorMessage={(configurationsQuery.error as Error | null)?.message}
          narrow={narrow}
        />
      ) : null}

      {showContent ? (
        <main
          className="flex min-h-0 min-w-0 flex-1 flex-col bg-panel"
          aria-label={
            route.configurationId
              ? `${route.configurationId} settings`
              : 'General settings'
          }
        >
          {!route.open ? (
            <>
              <SettingsContentHeader
                icon={Settings2}
                title="General"
                description="Console appearance, permissions, providers, and filesystem access."
                onBack={narrow ? showMobileNavigation : undefined}
              />
              <ConsoleSettingsTab theme={theme} onThemeChange={onThemeChange} />
            </>
          ) : configurationsQuery.isLoading ? (
            <ConfigurationLoading
              onBack={narrow ? showMobileNavigation : undefined}
            />
          ) : configurationsQuery.isError ? (
            <ConfigurationUnavailable
              onBack={narrow ? showMobileNavigation : undefined}
              headerTitle="Worker settings"
              title="Could not load worker settings"
              description={
                (configurationsQuery.error as Error | null)?.message ??
                'The configuration service did not return the registered workers.'
              }
            />
          ) : selectedEntry ? (
            <WorkerEditor
              key={selectedEntry.id}
              entry={selectedEntry}
              onDirtyChange={onDirtyChange}
              onBack={narrow ? showMobileNavigation : undefined}
            />
          ) : route.configurationId ? (
            <ConfigurationUnavailable
              onBack={narrow ? showMobileNavigation : undefined}
              headerTitle={configurationName({
                id: route.configurationId,
                name: route.configurationId,
              })}
              title="Worker settings unavailable"
              description={`No registered configuration named “${route.configurationId}” is available. Start or enable its worker, then try again.`}
            />
          ) : (
            <EditorEmptyState
              title="Select a worker"
              description="Choose a worker from the settings sidebar to view its configuration."
            />
          )}
        </main>
      ) : null}
    </div>
  )
}

interface SettingsNavigationProps {
  entries: ConfigurationSchemaView[]
  route: WorkersConfigurationRoute
  onSelectGeneral: () => void
  onSelectWorker: (configurationId: string) => void
  isLoading: boolean
  isError: boolean
  errorMessage?: string
  narrow: boolean
}

function SettingsNavigation({
  entries,
  route,
  onSelectGeneral,
  onSelectWorker,
  isLoading,
  isError,
  errorMessage,
  narrow,
}: SettingsNavigationProps) {
  const [query, setQuery] = useState('')
  const searchRef = useRef<HTMLInputElement | null>(null)
  const normalizedQuery = query.trim().toLowerCase()
  const filteredEntries = useMemo(() => {
    if (!normalizedQuery) return entries
    return entries.filter((entry) =>
      `${entry.name} ${entry.id} ${entry.description}`
        .toLowerCase()
        .includes(normalizedQuery),
    )
  }, [entries, normalizedQuery])

  return (
    <aside
      className={cn(
        'flex min-h-0 flex-col bg-sidebar',
        narrow ? 'min-w-0 flex-1' : 'w-80 shrink-0',
      )}
      aria-label="Settings navigation"
    >
      <div className="shrink-0 px-4 pb-3 pt-5 pr-12 sm:px-3 sm:pt-4 sm:pr-12">
        <h1 className="text-balance font-sans text-xl font-semibold text-ink sm:text-lg">
          Settings
        </h1>
        <div className="relative mt-4 sm:mt-3">
          <Search
            className="pointer-events-none absolute left-2.5 top-1/2 size-4 shrink-0 -translate-y-1/2 text-ink-ghost"
            aria-hidden
          />
          <Input
            ref={searchRef}
            name="settings-search"
            value={query}
            onChange={setQuery}
            placeholder="Search settings"
            aria-label="Search settings"
            className="h-11 pl-8 pr-9 text-base sm:h-9 sm:text-[0.8125rem]"
            onKeyDown={(event) => {
              if (event.key === 'Escape' && query) setQuery('')
            }}
          />
          {query ? (
            <button
              type="button"
              aria-label="Clear settings search"
              onClick={() => {
                setQuery('')
                searchRef.current?.focus()
              }}
              className="absolute right-1 top-1/2 flex size-8 shrink-0 -translate-y-1/2 items-center justify-center rounded-sm text-ink-ghost hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
            >
              <span
                className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                aria-hidden="true"
              />
              <X className="size-4 shrink-0" aria-hidden />
            </button>
          ) : null}
        </div>
      </div>

      <nav
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-4"
        aria-label="Settings sections"
      >
        {/* biome-ignore lint/a11y/noRedundantRoles: list-none removes native list semantics in Safari/VoiceOver. */}
        <ul role="list" className="flex list-none flex-col gap-1">
          <li>
            <SettingsNavItem
              icon={Settings2}
              label="General"
              description="Console preferences"
              selected={!route.open}
              onClick={onSelectGeneral}
            />
          </li>
        </ul>

        <div className="flex items-center gap-2 px-2 pb-1 pt-5">
          <h2 className="font-sans text-sm font-medium text-ink-faint">
            Workers
          </h2>
          {!isLoading && !isError ? (
            <p
              className="font-mono text-sm tabular-nums text-ink-ghost"
              aria-live="polite"
            >
              {filteredEntries.length}
            </p>
          ) : null}
        </div>

        {isLoading ? <NavigationLoading /> : null}
        {isError ? (
          <p className="px-2 py-4 font-sans text-base text-alert sm:text-sm">
            {errorMessage ?? 'Could not load worker settings.'}
          </p>
        ) : null}
        {!isLoading && !isError && filteredEntries.length === 0 ? (
          <p className="px-2 py-4 font-sans text-base text-ink-faint sm:text-sm">
            {entries.length === 0
              ? 'No worker configurations are registered.'
              : `No settings match “${query.trim()}”.`}
          </p>
        ) : null}
        {/* biome-ignore lint/a11y/noRedundantRoles: list-none removes native list semantics in Safari/VoiceOver. */}
        <ul role="list" className="flex list-none flex-col gap-1">
          {filteredEntries.map((entry) => (
            <li key={entry.id}>
              <SettingsNavItem
                icon={iconForConfiguration(entry)}
                label={configurationName(entry)}
                description={entry.description || entry.id}
                selected={route.configurationId === entry.id}
                onClick={() => onSelectWorker(entry.id)}
              />
            </li>
          ))}
        </ul>
      </nav>
    </aside>
  )
}

interface SettingsNavItemProps {
  icon: LucideIcon
  label: string
  description: string
  selected: boolean
  onClick: () => void
}

function SettingsNavItem({
  icon: Icon,
  label,
  description,
  selected,
  onClick,
}: SettingsNavItemProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={selected ? 'page' : undefined}
      className={cn(
        'flex min-h-12 w-full items-start gap-2.5 rounded-md px-2.5 py-2 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-rule-focus sm:min-h-0',
        selected
          ? 'bg-surface-selected text-ink'
          : 'text-ink-faint hover:bg-surface-hover hover:text-ink',
      )}
    >
      <Icon className="size-4 shrink-0" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="truncate font-sans text-base font-medium sm:text-sm leading-4">
          {label}
        </p>
        <p className="line-clamp-1 font-sans text-sm text-ink-ghost sm:text-[0.75rem]">
          {description}
        </p>
      </div>
    </button>
  )
}

function SettingsContentHeader({
  icon: Icon,
  title,
  description,
  onBack,
}: {
  icon: LucideIcon
  title: string
  description: string
  onBack?: () => void
}) {
  return (
    <header className="flex min-h-16 shrink-0 items-start gap-3 border-b border-edge bg-panel-raised px-4 py-3 pr-12 sm:min-h-14 sm:px-5">
      {onBack ? (
        <button
          type="button"
          onClick={onBack}
          aria-label="Open settings navigation"
          className="relative flex size-9 shrink-0 items-center justify-center rounded-md text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
        >
          <span
            className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <ArrowLeft className="size-4 shrink-0" aria-hidden />
        </button>
      ) : (
        <Icon className="size-4 shrink-0 text-ink-faint" aria-hidden />
      )}
      <div className="min-w-0 flex-1">
        <h2 className="text-balance font-sans text-lg font-semibold text-ink">
          {title}
        </h2>
        <p className="line-clamp-2 text-pretty font-sans text-sm text-ink-faint">
          {description}
        </p>
      </div>
    </header>
  )
}

function NavigationLoading() {
  return (
    // biome-ignore lint/a11y/noRedundantRoles: list-none removes native list semantics in Safari/VoiceOver.
    <ul role="list" className="flex list-none flex-col gap-2 px-2 py-1">
      {[0, 1, 2].map((item) => (
        <li key={item} className="flex items-start gap-2.5 py-2">
          <Skeleton className="size-4 shrink-0" />
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <Skeleton className="h-4 w-28" />
            <Skeleton className="h-3 w-full" />
          </div>
        </li>
      ))}
    </ul>
  )
}

function ConfigurationLoading({ onBack }: { onBack?: () => void }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-panel">
      <SettingsContentHeader
        icon={Box}
        title="Worker settings"
        description="Loading the registered configuration."
        onBack={onBack}
      />
      <div className="flex flex-col gap-4 p-6">
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-10 w-full" />
      </div>
    </div>
  )
}

function ConfigurationUnavailable({
  headerTitle,
  title,
  description,
  onBack,
}: {
  headerTitle: string
  title: string
  description: string
  onBack?: () => void
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-panel">
      <SettingsContentHeader
        icon={Box}
        title={headerTitle}
        description="Configuration is temporarily unavailable."
        onBack={onBack}
      />
      <EditorEmptyState title={title} description={description} />
    </div>
  )
}

function iconForConfiguration(
  entry: Pick<ConfigurationSchemaView, 'id' | 'metadata'>,
): LucideIcon {
  return CONFIGURATION_ICONS[configurationFormFamily(entry)] ?? Box
}

function configurationName(
  entry: Pick<ConfigurationSchemaView, 'id' | 'name' | 'metadata'>,
): string {
  const family = configurationFormFamily(entry)
  const stableName = CONFIGURATION_NAMES[family]
  if (!stableName) return entry.name || entry.id
  return entry.id === family ? stableName : `${stableName} · ${entry.id}`
}
