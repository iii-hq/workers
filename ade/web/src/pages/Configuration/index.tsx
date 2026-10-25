import {
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
  Send,
  Settings2,
  ShieldCheck,
  Sparkles,
  SquareTerminal,
  Webhook,
  Workflow,
} from 'lucide-react'
import { MotionConfig } from 'motion/react'
import { useMemo } from 'react'
import { Skeleton } from '@/components/ui/Skeleton'
import { useContainerNarrow } from '@/hooks/use-container-narrow'
import { useWorkersConfigurationRoute } from '@/hooks/use-hash-route'
import type { ThemePreference } from '@/hooks/use-theme'
import { configurationFormFamily } from '@/lib/configuration-family'
import { NEUTRAL_SURFACE } from '@/lib/neutral-surface'
import { useExtConfigForms, useUiAssetsStatus } from '@/lib/ui-slots'
import { cn } from '@/lib/utils'
import { SettingsPageTransition } from './components/motion'
import { SettingsPageHeader } from './components/primitives'
import {
  type SettingsNavEntry,
  SettingsNavigation,
} from './components/SettingsNavigation'
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
  console: Settings2,
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
  ide: SquareTerminal,
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
  'security-scan': ScanSearch,
  'session-manager': MessagesSquare,
  shell: SquareTerminal,
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
  console: 'ADE',
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
  ide: 'IDE',
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
  'security-scan': 'Security Scan',
  'session-manager': 'Session Manager',
  shell: 'IDE',
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
  theme: ThemePreference
  onThemeChange: (next: ThemePreference) => void
  onDirtyChange: (dirty: boolean) => void
  tryNavigate: (action: () => void) => boolean
  /** Close the hosting dialog; drawn in the content header (and the sidebar
   * when it is the whole dialog). */
  onClose?: () => void
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
  onClose,
}: ConfigurationProps) {
  const [route, navigateWorker] = useWorkersConfigurationRoute()
  const configurationsQuery = useConfigurationsList()
  const configForms = useExtConfigForms()
  const uiAssetsStatus = useUiAssetsStatus()
  useWorkerRegistryReactivity()

  const configFormIds = useMemo(
    () => new Set(configForms.map((form) => form.configurationId)),
    [configForms],
  )
  const entries = useMemo(
    () =>
      (configurationsQuery.data ?? [])
        .filter(isOperatorConfiguration)
        .filter(
          (entry) =>
            configFormIds.has(entry.id) ||
            configFormIds.has(configurationFormFamily(entry)),
        )
        .sort((left, right) =>
          configurationName(left).localeCompare(configurationName(right)),
        ),
    [configurationsQuery.data, configFormIds],
  )
  const settingsLoading =
    configurationsQuery.isLoading || uiAssetsStatus === 'loading'
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

  const navEntries = useMemo<SettingsNavEntry[]>(
    () =>
      entries.map((entry) => ({
        id: entry.id,
        label: configurationName(entry),
        description: entry.description || entry.id,
        icon: iconForConfiguration(entry),
      })),
    [entries],
  )

  const paneKey = !route.open
    ? 'general'
    : settingsLoading
      ? 'loading'
      : configurationsQuery.isError
        ? 'error'
        : (route.configurationId ?? 'none')

  return (
    <MotionConfig reducedMotion="user">
      <div
        ref={rootRef}
        className={cn(
          NEUTRAL_SURFACE,
          'configuration-surface flex min-h-0 min-w-0 flex-1 bg-white font-inter text-sm text-foreground antialiased dark:bg-black',
          // A neutral focus stroke: the console's light accent ring is burnt
          // orange, which the neutral settings page has no other use for.
          '[--ring:var(--color-neutral-500)] [--color-rule-focus:var(--color-neutral-500)] dark:[--ring:var(--color-neutral-400)] dark:[--color-rule-focus:var(--color-neutral-400)]',
        )}
      >
        {showNavigation ? (
          <SettingsNavigation
            entries={navEntries}
            generalSelected={!route.open}
            selectedId={route.configurationId}
            onSelectGeneral={selectGeneral}
            onSelectWorker={selectWorker}
            isLoading={settingsLoading}
            isError={configurationsQuery.isError}
            errorMessage={(configurationsQuery.error as Error | null)?.message}
            narrow={narrow}
            onClose={onClose}
          />
        ) : null}

        {showContent ? (
          <main
            // The dialog parks its initial focus here, so opening settings
            // paints no focus ring; Tab moves on to the first control.
            data-settings-focus-root
            tabIndex={-1}
            className="@container flex min-h-0 min-w-0 flex-1 flex-col outline-none"
            aria-label={
              route.configurationId
                ? `${route.configurationId} settings`
                : 'General settings'
            }
          >
            <SettingsPageTransition pageKey={paneKey}>
              {!route.open ? (
                <div className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain">
                  <SettingsPageHeader
                    title="General"
                    description="ADE appearance, permissions, providers, and filesystem access."
                    onBack={narrow ? showMobileNavigation : undefined}
                  />
                  <ConsoleSettingsTab
                    theme={theme}
                    onThemeChange={onThemeChange}
                  />
                </div>
              ) : settingsLoading ? (
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
                  description={`No configurable worker named “${route.configurationId}” is available. Start or enable its worker and ensure it provides a settings UI, then try again.`}
                />
              ) : (
                <EditorEmptyState
                  title="Select a worker"
                  description="Choose a worker from the settings sidebar to view its configuration."
                />
              )}
            </SettingsPageTransition>
          </main>
        ) : null}
      </div>
    </MotionConfig>
  )
}

function ConfigurationLoading({ onBack }: { onBack?: () => void }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain">
      <SettingsPageHeader
        title="Worker settings"
        description="Loading the registered configuration."
        onBack={onBack}
      />
      <div className="flex w-full max-w-[760px] flex-col gap-4 px-5 pt-6 @3xl:px-14">
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-9 w-full" />
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
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain">
      <SettingsPageHeader
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
